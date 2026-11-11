"""Real HTTP contract test. Does not read accounts or access external networks."""
import json
import os
import socket
import subprocess
import sys
import time
import unittest
from urllib.error import HTTPError
from urllib.request import Request, urlopen

EXE = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else ""
sys.argv = [sys.argv[0]]
KEY = "local-fixture-key-no-real-credentials"
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    PORT = sock.getsockname()[1]
BASE = f"http://127.0.0.1:{PORT}"

def send(path, method="GET", body=None, auth=True):
    headers = {}
    if auth: headers["Authorization"] = f"Bearer {KEY}"
    if body is not None: headers["Content-Type"] = "application/json"
    request = Request(BASE + path,
                      data=json.dumps(body).encode() if body is not None else None,
                      headers=headers, method=method)
    try:
        with urlopen(request, timeout=6) as response:
            return response.status, response.headers.get("Content-Type"), response.read().decode()
    except HTTPError as error:
        return error.code, error.headers.get("Content-Type"), error.read().decode()

class Contract(unittest.TestCase):
    def test_health_no_external_upstream(self):
        code, _, body = send("/healthz", auth=False)
        self.assertEqual(code, 200)
        self.assertEqual(json.loads(body)["upstream"], "disabled")
    def test_reject_missing_token(self):
        self.assertEqual(send("/v1/models", auth=False)[0], 401)
    def test_models(self):
        code, _, body = send("/v1/models")
        self.assertEqual(code, 200)
        self.assertEqual(json.loads(body)["data"][0]["id"], "workbuddy-pool-p0-mock")
    def test_non_stream(self):
        code, ct, body = send("/v1/chat/completions", "POST", {
            "model":"workbuddy-pool-p0-mock","messages":[{"role":"user","content":"ping"}],"stream":False
        })
        self.assertEqual(code, 200)
        self.assertIn("application/json", ct)
        self.assertEqual(json.loads(body)["choices"][0]["message"]["content"], "P0_MOCK_OK")
    def test_stream_sse(self):
        code, ct, body = send("/v1/chat/completions", "POST", {
            "model":"workbuddy-pool-p0-mock","messages":[{"role":"user","content":"ping"}],"stream":True
        })
        self.assertEqual(code, 200)
        self.assertIn("text/event-stream", ct)
        self.assertIn("P0_MOCK_OK", body)
        self.assertTrue(body.endswith("data: [DONE]\n\n"))
    def test_reject_unknown_model(self):
        code, _, _ = send("/v1/chat/completions", "POST", {
            "model":"invalid","messages":[{"role":"user","content":"ping"}],"stream":True
        })
        self.assertEqual(code,400)
    def test_responses_unsupported(self):
        self.assertEqual(send("/v1/responses", "POST", {"model":"test"})[0],404)

if __name__ == "__main__":
    if not EXE or not os.path.isfile(EXE):
        sys.exit("usage: python cloud-p0/tests/contract.py <compiled-p0-executable>")
    env = os.environ.copy()
    env.update(P0_API_KEY=KEY,P0_BIND=f"127.0.0.1:{PORT}")
    proc = subprocess.Popen([EXE],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:
        for _ in range(80):
            if proc.poll() is not None: raise RuntimeError("P0 process exited prematurely")
            try:
                with urlopen(BASE + "/healthz",timeout=.2): break
            except OSError: time.sleep(.1)
        else: raise RuntimeError("P0 service startup timeout")
        unittest.main(verbosity=2)
    finally:
        proc.terminate()
        try: proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
