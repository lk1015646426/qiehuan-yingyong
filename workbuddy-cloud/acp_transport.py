"""Official ACP two-channel client: SSE GET + JSON-RPC POST linked by Acp-Connection-Id.

Only documented methods are sent. Requires a task ticket obtained through
proper OAuth task access. Never accesses Windows WorkBuddy client tokens.
This is a library, not a public HTTP service.
"""
from __future__ import annotations

import json
import re
from typing import Iterator
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import Request, build_opener

from official import DenyRedirects, Task
from acp import (
    ACPProtocolError, SSEParser, decode_jsonrpc,
    request_initialize, request_load, request_prompt,
)
from bridge import ACPTextBridge

CONNECT_TIMEOUT = 20
POST_TIMEOUT = 20
MAX_EVENT_LINE = 64 * 1024
MAX_RPC_BYTES = 64 * 1024
CONNECTION_ID = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
ALLOWED_METHODS = frozenset({"initialize", "session/load", "session/prompt"})

class ACPTransportError(ACPProtocolError):
    pass

def validate_acp_endpoint(link: str, *, allow_test_loopback: bool = False) -> str:
    """Allow only documented WorkBuddy ACP hosts, or opt-in local test HTTP.

    Dynamic AgentOS sandbox hostnames appear in official docs; no arbitrary
    third-party HTTPS URL may receive a short-lived ACP Bearer ticket.
    """
    if not isinstance(link, str) or len(link) > 4096:
        raise ACPTransportError("Missing or invalid ACP endpoint")
    try:
        parsed = urlsplit(link)
        host = parsed.hostname
        port = parsed.port
    except ValueError as exc:
        raise ACPTransportError("Malformed ACP URL") from exc
    if not host or parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ACPTransportError("Unsafe ACP URL")
    if allow_test_loopback and parsed.scheme == "http" and host == "127.0.0.1":
        if port is None or not 1024 <= port <= 65535:
            raise ACPTransportError("Invalid local fixture port")
        return link
    if parsed.scheme != "https" or port not in (None, 443):
        raise ACPTransportError("ACP requires HTTPS")
    host = host.lower().rstrip(".")
    if not (host == "acp.workbuddy.cn" or host.endswith(".agentos-run.net")):
        raise ACPTransportError("ACP destination is not an allowed official host")
    if not parsed.path or parsed.path == "/":
        raise ACPTransportError("ACP path missing")
    return link

class ACPChannel:
    """One connected official task: GET keeps SSE open; POST sends JSON-RPC."""

    def __init__(self, task: Task, *, allow_test_loopback: bool = False):
        if not isinstance(task, Task) or not task.acp_ticket or not task.link:
            raise ACPTransportError("Task is missing an authorized ACP link or ticket")
        self.task_id = task.task_id
        self._url = validate_acp_endpoint(task.link, allow_test_loopback=allow_test_loopback)
        self._ticket = task.acp_ticket
        self._opener = build_opener(DenyRedirects)
        self._sse = None
        self.connection_id = None
        self._parser = SSEParser(max_pending=MAX_EVENT_LINE * 2)

    def __enter__(self) -> "ACPChannel":
        request = Request(self._url, method="GET", headers={
            "Authorization": f"Bearer {self._ticket}",
            "Accept": "text/event-stream",
            "Cache-Control": "no-cache",
        })
        try:
            response = self._opener.open(request, timeout=CONNECT_TIMEOUT)
        except (HTTPError, URLError, TimeoutError, OSError):
            raise ACPTransportError("ACP SSE connection rejected or unavailable") from None
        if not response.headers.get("Content-Type", "").lower().startswith("text/event-stream"):
            response.close()
            raise ACPTransportError("ACP server returned non-SSE response")
        connection_id = response.headers.get("Acp-Connection-Id", "").strip()
        if not CONNECTION_ID.fullmatch(connection_id):
            response.close()
            raise ACPTransportError("ACP server omitted a valid Acp-Connection-Id")
        self._sse = response
        self.connection_id = connection_id
        return self

    def __exit__(self, *_):
        if self._sse is not None:
            self._sse.close()
        self._sse = None
        self.connection_id = None

    def send(self, rpc: dict) -> None:
        if self._sse is None or not self.connection_id:
            raise ACPTransportError("ACP SSE must be open before sending JSON-RPC")
        if not isinstance(rpc, dict) or rpc.get("jsonrpc") != "2.0":
            raise ACPTransportError("Invalid JSON-RPC request")
        if rpc.get("method") not in ALLOWED_METHODS or not isinstance(rpc.get("id"), int):
            raise ACPTransportError("Undocumented ACP method rejected")
        raw = json.dumps(rpc, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        if len(raw) > MAX_RPC_BYTES:
            raise ACPTransportError("ACP JSON-RPC request too large")
        request = Request(self._url, data=raw, method="POST", headers={
            "Authorization": f"Bearer {self._ticket}",
            "Content-Type": "application/json",
            "Accept": "application/json",
            "Acp-Connection-Id": self.connection_id,
        })
        try:
            with self._opener.open(request, timeout=POST_TIMEOUT) as response:
                if response.status not in (200, 201, 202, 204):
                    raise ACPTransportError("ACP JSON-RPC request rejected")
                # As defined by WorkBuddy, responses arrive on SSE; no POST body required.
        except ACPTransportError:
            raise
        except (HTTPError, URLError, TimeoutError, OSError):
            raise ACPTransportError("ACP JSON-RPC send failed") from None

    def messages(self) -> Iterator[dict]:
        if self._sse is None:
            raise ACPTransportError("ACP SSE connection not open")
        while True:
            try:
                line = self._sse.readline(MAX_EVENT_LINE + 1)
            except (OSError, TimeoutError):
                raise ACPTransportError("ACP SSE stream read failed or timed out") from None
            if not line:
                raise ACPTransportError("ACP SSE closed before expected result")
            if len(line) > MAX_EVENT_LINE:
                raise ACPTransportError("ACP SSE line size exceeded")
            for event in self._parser.feed(line):
                yield decode_jsonrpc(event)

def _await_result(channel: ACPChannel, expected_id: int) -> dict:
    for message in channel.messages():
        if message.get("method") == "session/request_permission":
            raise ACPTransportError("ACP requested manual approval during handshake")
        if message.get("id") != expected_id:
            continue
        if "error" in message or "result" not in message:
            raise ACPTransportError("ACP handshake failed")
        return message
    raise ACPTransportError("Missing ACP handshake result")

def stream_prompt(task: Task, text: str, *, allow_test_loopback: bool = False) -> Iterator[str]:
    """Generate OpenAI-compatible *text only* SSE frames for an authorized task.

    Raises on permission requests, incomplete prompts, protocol errors and
    abnormal stop reasons. Never acknowledges permissions or executes tools.
    """
    bridge = ACPTextBridge(task.task_id)
    with ACPChannel(task, allow_test_loopback=allow_test_loopback) as channel:
        channel.send(request_initialize(1))
        _await_result(channel, 1)
        channel.send(request_load(task.task_id, 2))
        _await_result(channel, 2)
        channel.send(request_prompt(task.task_id, text, 3))
        for message in channel.messages():
            for frame in bridge.accept(message):
                yield frame
            if bridge.done:
                return
    raise ACPTransportError("ACP prompt ended without normal completion")
