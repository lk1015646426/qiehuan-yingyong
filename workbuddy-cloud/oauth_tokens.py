"""Official WorkBuddy OAuth 2.1 token exchange/refresh, using explicit app credentials.

This is NOT a private desktop session token refresh. No credentials are loaded from
WorkBuddy files. Production applications must store client_secret server-side.
"""
from __future__ import annotations
from dataclasses import dataclass
from typing import Callable
from urllib.parse import urlencode
from urllib.request import Request, build_opener
from urllib.error import HTTPError, URLError
import json

from official import BASE, WorkBuddyOfficialError, DenyRedirects, _https_redirect_uri

MAX_TOKEN_BODY = 128 * 1024

@dataclass(frozen=True,repr=False)
class OAuthTokens:
    access_token: str
    refresh_token: str | None
    expires_in: int
    scope: str

    def __repr__(self) -> str:
        return "OAuthTokens(<redacted>)"

    @classmethod
    def parse(cls, body: dict, prior_refresh_token: str | None = None) -> "OAuthTokens":
        access=body.get("access_token")
        token_type=body.get("token_type")
        expires=body.get("expires_in")
        if not isinstance(access,str) or not access or token_type!="Bearer":
            raise WorkBuddyOfficialError("OAuth token response invalid")
        if not isinstance(expires,int) or expires<=0 or expires>7*24*3600:
            raise WorkBuddyOfficialError("OAuth expiration invalid")
        refresh=body.get("refresh_token") or prior_refresh_token
        if refresh is not None and not isinstance(refresh,str):
            raise WorkBuddyOfficialError("OAuth refresh token invalid")
        scope=body.get("scope","")
        if not isinstance(scope,str):
            raise WorkBuddyOfficialError("OAuth scopes invalid")
        return cls(access,refresh,expires,scope)

def official_token_transport(payload: dict) -> dict:
    """Explicit token request only, no redirects; never prints values."""
    encoded=urlencode(payload).encode("utf-8")
    req=Request(BASE+"/token",data=encoded,method="POST",headers={
        "Content-Type":"application/x-www-form-urlencoded",
        "Accept":"application/json"
    })
    try:
        with build_opener(DenyRedirects).open(req,timeout=20) as response:
            if response.status!=200:
                raise WorkBuddyOfficialError("OAuth provider rejected token exchange")
            body=response.read(MAX_TOKEN_BODY+1)
    except WorkBuddyOfficialError:
        raise
    except (HTTPError,URLError,TimeoutError):
        raise WorkBuddyOfficialError("OAuth provider token request failed") from None
    if len(body)>MAX_TOKEN_BODY:
        raise WorkBuddyOfficialError("OAuth token payload exceeds allowed size")
    try: value=json.loads(body)
    except (ValueError,UnicodeDecodeError):
        raise WorkBuddyOfficialError("OAuth provider response is not JSON") from None
    if not isinstance(value,dict):
        raise WorkBuddyOfficialError("OAuth provider returned unexpected value")
    return value

TokenTransport=Callable[[dict],dict]

class OAuthClient:
    def __init__(self,client_id: str, client_secret: str, transport: TokenTransport=official_token_transport):
        if not client_id or not client_secret:
            raise WorkBuddyOfficialError("Registered app ID and server-side secret are required")
        self._id=client_id
        self._secret=client_secret
        self._transport=transport

    def exchange(self,authorization_code: str,redirect_uri: str) -> OAuthTokens:
        if not authorization_code or len(authorization_code)>2048:
            raise WorkBuddyOfficialError("Invalid one-time authorization code")
        uri=_https_redirect_uri(redirect_uri)
        body=self._transport({
            "grant_type":"authorization_code",
            "code":authorization_code,"redirect_uri":uri,
            "client_id":self._id,"client_secret":self._secret
        })
        return OAuthTokens.parse(body)

    def refresh(self,refresh_token: str) -> OAuthTokens:
        if not refresh_token:
            raise WorkBuddyOfficialError("Missing OAuth refresh token")
        body=self._transport({
            "grant_type":"refresh_token",
            "refresh_token":refresh_token,
            "client_id":self._id,
            "client_secret":self._secret
        })
        return OAuthTokens.parse(body,prior_refresh_token=refresh_token)
