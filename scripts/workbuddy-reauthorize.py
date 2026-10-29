"""Official, account-bound reauthorization diagnostic. No client restart or raw token output.

start <account-id> creates a five-minute official login request.
poll <flow-id> polls once, verifies account identity and stages an encrypted candidate.
The existing native recovery transaction must perform the final account commit.
"""
from pathlib import Path
import argparse
import base64
import hashlib
import json
import os
import re
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

PROFILE = Path.home() / '.qiehuan_yingyong'
HOST = 'www.workbuddy.cn'


def key():
    return base64.b64decode((PROFILE / 'secure-account-storage.key').read_text().strip())


def read_encrypted(path):
    envelope = json.loads(path.read_text(encoding='utf-8'))
    return json.loads(AESGCM(key()).decrypt(base64.b64decode(envelope['nonce']),
        base64.b64decode(envelope['ciphertext']), None))


def write_encrypted(path, value):
    nonce = os.urandom(12)
    ciphertext = AESGCM(key()).encrypt(nonce, json.dumps(value, separators=(',', ':')).encode(), None)
    envelope = dict(version=1, kind='workbuddy-auth-recovery', algorithm='AES-256-GCM',
        key_id='local-secure-account-storage-v1', nonce=base64.b64encode(nonce).decode(),
        ciphertext=base64.b64encode(ciphertext).decode(), encrypted_at=int(time.time()))
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name('.' + path.name + '.' + uuid.uuid4().hex + '.tmp')
    temporary.write_text(json.dumps(envelope), encoding='utf-8')
    os.replace(temporary, path)


def revision(snapshot):
    value = [snapshot['account']['uid'], snapshot['auth']['accessToken'], snapshot['auth']['refreshToken']]
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()[:24]


def valid_login_url(value):
    parsed = urllib.parse.urlsplit(value)
    return parsed.scheme == 'https' and parsed.hostname in {'www.workbuddy.cn', 'www.codebuddy.cn'} and not parsed.username and not parsed.password and parsed.port in {None, 443}


def candidate_identity(auth, account, expected_uid):
    if account.get('uid') != expected_uid:
        raise ValueError('登录账号与待恢复账号不同，未覆盖任何账号；请重新发起并选择正确账号')
    token = auth.get('accessToken')
    if not isinstance(token, str) or not isinstance(auth.get('refreshToken'), str):
        raise ValueError('授权结果缺少完整凭证，未写回账号')
    try:
        claims = json.loads(base64.urlsafe_b64decode(token.split('.')[1] + '==='))
    except (ValueError, IndexError):
        raise ValueError('授权返回的凭证结构无效')
    if claims.get('sub') != expected_uid or claims.get('exp', 0) <= time.time():
        raise ValueError('授权凭证身份或有效期不匹配，未写回账号')


def request(path, headers=None, post=False):
    request_headers = {'Accept': 'application/json', 'Content-Type': 'application/json'}
    request_headers.update(headers or {})
    req = urllib.request.Request('https://' + HOST + path,
        data=b'{}' if post else None, headers=request_headers)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        response = opener.open(req, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        try:
            payload = json.loads(response.read(1024 * 1024))
        except (ValueError, UnicodeDecodeError):
            raise ValueError(f'官方接口返回 HTTP {response.status}，响应格式无效')
        return response.status, payload


def start(account_id):
    if not re.fullmatch(r'wb-[0-9a-f]{12}', account_id):
        raise ValueError('账号 ID 无效')
    record = read_encrypted(PROFILE / 'workbuddy_accounts' / (account_id + '.json'))
    status, payload = request('/v2/plugin/auth/state?platform=workbuddy', post=True)
    data = payload.get('data') or {}
    state, login_url = data.get('state'), data.get('authUrl')
    if status != 200 or not isinstance(state, str) or not isinstance(login_url, str) or not valid_login_url(login_url):
        raise ValueError('官方未提供有效授权页面，未改动账号')
    flow_id = uuid.uuid4().hex
    flow = dict(account_id=account_id, expected_uid=record['uid'],
        previous_revision=revision(record['snapshot']), state=state, login_url=login_url,
        expires_at=time.time() + 300, status='waiting')
    write_encrypted(PROFILE / 'workbuddy-reauthorization' / (flow_id + '.json'), flow)
    return dict(flowId=flow_id, expectedAccount=record['display_name'],
        expectedPhone=record.get('masked_phone'), loginUrl=login_url, expiresIn=300)


def poll(flow_id):
    if not re.fullmatch(r'[0-9a-f]{32}', flow_id):
        raise ValueError('授权请求 ID 无效')
    path = PROFILE / 'workbuddy-reauthorization' / (flow_id + '.json')
    flow = read_encrypted(path)
    if flow.get('status') == 'cancelled':
        return dict(status='cancelled')
    if flow.get('status') == 'staged':
        return dict(status='staged', accountId=flow['account_id'])
    if flow['expires_at'] < time.time() and not flow.get('candidate'):
        return dict(status='expired')
    if not flow.get('candidate'):
        status, payload = request('/v2/plugin/auth/token?state=' + urllib.parse.quote(flow['state'], safe=''))
        data = payload.get('data') or {}
        if status != 200 or not isinstance(data, dict) or not data.get('accessToken'):
            return dict(status='waiting')
        # Preserve the newly issued auth before the following validation request can fail.
        flow['candidate'] = payload
        flow['received_at_ms'] = int(time.time() * 1000)
        write_encrypted(path, flow)
    auth = flow['candidate']['data']
    status, payload = request('/v2/plugin/account', {'Authorization': 'Bearer ' + auth['accessToken'],
        'X-User-Id': flow['expected_uid'], 'X-Domain': auth.get('domain') or HOST})
    if status != 200 or payload.get('code') not in {0, 200}:
        raise ValueError('新授权已加密保留，但账号验证未通过；未写回账号')
    candidate_identity(auth, payload.get('data') or {}, flow['expected_uid'])
    record = read_encrypted(PROFILE / 'workbuddy_accounts' / (flow['account_id'] + '.json'))
    if revision(record['snapshot']) != flow['previous_revision']:
        raise ValueError('授权期间原账号凭证已变化；新授权仍加密保留，未覆盖较新凭证')
    pending = PROFILE / 'workbuddy-auth-pending' / (flow['account_id'] + '.json')
    if pending.exists():
        raise ValueError('已有待验证凭证，未覆盖；请先核对现有恢复任务')
    write_encrypted(pending, dict(previous_revision=flow['previous_revision'],
        received_at_ms=flow['received_at_ms'], payload=flow['candidate']))
    flow['status'] = 'staged'
    write_encrypted(path, flow)
    return dict(status='staged', accountId=flow['account_id'], identityVerified=True,
        message='新授权身份已核对，候选凭证已加密保存；等待原生事务验证并提交')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description='WorkBuddy 官方网页重新授权诊断')
    parser.add_argument('action', choices=['start', 'poll', 'cancel'])
    parser.add_argument('identifier')
    args = parser.parse_args()
    try:
        if args.action == 'cancel':
            if not re.fullmatch(r'[0-9a-f]{32}', args.identifier):
                raise ValueError('授权请求 ID 无效')
            path = PROFILE / 'workbuddy-reauthorization' / (args.identifier + '.json')
            flow = read_encrypted(path)
            flow['status'] = 'cancelled'
            write_encrypted(path, flow)
            result = dict(status='cancelled')
        else:
            result = start(args.identifier) if args.action == 'start' else poll(args.identifier)
        print(json.dumps(result, ensure_ascii=True))
    except Exception as error:
        # Never print exception details from cryptography or raw HTTP payloads.
        safe = str(error) if isinstance(error, ValueError) else type(error).__name__
        print(json.dumps({'status': 'error', 'message': safe}, ensure_ascii=True))
        raise SystemExit(1)
