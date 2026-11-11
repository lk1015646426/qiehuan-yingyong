"""Loopback-only OpenAI HTTP *contract simulator* for DSH Go integration tests.

No WorkBuddy account, ACP network connection, cloud task or model request.
Do not expose to LAN/public internet. This is a protocol mock, not AI.
"""
from __future__ import annotations
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from bridge import ACPTextBridge

MODEL = "workbuddy-official-acp-mock"
TASK_ID = "2076261663968759808"
MAX_BODY = 1024 * 1024

def _rpc_update(text: str) -> dict:
    return {"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":TASK_ID,
        "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}
    }}

class Handler(BaseHTTPRequestHandler):
    server_version = "WorkBuddy-ContractMock/0.1"
    sys_version = ""

    def log_message(self, format, *args):
        # Avoid logging authorization headers or prompts.
        pass

    def json(self, status: int, body: dict):
        data=json.dumps(body,ensure_ascii=False,separators=(",",":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type","application/json; charset=utf-8")
        self.send_header("Cache-Control","no-store")
        self.send_header("X-Content-Type-Options","nosniff")
        self.send_header("Content-Length",str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def error(self,status:int,message:str):
        self.json(status,{"error":{"message":message,"type":"contract_mock_error"}})

    def auth(self) -> bool:
        if self.headers.get("Authorization") != f"Bearer {self.server.api_key}":
            self.error(401,"Invalid test client key")
            return False
        return True

    def do_GET(self):
        if self.path == "/healthz":
            self.json(200,{"status":"ok","upstream":"disabled","environment":"mock-only"})
            return
        if not self.auth():
            return
        if self.path == "/v1/models":
            self.json(200,{"object":"list","data":[{"id":MODEL,"object":"model","owned_by":"contract-mock"}]})
        else:
            self.error(404,"Mock endpoint not found")

    def do_POST(self):
        if not self.auth():
            return
        if self.path != "/v1/chat/completions":
            self.error(404,"This mock only supports Chat Completions")
            return
        length=self.headers.get("Content-Length")
        if not length or not length.isdecimal() or int(length)>MAX_BODY:
            self.error(413,"Content-Length required and must be <= 1 MiB")
            return
        try:
            body=json.loads(self.rfile.read(int(length)))
        except (UnicodeDecodeError,ValueError):
            self.error(400,"Malformed JSON request")
            return
        if not isinstance(body,dict) or body.get("model")!=MODEL:
            self.error(400,"Unknown mock model")
            return
        messages=body.get("messages")
        if not isinstance(messages,list) or not messages:
            self.error(400,"messages must be a non-empty list")
            return
        if not isinstance(body.get("stream",False),bool):
            self.error(400,"stream must be boolean")
            return
        # Every reply is synthetic, making it impossible to accidentally run real agents.
        bridge=ACPTextBridge(TASK_ID)
        frames=[]
        for rpc in (_rpc_update("WORKBUDDY_ACP_MOCK_OK"), {"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}):
            frames.extend(bridge.accept(rpc))
        if not body.get("stream",False):
            answer=bridge.completed_message()
            answer["model"]=MODEL
            self.json(200,answer)
            return
        self.send_response(200)
        self.send_header("Content-Type","text/event-stream; charset=utf-8")
        self.send_header("Cache-Control","no-cache")
        self.send_header("X-Content-Type-Options","nosniff")
        self.end_headers()
        for frame in frames:
            self.wfile.write(frame.encode("utf-8"))
            self.wfile.flush()

def main():
    key=os.environ.get("WB_MOCK_CLIENT_KEY","")
    if len(key)<24:
        raise SystemExit("Set WB_MOCK_CLIENT_KEY to a 24+ character LOCAL TEST key")
    bind=os.environ.get("WB_MOCK_BIND","127.0.0.1")
    if bind != "127.0.0.1":
        raise SystemExit("Mock gateway refuses non-loopback bindings")
    port=int(os.environ.get("WB_MOCK_PORT","18792"))
    if port<1024 or port>65535:
        raise SystemExit("Invalid local port")
    server=ThreadingHTTPServer((bind,port),Handler)
    server.api_key=key
    try:
        print(f"ACP contract mock on http://{bind}:{port} (no WorkBuddy upstream)",flush=True)
        server.serve_forever(poll_interval=.05)
    finally:
        server.server_close()

if __name__=="__main__":
    main()
