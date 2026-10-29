import base64
import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('reauthorize', Path(__file__).parents[1] / 'workbuddy-reauthorize.py')
repair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repair)


def auth(uid='expected', expiry=None):
    claims = dict(sub=uid, exp=expiry if expiry is not None else int(time.time()) + 3600)
    token = 'header.' + base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip('=') + '.signature'
    return dict(accessToken=token, refreshToken='private-refresh-material')


class ReauthorizationGuards(unittest.TestCase):
    def test_login_url_only_allows_official_https(self):
        self.assertTrue(repair.valid_login_url('https://www.workbuddy.cn/login?state=test'))
        for url in ['http://www.workbuddy.cn/login', 'https://www.workbuddy.cn.evil.test/login', 'https://user:pass@www.workbuddy.cn/login', 'https://www.workbuddy.cn:8080/login']:
            self.assertFalse(repair.valid_login_url(url))

    def test_wrong_account_is_never_accepted(self):
        with self.assertRaisesRegex(ValueError, '登录账号'):
            repair.candidate_identity(auth('other'), {'uid': 'other'}, 'expected')
        with self.assertRaisesRegex(ValueError, '身份或有效期'):
            repair.candidate_identity(auth('other'), {'uid': 'expected'}, 'expected')

    def test_expired_candidate_is_never_accepted(self):
        with self.assertRaisesRegex(ValueError, '身份或有效期'):
            repair.candidate_identity(auth(expiry=1), {'uid': 'expected'}, 'expected')

    def test_encrypted_staging_leaves_formal_record_unchanged(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(repair, 'PROFILE', Path(folder)):
            root = Path(folder)
            (root / 'secure-account-storage.key').write_text(base64.b64encode(bytes(range(32))).decode())
            account_id = 'wb-000000000001'
            record = dict(snapshot=dict(account={'uid': 'expected'}, auth=dict(accessToken='old-access', refreshToken='old-refresh')))
            account_path = root / 'workbuddy_accounts' / (account_id + '.json')
            repair.write_encrypted(account_path, record)
            before = account_path.read_bytes()
            flow_id = 'a' * 32
            flow_path = root / 'workbuddy-reauthorization' / (flow_id + '.json')
            flow = dict(account_id=account_id, expected_uid='expected', previous_revision=repair.revision(record['snapshot']), candidate={'data': auth()}, received_at_ms=1234, expires_at=0)
            repair.write_encrypted(flow_path, flow)
            with patch.object(repair, 'request', return_value=(200, {'code': 0, 'data': {'uid': 'expected'}})):
                self.assertEqual(repair.poll(flow_id)['status'], 'staged')
            pending = root / 'workbuddy-auth-pending' / (account_id + '.json')
            self.assertNotIn(b'private-refresh-material', pending.read_bytes())
            self.assertEqual(repair.read_encrypted(pending)['payload']['data']['refreshToken'], 'private-refresh-material')
            self.assertEqual(account_path.read_bytes(), before)
            self.assertEqual(repair.poll(flow_id)['status'], 'staged')

    def test_invalid_identifier_rejected_before_file_access(self):
        with self.assertRaises(ValueError):
            repair.start('../account')
        with self.assertRaises(ValueError):
            repair.poll('../flow')


if __name__ == '__main__':
    unittest.main()
