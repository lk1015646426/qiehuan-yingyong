"""Real localhost-to-localhost provider contract tests with synthetic keys only."""
import json
import os
import socket
import subprocess
import sys
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.request import Request, urlopen

EXE = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else ""
sys.argv = [sys.argv[0]]
CLIENT_KEY = "local-contract-client-key-not-a-secret"
PROVIDER_KEY = "local-provider-test-token"
MODEL = "qwen-plus"

class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def do_POST(self):
        if self.path != "/compatible-mode/v1/chat/completions":
            self.send_error(404); return
        if self.headers.get("Authorization") != f"Bearer {PROVIDER_KEY}":
            self.send_error(401); return
        size = int(self.headers.get("Content-Length", "0"))
        payload = json.loads(self.rfile.read(size))
        if payload["model"] != MODEL:
            self.send_error(400); return
        messages = payload.get("messages", [])
        last = messages[-1].get("content", "")
        if last == "simulate_upstream_429":
            self.send_response(429)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"error":{"message":"private provider details"}}')
            return
        if payload.get("stream") is True:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            for data in [
                {"id":"upstream","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"P1_AUTHORIZED_TEST_OK"},"finish_reason":None}]},
                {"id":"upstream","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]},
            ]:
                self.wfile.write(("data: "+json.dumps(data)+"\n\n").encode())
                self.wfile.flush()
                time.sleep(.01)
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        else:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            response = {"id":"upstream","model":MODEL,"object":"chat.completion",
                        "choices":[{"index":0,"message":{"role":"assistant","content":"P1_AUTHORIZED_TEST_OK"},"finish_reason":"stop"}]}
            if last == "tooltest":
                response["choices"][0]["message"] = {
                    "role":"assistant","content":None,
                    "tool_calls":[{"id":"tool1","type":"function","function":{"name":"example","arguments":"{}"}}]
                }
                response["choices"][0]["finish_reason"] = "tool_calls"
            self.wfile.write(json.dumps(response).encode())

class Cases(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        provider = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        cls.provider = provider
        cls.provider_thread = threading.Thread(target=provider.serve_forever, daemon=True)
        cls.provider_thread.start()
        with socket.socket() as s:
            s.bind(("127.0.0.1",0))
            cls.port = s.getsockname()[1]
        cls.url = f"http://127.0.0.1:{cls.port}"
        env = os.environ.copy()
        env.update({
            "P1_BIND": f"127.0.0.1:{cls.port}",
            "P1_MODEL": MODEL,
            "P1_CLIENT_API_KEY": CLIENT_KEY,
            "P1_UPSTREAM_API_KEY": PROVIDER_KEY,
            "P1_UPSTREAM_BASE_URL": f"http://127.0.0.1:{provider.server_port}/compatible-mode/v1",
            "P1_TEST_ALLOW_HTTP_LOOPBACK": "1"
        })
        cls.proc = subprocess.Popen([EXE], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(80):
            if cls.proc.poll() is not None:
                raise RuntimeError(f"P1 process terminated unexpectedly: {cls.proc.returncode}")
            try:
                with urlopen(cls.url + "/healthz", timeout=.2): break
            except OSError:
                time.sleep(.1)
        else:
            raise RuntimeError("P1 failed to start")

    @classmethod
    def tearDownClass(cls):
        cls.proc.terminate()
        try: cls.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            cls.proc.kill(); cls.proc.wait()
        cls.provider.shutdown()
        cls.provider.server_close()
        cls.provider_thread.join(timeout=5)

    def send(self, path, method="GET", body=None, auth=True):
        headers = {}
        if auth: headers["Authorization"] = "Bearer " + CLIENT_KEY
        if body is not None: headers["Content-Type"] = "application/json"
        req = Request(self.url + path, data=json.dumps(body).encode() if body is not None else None,
                      headers=headers, method=method)
        try:
            with urlopen(req, timeout=10) as resp:
                return resp.status, resp.headers.get("Content-Type"), resp.read().decode()
        except HTTPError as err:
            return err.code, err.headers.get("Content-Type"), err.read().decode()

    def payload(self, text="test", stream=False):
        return {"model":MODEL,"messages":[{"role":"user","content":text}],"stream":stream}

    def test_health_without_auth(self):
        code, _, body = self.send("/healthz", auth=False)
        self.assertEqual(code,200)
        self.assertEqual(json.loads(body)["backend"],"authorized-provider")
    def test_client_key_is_required(self):
        self.assertEqual(self.send("/v1/models",auth=False)[0],401)
    def test_model_list(self):
        code, _, body = self.send("/v1/models")
        self.assertEqual(code,200)
        self.assertEqual(json.loads(body)["data"][0]["id"],MODEL)
    def test_non_streaming_provider_relay(self):
        code, ct, body = self.send("/v1/chat/completions","POST",self.payload())
        self.assertEqual(code,200)
        self.assertIn("application/json",ct)
        self.assertEqual(json.loads(body)["choices"][0]["message"]["content"],"P1_AUTHORIZED_TEST_OK")
    def test_streaming_sse_relay(self):
        code, ct, body = self.send("/v1/chat/completions","POST",self.payload(stream=True))
        self.assertEqual(code,200)
        self.assertIn("text/event-stream",ct)
        self.assertIn("P1_AUTHORIZED_TEST_OK",body)
        self.assertTrue(body.endswith("data: [DONE]\n\n"))
    def test_tool_calls_forwarded_without_execution(self):
        code, _, body = self.send("/v1/chat/completions","POST",self.payload("tooltest"))
        self.assertEqual(code,200)
        self.assertEqual(json.loads(body)["choices"][0]["message"]["tool_calls"][0]["function"]["name"],"example")
    def test_model_allowlist(self):
        bad = self.payload(); bad["model"]="another-model"
        self.assertEqual(self.send("/v1/chat/completions","POST",bad)[0],400)
    def test_no_raw_upstream_error_leak(self):
        code, _, body = self.send("/v1/chat/completions","POST",self.payload("simulate_upstream_429"))
        self.assertEqual(code,429)
        self.assertNotIn("private provider details",body)
    def test_responses_unsupported(self):
        self.assertEqual(self.send("/v1/responses","POST",{"model":MODEL})[0],404)

if __name__ == "__main__":
    if not EXE or not os.path.isfile(EXE):
        sys.exit("usage: python cloud-p1/tests/contract.py <compiled-executable>")
    unittest.main(verbosity=2)
