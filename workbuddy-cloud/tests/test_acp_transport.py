"""Real two-channel ACP HTTP fixture tests. No WorkBuddy calls or real tickets."""
from __future__ import annotations

import json
import queue
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from official import Task
from acp_transport import (
    ACPTransportError, ACPChannel, stream_prompt, validate_acp_endpoint,
)
from bridge import NeedsUserApproval

TASK_ID = "2076261663968759808"
TICKET = "synthetic-acp-ticket-no-real-credentials"
CONNECTION = "fixture-connection-001"

class ACPFixtureHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        self.server.record.append(("GET", self.path, self.headers.get("Authorization")))
        if self.path != "/acp" or self.headers.get("Authorization") != f"Bearer {TICKET}":
            self.send_error(401)
            return
        connection_queue = queue.Queue()
        with self.server.active_lock:
            self.server.active_queue = connection_queue
            self.server.all_queues.append(connection_queue)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Acp-Connection-Id", CONNECTION)
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()
        self.wfile.flush()
        while True:
            item = connection_queue.get()
            if item is None:
                break
            wire = ("data: " + json.dumps(item, ensure_ascii=False) + "\n\n").encode("utf-8")
            try:
                self.wfile.write(wire)
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError, OSError):
                break
        self.close_connection = True

    def do_POST(self):
        size = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(size)
        self.server.record.append((
            "POST",
            self.path,
            self.headers.get("Authorization"),
            self.headers.get("Acp-Connection-Id"),
            self.headers.get("Content-Type"),
        ))
        if (self.path != "/acp"
            or self.headers.get("Authorization") != f"Bearer {TICKET}"
            or self.headers.get("Acp-Connection-Id") != CONNECTION):
            self.send_error(401)
            return
        rpc = json.loads(raw.decode("utf-8"))
        self.server.rpcs.append(rpc)
        self.send_response(202)
        self.send_header("Content-Length", "0")
        self.end_headers()
        self.wfile.flush()
        with self.server.active_lock:
            target = self.server.active_queue
        if target is None:
            return
        if rpc["method"] in ("initialize", "session/load"):
            target.put({"jsonrpc":"2.0","id":rpc["id"],"result":{}})
        elif rpc["method"] == "session/prompt":
            if self.server.permission_mode:
                target.put({"jsonrpc":"2.0","id":100,
                    "method":"session/request_permission",
                    "params":{"sessionId":TASK_ID,"options":[{"optionId":"allow"}]}})
            else:
                for part in ("你好", "，ACP"):
                    target.put({
                        "jsonrpc":"2.0","method":"session/update",
                        "params":{"sessionId":TASK_ID,"update":{
                            "sessionUpdate":"agent_message_chunk",
                            "content":{"type":"text","text":part}
                        }},
                    })
                target.put({
                    "jsonrpc":"2.0","id":rpc["id"],"result":{"stopReason":"end_turn"},
                })

class ACPTransportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), ACPFixtureHandler)
        cls.server.record = []
        cls.server.rpcs = []
        cls.server.active_lock = threading.Lock()
        cls.server.active_queue = None
        cls.server.all_queues = []
        cls.server.permission_mode = False
        cls.background = threading.Thread(target=cls.server.serve_forever,daemon=True)
        cls.background.start()

    @classmethod
    def tearDownClass(cls):
        for connection_queue in cls.server.all_queues:
            connection_queue.put(None)
        cls.server.shutdown()
        cls.server.server_close()
        cls.background.join(timeout=3)

    def setUp(self):
        self.server.permission_mode = False
        self.server.record.clear()
        self.server.rpcs.clear()
        with self.server.active_lock:
            self.server.active_queue = None
        self.task = Task(
            TASK_ID, "working",
            f"http://127.0.0.1:{self.server.server_port}/acp",
            True, TICKET,
        )

    def test_real_sse_and_jsonrpc_http_closed_loop(self):
        frames=list(stream_prompt(self.task,"请说你好",allow_test_loopback=True))
        self.assertTrue(frames[-1].endswith("[DONE]\n\n"))
        joined="".join(frames)
        self.assertIn("你好",joined)
        self.assertIn("ACP",joined)
        self.assertEqual([r["method"] for r in self.server.rpcs],
                         ["initialize","session/load","session/prompt"])
        self.assertEqual(self.server.rpcs[1]["params"]["sessionId"],TASK_ID)
        self.assertEqual(self.server.rpcs[2]["params"]["prompt"][0]["text"],"请说你好")
        self.assertEqual(self.server.record[0][0],"GET")
        for req in self.server.record[1:]:
            self.assertEqual(req[3],CONNECTION)
            self.assertIn("application/json",req[4])

    def test_permission_halts_without_approval(self):
        self.server.permission_mode = True
        with self.assertRaises(NeedsUserApproval):
            list(stream_prompt(self.task,"needs tool",allow_test_loopback=True))
        self.assertEqual([r["method"] for r in self.server.rpcs],
                         ["initialize","session/load","session/prompt"])
        self.assertEqual(len(self.server.rpcs),3)

    def test_endpoint_is_pinned_to_official_hosts(self):
        for url in (
            "https://acp.workbuddy.cn/sessions/123",
            "https://65225-example.ap-guangzhou.agentos-run.net/acp",
        ):
            self.assertEqual(validate_acp_endpoint(url),url)
        for url in (
            "https://evil.example/acp",
            "http://acp.workbuddy.cn/acp",
            "https://acp.workbuddy.cn:4443/acp",
            "https://user:secret@acp.workbuddy.cn/acp",
            "https://acp.workbuddy.cn/acp?token=secret",
            "https://acp.workbuddy.cn.evil.example/acp",
            f"http://127.0.0.1:{self.server.server_port}/acp",
        ):
            with self.subTest(url=url), self.assertRaises(ACPTransportError):
                validate_acp_endpoint(url)

    def test_task_ticket_does_not_leak_through_repr(self):
        self.assertNotIn(TICKET,repr(self.task))
        parsed=Task.from_response({
            "task_id":TASK_ID,"status":"working",
            "link":"https://acp.workbuddy.cn/sessions/"+TASK_ID,
            "token":TICKET,
        })
        self.assertTrue(parsed.token_available)
        self.assertEqual(parsed.acp_ticket,TICKET)
        self.assertNotIn(TICKET,repr(parsed))

    def test_missing_ticket_rejected(self):
        no_ticket=Task(TASK_ID,"working",
                       "https://acp.workbuddy.cn/sessions/"+TASK_ID,False,None)
        with self.assertRaises(ACPTransportError):
            ACPChannel(no_ticket)

    def test_disallow_unsupported_rpcs(self):
        with ACPChannel(self.task, allow_test_loopback=True) as c:
            with self.assertRaises(ACPTransportError):
                c.send({"jsonrpc":"2.0","id":42,"method":"session/request_permission",
                        "params":{"allow":True}})

if __name__ == "__main__":
    unittest.main(verbosity=2)
