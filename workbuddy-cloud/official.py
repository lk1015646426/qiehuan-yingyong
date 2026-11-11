"""Official WorkBuddy Open API contract client. No desktop-session credentials.

Only documented OAuth 2.1 and cloud task APIs are supported.
This module is not an OpenAI Chat Completions API and has no model quota access.
"""
from __future__ import annotations

import json
import secrets
from dataclasses import dataclass
from typing import Callable, Any
from urllib.parse import urlencode, urlsplit, urlunsplit
from urllib.request import Request, build_opener, HTTPRedirectHandler
from urllib.error import HTTPError, URLError

API_HOST = "www.workbuddy.cn"
BASE = f"https://{API_HOST}/openapi/v2"
MAX_RESPONSE = 512 * 1024

class WorkBuddyOfficialError(Exception):
    """Sanitized error. Never include tokens or raw provider response."""

class DenyRedirects(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise WorkBuddyOfficialError("Provider redirect rejected")

def _https_redirect_uri(uri: str) -> str:
    try:
        parts = urlsplit(uri)
        if parts.scheme != "https" or not parts.hostname or parts.username or parts.password or parts.fragment:
            raise ValueError("invalid redirect_uri")
    except ValueError as exc:
        raise WorkBuddyOfficialError("OAuth callback must be an HTTPS URL without credentials or fragment") from exc
    return uri

def authorization_url(client_id: str, redirect_uri: str, *, scopes=("user.task.invokable","user.task.readable"), state: str | None = None) -> tuple[str,str]:
    """Return the official browser consent URL and anti-CSRF state; do not open it."""
    if not client_id or len(client_id) > 256:
        raise WorkBuddyOfficialError("Invalid registered client_id")
    redirect_uri = _https_redirect_uri(redirect_uri)
    state = state or secrets.token_urlsafe(24)
    if len(state) < 16:
        raise WorkBuddyOfficialError("OAuth state must be unpredictable and at least 16 characters")
    allowed = {"user.task.invokable","user.task.readable"}
    if not scopes or any(scope not in allowed for scope in scopes):
        raise WorkBuddyOfficialError("Unsupported requested OAuth scope")
    query = urlencode({
        "response_type":"code", "client_id":client_id, "redirect_uri":redirect_uri,
        "scope":" ".join(scopes), "state":state
    })
    return f"{BASE}/authorize?{query}", state

def verify_callback(received_state: str, expected_state: str, code: str) -> str:
    if not received_state or not secrets.compare_digest(received_state, expected_state):
        raise WorkBuddyOfficialError("OAuth state mismatch")
    if not code or len(code)>2048:
        raise WorkBuddyOfficialError("Invalid authorization code")
    return code

def _official_url(path: str) -> str:
    if path == "/tasks":
        return BASE + path
    if path.startswith("/tasks/"):
        task_id = path[len("/tasks/"):]
        if not task_id.isdigit() or len(task_id)>64:
            raise WorkBuddyOfficialError("Invalid WorkBuddy task_id")
        return BASE + path
    raise WorkBuddyOfficialError("Undocumented WorkBuddy API path rejected")

def real_transport(method: str, path: str, access_token: str, payload: dict | None) -> dict:
    """For use after documented OAuth permission is granted. Never follows redirects."""
    url = _official_url(path)
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    request = Request(url, data=data, method=method, headers={
        "Authorization": f"Bearer {access_token}",
        "Accept": "application/json",
        **({"Content-Type":"application/json"} if payload is not None else {})
    })
    opener=build_opener(DenyRedirects)
    try:
        with opener.open(request, timeout=20) as response:
            if response.status not in (200,201):
                raise WorkBuddyOfficialError(f"Official WorkBuddy API returned HTTP {response.status}")
            data=response.read(MAX_RESPONSE+1)
    except WorkBuddyOfficialError:
        raise
    except (HTTPError, URLError, TimeoutError) as exc:
        if isinstance(exc,HTTPError) and exc.code in (401,403):
            raise WorkBuddyOfficialError("OAuth token invalid or scope not authorized") from None
        raise WorkBuddyOfficialError("Official WorkBuddy API unavailable or rejected request") from None
    if len(data)>MAX_RESPONSE:
        raise WorkBuddyOfficialError("Official API response exceeds size limit")
    try:
        result=json.loads(data)
    except (ValueError,UnicodeDecodeError):
        raise WorkBuddyOfficialError("Official API returned invalid JSON") from None
    if not isinstance(result,dict):
        raise WorkBuddyOfficialError("Official API response is not an object")
    return result

Transport = Callable[[str,str,str,dict|None],dict]

@dataclass(frozen=True)
class Task:
    task_id: str
    status: str
    link: str | None
    # Do not store ACP task ticket in a printable Task.
    token_available: bool

    @classmethod
    def from_response(cls, payload: dict) -> "Task":
        task_id=payload.get("task_id")
        status=payload.get("status")
        if not isinstance(task_id,str) or not task_id.isdigit() or not isinstance(status,str):
            raise WorkBuddyOfficialError("Malformed cloud task response")
        link=payload.get("link")
        if link is not None:
            parts=urlsplit(link)
            if parts.scheme!="https" or not parts.hostname or parts.username or parts.password:
                raise WorkBuddyOfficialError("Invalid ACP link")
        return cls(task_id,status,link, bool(payload.get("token")))

class TasksClient:
    def __init__(self, access_token: str, transport: Transport=real_transport):
        if not access_token:
            raise WorkBuddyOfficialError("An official OAuth access_token is required")
        self._token=access_token
        self._transport=transport

    def create_task(self, prompt: str, name: str | None = None) -> Task:
        if not isinstance(prompt,str) or not prompt.strip() or len(prompt)>16000:
            raise WorkBuddyOfficialError("Prompt must contain 1 to 16000 characters")
        body={"prompt":prompt}
        if name is not None:
            if not isinstance(name,str) or not name.strip() or len(name)>120:
                raise WorkBuddyOfficialError("Invalid task name")
            body["name"]=name
        response=self._transport("POST","/tasks",self._token,body)
        return Task.from_response(response)

    def get_task(self, task_id: str) -> Task:
        if not isinstance(task_id,str):
            raise WorkBuddyOfficialError("Invalid task_id")
        _official_url("/tasks/"+task_id)
        response=self._transport("GET","/tasks/"+task_id,self._token,None)
        return Task.from_response(response)
