"""Offline contract tests for WorkBuddy's documented OAuth/tasks/ACP shapes.

All tokens, task IDs and HTTP payloads below are synthetic fixtures.
"""
import json
import sys
import unittest
from pathlib import Path
from urllib.parse import parse_qs,urlsplit

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from official import TasksClient, Task, WorkBuddyOfficialError, authorization_url, verify_callback, _official_url
from acp import (SSEParser, SSEMessage, ACPConversation, ACPProtocolError,
                 request_initialize, request_load, request_prompt, decode_jsonrpc)

TASK_ID="2076261663968759808"
FAKE_TOKEN="offline-test-token-no-real-session"

class WorkBuddyOfficialTests(unittest.TestCase):
    def test_authorize_link_and_state(self):
        url,state=authorization_url("app_offline_demo","https://example.org/callback")
        parts=urlsplit(url)
        q=parse_qs(parts.query)
        self.assertEqual(parts.netloc,"www.workbuddy.cn")
        self.assertEqual(parts.path,"/openapi/v2/authorize")
        self.assertEqual(q["response_type"],["code"])
        self.assertIn("user.task.invokable",q["scope"][0])
        self.assertGreaterEqual(len(state),16)
        self.assertEqual(verify_callback(state,state,"demo-code"),"demo-code")

    def test_callback_rejects_state_mismatch(self):
        with self.assertRaises(WorkBuddyOfficialError):
            verify_callback("attacker-state-1234567890","real-state-123456789012345","code")

    def test_invalid_redirect_and_scopes(self):
        for redirect in ("http://example.com/callback","https://u:p@example.org/callback","https://example.org/#part"):
            with self.assertRaises(WorkBuddyOfficialError):
                authorization_url("app_demo",redirect)
        with self.assertRaises(WorkBuddyOfficialError):
            authorization_url("app_demo","https://example.org/callback",scopes=("admin.all",))

    def test_task_creation_official_contract(self):
        requests=[]
        def fake(method,path,token,payload):
            requests.append((method,path,token,payload))
            return {"task_id":TASK_ID,"status":"working",
                    "link":f"https://acp.workbuddy.cn/sessions/{TASK_ID}",
                    "token":"TEST_TICKET_NOT_REAL"}
        client=TasksClient(FAKE_TOKEN,transport=fake)
        task=client.create_task("今天有什么安排","日程")
        self.assertEqual(task.task_id,TASK_ID)
        self.assertTrue(task.token_available)
        self.assertEqual(requests,[("POST","/tasks",FAKE_TOKEN,{"prompt":"今天有什么安排","name":"日程"})])
        self.assertNotIn("TEST_TICKET_NOT_REAL",repr(task))

    def test_task_query_and_path_protection(self):
        requests=[]
        def fake(method,path,token,payload):
            requests.append((method,path))
            return {"task_id":TASK_ID,"status":"working"}
        client=TasksClient(FAKE_TOKEN,transport=fake)
        self.assertEqual(client.get_task(TASK_ID).task_id,TASK_ID)
        self.assertEqual(requests,[("GET","/tasks/"+TASK_ID)])
        for path in ("../../private", "abc", "%2F", "1?x=2"):
            with self.assertRaises(WorkBuddyOfficialError):
                client.get_task(path)
        with self.assertRaises(WorkBuddyOfficialError):
            _official_url("/v2/plugin/auth/token/refresh")

    def test_no_implicit_unapproved_credentials(self):
        with self.assertRaises(WorkBuddyOfficialError):
            TasksClient("")
        with self.assertRaises(WorkBuddyOfficialError):
            TasksClient(FAKE_TOKEN).create_task("")

    def test_task_validation_refuses_invalid_links(self):
        with self.assertRaises(WorkBuddyOfficialError):
            Task.from_response({"task_id":TASK_ID,"status":"idle","link":"http://evil.example/sessions/1"})

class ACPTests(unittest.TestCase):
    def test_initialize_load_prompt_wire_shapes(self):
        init=request_initialize()
        load=request_load(TASK_ID)
        prompt=request_prompt(TASK_ID,"测试")
        self.assertEqual(init["method"],"initialize")
        self.assertFalse(init["params"]["clientCapabilities"]["fs"]["writeTextFile"])
        self.assertEqual(load["method"],"session/load")
        self.assertEqual(load["params"]["sessionId"],TASK_ID)
        self.assertEqual(prompt["method"],"session/prompt")
        self.assertEqual(prompt["params"]["prompt"],[{"type":"text","text":"测试"}])

    def test_incremental_utf8_sse_frames(self):
        body=('event: message\r\n'
              'data: {"jsonrpc":"2.0",\r\n'
              'data: "method":"session/update","params":{"text":"中文"}}\r\n\r\n').encode("utf-8")
        parser=SSEParser()
        found=[]
        for b in body:
            found.extend(parser.feed(bytes([b])))
        self.assertEqual(len(found),1)
        item=decode_jsonrpc(found[0])
        self.assertEqual(item["method"],"session/update")
        self.assertEqual(item["params"]["text"],"中文")

    def test_sse_disallows_giant_pending_line(self):
        parser=SSEParser(max_pending=10)
        with self.assertRaises(ACPProtocolError):
            parser.feed(b"x"*12)

    def test_session_completion_permission_and_opaque_updates(self):
        conv=ACPConversation(TASK_ID)
        self.assertEqual(conv.accept({"jsonrpc":"2.0","id":1,"result":{}}),"initialized")
        self.assertEqual(conv.accept({"jsonrpc":"2.0","id":2,"result":{}}),"loaded")
        self.assertEqual(conv.accept({"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}}}),"session_update")
        permission={"jsonrpc":"2.0","id":90,"method":"session/request_permission","params":{"sessionId":TASK_ID}}
        self.assertEqual(conv.accept(permission),"permission_required")
        self.assertFalse(conv.complete)
        self.assertEqual(len(conv.pending_permissions),1)
        with self.assertRaises(ACPProtocolError):
            conv.permission_decision()
        self.assertEqual(conv.accept({"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}),"completed")
        self.assertTrue(conv.complete)
        self.assertEqual(conv.stop_reason,"end_turn")

    def test_bad_rpc_and_session_id(self):
        with self.assertRaises(ACPProtocolError):
            request_load("not-numeric")
        with self.assertRaises(ACPProtocolError):
            request_prompt(TASK_ID," ")
        with self.assertRaises(ACPProtocolError):
            decode_jsonrpc(SSEMessage("message","data: not json"))

if __name__=="__main__":
    unittest.main(verbosity=2)
