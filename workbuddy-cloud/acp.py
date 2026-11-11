"""WorkBuddy ACP protocol framing, message envelopes and cautious state tracking.

The documented ACP transport uses GET SSE and POST JSON-RPC linked via
Acp-Connection-Id. This module formats messages and decodes events; it does NOT
establish a live connection, auto-approve permissions or execute tools.
"""
from __future__ import annotations
import codecs
import json
from dataclasses import dataclass, field
from typing import Any

class ACPProtocolError(ValueError):
    pass

def request_initialize(rpc_id: int = 1) -> dict:
    return {"jsonrpc":"2.0","id":rpc_id,"method":"initialize","params":{
        "protocolVersion":1,
        "clientCapabilities":{"fs":{"readTextFile":False,"writeTextFile":False}}
    }}

def request_load(session_id: str, rpc_id: int = 2) -> dict:
    _validate_session_id(session_id)
    return {"jsonrpc":"2.0","id":rpc_id,"method":"session/load","params":{
        "sessionId":session_id,"cwd":"/workspace","mcpServers":[]
    }}

def request_prompt(session_id: str, text: str, rpc_id: int = 3) -> dict:
    _validate_session_id(session_id)
    if not isinstance(text,str) or not text.strip():
        raise ACPProtocolError("prompt text must not be empty")
    return {"jsonrpc":"2.0","id":rpc_id,"method":"session/prompt","params":{
        "sessionId":session_id,"prompt":[{"type":"text","text":text}]
    }}

def _validate_session_id(session_id: str) -> None:
    if not isinstance(session_id,str) or not session_id.isdigit() or len(session_id)>64:
        raise ACPProtocolError("invalid task/session id")

@dataclass(frozen=True)
class SSEMessage:
    event: str
    data: str

class SSEParser:
    """Incremental UTF-8 SSE parser; handles multi-line data and split byte chunks."""
    def __init__(self, max_pending: int = 1024*1024):
        self._decoder=codecs.getincrementaldecoder("utf-8")("strict")
        self._line_buf=""
        self._event="message"
        self._data: list[str]=[]
        self._max_pending=max_pending

    def feed(self, chunk: bytes) -> list[SSEMessage]:
        try: text=self._decoder.decode(chunk)
        except UnicodeDecodeError as exc:
            raise ACPProtocolError("Invalid UTF-8 in SSE stream") from exc
        self._line_buf+=text
        if len(self._line_buf)>self._max_pending:
            raise ACPProtocolError("SSE pending line too long")
        found=[]
        while "\n" in self._line_buf:
            line,self._line_buf=self._line_buf.split("\n",1)
            line=line.rstrip("\r")
            if not line:
                if self._data:
                    found.append(SSEMessage(self._event,"\n".join(self._data)))
                self._event="message"
                self._data=[]
            elif line.startswith(":"):
                continue
            elif line.startswith("event:"):
                self._event=line[6:].lstrip(" ")
            elif line.startswith("data:"):
                self._data.append(line[5:].lstrip(" "))
        if sum(map(len,self._data))>self._max_pending:
            raise ACPProtocolError("SSE accumulated data too large")
        return found

def decode_jsonrpc(event: SSEMessage) -> dict:
    try: obj=json.loads(event.data)
    except ValueError as exc:
        raise ACPProtocolError("SSE data is not JSON-RPC JSON") from exc
    if not isinstance(obj,dict) or obj.get("jsonrpc")!="2.0":
        raise ACPProtocolError("Invalid JSON-RPC envelope")
    return obj

@dataclass
class ACPConversation:
    """Protocol signals only; cannot infer OpenAI deltas from unknown update shapes."""
    session_id: str
    prompt_id: int = 3
    initialized: bool=False
    loaded: bool=False
    complete: bool=False
    stop_reason: str|None=None
    events: list[dict] = field(default_factory=list)
    pending_permissions: list[dict] = field(default_factory=list)

    def __post_init__(self) -> None:
        _validate_session_id(self.session_id)

    def accept(self, message: dict) -> str:
        if message.get("jsonrpc")!="2.0":
            raise ACPProtocolError("Not JSON-RPC 2.0")
        method=message.get("method")
        if method=="session/request_permission":
            # Client must explicitly ask the user. Never auto-approve.
            self.pending_permissions.append(message)
            return "permission_required"
        if method=="session/update":
            self.events.append(message)
            return "session_update"
        if isinstance(method,str) and method.startswith("_codebuddy.ai/"):
            self.events.append(message)
            return "extension"
        if message.get("id")==1 and "result" in message:
            self.initialized=True
            return "initialized"
        if message.get("id")==2 and "result" in message:
            self.loaded=True
            return "loaded"
        if message.get("id")==self.prompt_id and "result" in message:
            result=message["result"]
            self.stop_reason=result.get("stopReason") if isinstance(result,dict) else None
            self.complete=True
            return "completed"
        if "error" in message:
            return "rpc_error"
        return "other"

    def permission_decision(self, *args, **kwargs):
        raise ACPProtocolError("P0 refuses automated permission responses; interactive approval required")
