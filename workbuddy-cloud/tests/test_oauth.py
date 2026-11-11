"""Contract tests for documented official OAuth exchange and refresh, mock only."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from oauth_tokens import OAuthClient, OAuthTokens
from official import WorkBuddyOfficialError

class OAuthContractTests(unittest.TestCase):
    def test_exchange_token_uses_registered_app_credentials(self):
        seen=[]
        def fake(post):
            seen.append(post.copy())
            return {"access_token":"offline-demo-access","token_type":"Bearer","expires_in":3600,
                    "refresh_token":"offline-demo-refresh","scope":"user.task.invokable user.task.readable"}
        c=OAuthClient("app_offline_id","fake-server-only-secret",transport=fake)
        t=c.exchange("one-time-code","https://example.org/callback")
        self.assertEqual(t.scope,"user.task.invokable user.task.readable")
        self.assertEqual(seen[0]["grant_type"],"authorization_code")
        self.assertEqual(seen[0]["redirect_uri"],"https://example.org/callback")
        self.assertEqual(seen[0]["client_secret"],"fake-server-only-secret")
        self.assertNotIn("offline-demo-access",repr(t))
        self.assertNotIn("offline-demo-refresh",repr(t))

    def test_refresh_retains_or_rotates_token(self):
        seen=[]
        def same(post):
            seen.append(post)
            return {"access_token":"next-access","token_type":"Bearer","expires_in":3600}
        c=OAuthClient("app","fake-secret",transport=same)
        t=c.refresh("previous-refresh")
        self.assertEqual(t.refresh_token,"previous-refresh")
        self.assertEqual(seen[0]["grant_type"],"refresh_token")
        def rotate(_):
            return {"access_token":"later-access","token_type":"Bearer","expires_in":3600,
                    "refresh_token":"new-refresh"}
        newer=OAuthClient("app","fake-secret",transport=rotate).refresh("previous-refresh")
        self.assertEqual(newer.refresh_token,"new-refresh")

    def test_invalid_token_response_is_denied(self):
        for response in (
            {},
            {"access_token":"a","token_type":"Unknown","expires_in":3600},
            {"access_token":"a","token_type":"Bearer","expires_in":0},
            {"access_token":"a","token_type":"Bearer","expires_in":"3600"}
        ):
            c=OAuthClient("app","fake-secret",transport=lambda _:response)
            with self.assertRaises(WorkBuddyOfficialError):
                c.exchange("one-time-code","https://example.org/callback")

    def test_reject_insecure_redirect_and_blank_secret(self):
        with self.assertRaises(WorkBuddyOfficialError):
            OAuthClient("app","")
        c=OAuthClient("app","fake-secret",transport=lambda _: {})
        with self.assertRaises(WorkBuddyOfficialError):
            c.exchange("code","http://example.org/callback")

if __name__=="__main__":
    unittest.main(verbosity=2)
