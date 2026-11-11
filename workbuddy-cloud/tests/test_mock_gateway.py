"""HTTP black-box tests for the local ACP-to-OpenAI simulator.

Never opens a remote network connection and never sends WorkBuddy credentials.
"""
import json
import os
import socket
import subprocess
import sys
import time
import unittest
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ROOT=Path(__file__).resolve().parents[1]
KEY="local-test-key-no-real-account-12345"
with socket.socket() as sock:
    sock.bind(("127.0.0.1",0))
    PORT=sock.getsockname()[1]
BASE=f"http://127.0.0.1:{PORT}"

def send(path,body=None,method="GET",auth=True):
    headers={}
    if auth:
        headers["Authorization"]="Bearer "+KEY
    if body is not None:
        headers["Content-Type"]="application/json"
    request=Request(BASE+path,data=json.dumps(body).encode() if body is not None else None,headers=headers,method=method)
    try:
        with urlopen(request,timeout=5) as response:
            return response.status,response.headers.get("Content-Type"),response.read().decode("utf-8")
    except HTTPError as err:
        return err.code,err.headers.get("Content-Type"),err.read().decode("utf-8")

class HTTPContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        env=os.environ.copy()
        env.update(WB_MOCK_CLIENT_KEY=KEY,WB_MOCK_PORT=str(PORT),WB_MOCK_BIND="127.0.0.1")
        cls.process=subprocess.Popen([sys.executable,str(ROOT/"mock_openai_server.py")],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for _ in range(80):
            if cls.process.poll() is not None:
                raise RuntimeError("Mock process exited unexpectedly")
            try:
                with urlopen(BASE+"/healthz",timeout=.2):
                    break
            except OSError:
                time.sleep(.05)
        else:
            raise RuntimeError("Mock startup timeout")

    @classmethod
    def tearDownClass(cls):
        cls.process.terminate()
        try:
            cls.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            cls.process.kill()
            cls.process.wait()

    def payload(self,stream=False):
        return {"model":"workbuddy-official-acp-mock","messages":[{"role":"user","content":"hello"}],"stream":stream}

    def test_health_exposes_only_mock_status(self):
        status,_,body=send("/healthz",auth=False)
        self.assertEqual(status,200)
        self.assertEqual(json.loads(body)["upstream"],"disabled")

    def test_bearer_key_required(self):
        self.assertEqual(send("/v1/models",auth=False)[0],401)

    def test_model_list(self):
        status,_,body=send("/v1/models")
        self.assertEqual(status,200)
        self.assertEqual(json.loads(body)["data"][0]["id"],"workbuddy-official-acp-mock")

    def test_non_streaming_response(self):
        status,_,body=send("/v1/chat/completions",self.payload(),method="POST")
        self.assertEqual(status,200)
        result=json.loads(body)
        self.assertEqual(result["choices"][0]["message"]["content"],"WORKBUDDY_ACP_MOCK_OK")
        self.assertEqual(result["model"],"workbuddy-official-acp-mock")

    def test_streaming_response(self):
        status,ct,body=send("/v1/chat/completions",self.payload(stream=True),method="POST")
        self.assertEqual(status,200)
        self.assertIn("text/event-stream",ct)
        self.assertIn("WORKBUDDY_ACP_MOCK_OK",body)
        self.assertTrue(body.endswith("data: [DONE]\n\n"))

    def test_no_responses_api(self):
        self.assertEqual(send("/v1/responses",self.payload(),method="POST")[0],404)

    def test_reject_other_model(self):
        wrong=self.payload()
        wrong["model"]="not-authorized"
        self.assertEqual(send("/v1/chat/completions",wrong,method="POST")[0],400)

    def test_deny_public_bind(self):
        env=os.environ.copy()
        env.update(WB_MOCK_CLIENT_KEY=KEY,WB_MOCK_BIND="0.0.0.0")
        result=subprocess.run([sys.executable,str(ROOT/"mock_openai_server.py")],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=4)
        self.assertNotEqual(result.returncode,0)

if __name__=="__main__":
    unittest.main(verbosity=2)
