"""Conservative adapter: ACP v1 text events -> OpenAI Chat Completion chunks.

Contract-only: no HTTP server, no OAuth credentials, no WorkBuddy calls.
Tool and permission events are NOT executed or silently approved.
"""
from __future__ import annotations
import json
from dataclasses import dataclass, field
from typing import Any
from acp import ACPConversation, ACPProtocolError

class NeedsUserApproval(ACPProtocolError):
    pass

class UnsupportedACPOutcome(ACPProtocolError):
    pass

def _chunk(text: str | None, finish_reason: str | None = None, role: bool = False) -> dict:
    delta: dict[str, Any] = {}
    if role:
        delta["role"] = "assistant"
    if text is not None:
        delta["content"] = text
    return {
        "id": "wb-official-protocol-fixture",
        "object": "chat.completion.chunk",
        "model": "workbuddy-cloud-official-contract",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
    }

def _sse(value: dict) -> str:
    return "data: " + json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n\n"

@dataclass
class ACPTextBridge:
    """Only valid when a legally authorized ACP session and prompt exists.

    Each response is scoped to one task_id/session_id and one prompt rpc_id.
    Upstream server requests are errors until explicit user approval is built.
    """
    task_id: str
    prompt_rpc_id: int = 3
    conversation: ACPConversation = field(init=False)
    sent_role: bool = field(default=False, init=False)
    done: bool = field(default=False, init=False)
    parts: list[str] = field(default_factory=list, init=False)

    def __post_init__(self) -> None:
        if not isinstance(self.prompt_rpc_id, int) or self.prompt_rpc_id < 1:
            raise ACPProtocolError("Invalid prompt RPC ID")
        self.conversation = ACPConversation(self.task_id, prompt_id=self.prompt_rpc_id)

    def accept(self, payload: dict) -> list[str]:
        """Consume one ACP JSON-RPC message; return zero or more OpenAI SSE frames."""
        if self.done:
            raise ACPProtocolError("The prompt turn is already closed")
        if not isinstance(payload, dict) or payload.get("jsonrpc") != "2.0":
            raise ACPProtocolError("Invalid ACP JSON-RPC message")
        method = payload.get("method")
        params = payload.get("params")
        if method in ("session/update", "session/request_permission"):
            if not isinstance(params, dict) or params.get("sessionId") != self.task_id:
                raise ACPProtocolError("ACP event belongs to another session")
        outcome = self.conversation.accept(payload)
        if outcome == "permission_required":
            raise NeedsUserApproval("WorkBuddy needs explicit approval; adapter cannot auto-approve")
        if outcome == "rpc_error":
            raise UnsupportedACPOutcome("WorkBuddy ACP returned a JSON-RPC error")
        if outcome == "session_update":
            update = params.get("update")
            if not isinstance(update, dict):
                raise ACPProtocolError("Malformed ACP session update")
            if update.get("sessionUpdate") != "agent_message_chunk":
                # Do not expose thoughts, tools, user replay, raw artifacts as assistant answers.
                return []
            content = update.get("content")
            if not isinstance(content, dict) or content.get("type") != "text":
                # Image and complex content require a separate explicitly supported adapter.
                return []
            text = content.get("text")
            if not isinstance(text, str):
                raise ACPProtocolError("Malformed ACP assistant text block")
            if not text:
                return []
            frames = []
            if not self.sent_role:
                frames.append(_sse(_chunk(None, role=True)))
                self.sent_role = True
            self.parts.append(text)
            frames.append(_sse(_chunk(text)))
            return frames
        if outcome == "completed":
            reason = self.conversation.stop_reason
            if reason != "end_turn":
                raise UnsupportedACPOutcome("ACP turn did not end normally")
            self.done = True
            frames = []
            if not self.sent_role:
                frames.append(_sse(_chunk(None, role=True)))
                self.sent_role = True
            frames.extend([_sse(_chunk(None, finish_reason="stop")), "data: [DONE]\n\n"])
            return frames
        return []

    def completed_message(self) -> dict:
        if not self.done:
            raise ACPProtocolError("ACP prompt is incomplete")
        return {
            "id": "wb-official-protocol-fixture",
            "object": "chat.completion",
            "model": "workbuddy-cloud-official-contract",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "".join(self.parts)},
                "finish_reason": "stop",
            }],
        }
