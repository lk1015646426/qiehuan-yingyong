"""Safe ACP to OpenAI contract tests. No network and no real WorkBuddy account."""
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bridge import ACPTextBridge, NeedsUserApproval, UnsupportedACPOutcome
from acp import ACPProtocolError, SSEParser, decode_jsonrpc

SESSION="2076261663968759808"
def update(kind, content, *, session=SESSION):
    return {"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":session,"update":{"sessionUpdate":kind,"content":content}
    }}

def finish(reason="end_turn", rpc_id=3):
    return {"jsonrpc":"2.0","id":rpc_id,"result":{"stopReason":reason}}

class ACPBridgeTests(unittest.TestCase):
    def test_text_stream_and_finish(self):
        bridge=ACPTextBridge(SESSION)
        frames=[]
        frames.extend(bridge.accept(update("agent_message_chunk",{"type":"text","text":"你好"})))
        frames.extend(bridge.accept(update("agent_message_chunk",{"type":"text","text":"，世界"})))
        frames.extend(bridge.accept(finish()))
        self.assertTrue(bridge.done)
        self.assertEqual(len(frames),5)  # role, first text, second text, final, [DONE]
        self.assertTrue(frames[-1].endswith("[DONE]\n\n"))
        parsed=[json.loads(s.removeprefix("data: ").strip()) for s in frames[:-1]]
        self.assertEqual(parsed[0]["choices"][0]["delta"]["role"],"assistant")
        self.assertEqual(parsed[1]["choices"][0]["delta"]["content"],"你好")
        self.assertEqual(parsed[2]["choices"][0]["delta"]["content"],"，世界")
        self.assertEqual(parsed[-1]["choices"][0]["finish_reason"],"stop")
        self.assertEqual(bridge.completed_message()["choices"][0]["message"]["content"],"你好，世界")

    def test_ignores_user_thoughts_tools_and_artifacts(self):
        bridge=ACPTextBridge(SESSION)
        for kind in ("user_message_chunk","agent_thought_chunk","tool_call","tool_call_update","plan"):
            self.assertEqual(bridge.accept(update(kind,{"type":"text","text":"hidden-not-an-answer"})),[])
        artifact={"jsonrpc":"2.0","method":"_codebuddy.ai/artifact","params":{"sessionId":SESSION,"artifact":{}}}
        self.assertEqual(bridge.accept(artifact),[])
        frames=bridge.accept(update("agent_message_chunk",{"type":"text","text":"visible"}))
        self.assertEqual(len(frames),2)
        self.assertNotIn("hidden-not-an-answer","".join(frames))

    def test_refuses_permission_and_no_success_marker(self):
        bridge=ACPTextBridge(SESSION)
        permission={"jsonrpc":"2.0","id":20,"method":"session/request_permission","params":{"sessionId":SESSION}}
        with self.assertRaises(NeedsUserApproval):
            bridge.accept(permission)
        self.assertFalse(bridge.done)
        self.assertEqual(len(bridge.conversation.pending_permissions),1)
        with self.assertRaises(ACPProtocolError):
            bridge.completed_message()

    def test_rejects_wrong_session(self):
        bridge=ACPTextBridge(SESSION)
        with self.assertRaises(ACPProtocolError):
            bridge.accept(update("agent_message_chunk",{"type":"text","text":"wrong"},session="12345"))

    def test_rejects_incomplete_cancellation_or_refusal(self):
        for reason in ("cancelled","refusal","max_tokens",None):
            bridge=ACPTextBridge(SESSION)
            with self.subTest(reason=reason), self.assertRaises(UnsupportedACPOutcome):
                bridge.accept(finish(reason))
            self.assertFalse(bridge.done)

    def test_other_rpc_id_does_not_complete(self):
        bridge=ACPTextBridge(SESSION)
        self.assertEqual(bridge.accept(finish(rpc_id=42)),[])
        self.assertFalse(bridge.done)

    def test_empty_message_can_complete(self):
        bridge=ACPTextBridge(SESSION)
        frames=bridge.accept(finish())
        self.assertEqual(len(frames),3)
        self.assertTrue(bridge.done)
        self.assertEqual(bridge.completed_message()["choices"][0]["message"]["content"],"")

    def test_sse_parser_end_to_end(self):
        bridge=ACPTextBridge(SESSION)
        event=update("agent_message_chunk",{"type":"text","text":"test"})
        wire=("data: "+json.dumps(event)+"\n\n").encode()
        parser=SSEParser()
        frames=[]
        for byte in wire:
            for item in parser.feed(bytes([byte])):
                frames.extend(bridge.accept(decode_jsonrpc(item)))
        frames.extend(bridge.accept(finish()))
        self.assertEqual(bridge.completed_message()["choices"][0]["message"]["content"],"test")
        self.assertEqual(frames[-1],"data: [DONE]\n\n")

    def test_strict_input_validation(self):
        with self.assertRaises(ACPProtocolError):
            ACPTextBridge(SESSION,prompt_rpc_id=0)
        bridge=ACPTextBridge(SESSION)
        with self.assertRaises(ACPProtocolError):
            bridge.accept("bad")
        with self.assertRaises(ACPProtocolError):
            bridge.accept(update("agent_message_chunk",{"type":"text","text":None}))

if __name__=="__main__":
    unittest.main(verbosity=2)
