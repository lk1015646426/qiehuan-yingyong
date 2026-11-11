"""WorkBuddy 0.1.22 headless-readiness audit: static source only, no credentials.

Run:
  python workbuddy-headless/audit.py --root . --json
  python workbuddy-headless/audit.py --root . --require-cloud-ready

This tool does not read ~/.workbuddy, LOCALAPPDATA, login snapshots, or
other account files. It only reads four known repository .rs source files.
It intentionally cannot certify business/API authorization.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import re
import sys
from typing import Sequence

POOL = "src-tauri/src/modules/workbuddy_pool.rs"
ACCOUNT = "src-tauri/src/modules/workbuddy_account.rs"
CRYPTO = "src-tauri/src/modules/workbuddy_auth_crypto.rs"
STORE = "src-tauri/src/modules/secure_account_storage.rs"
SOURCE_FILES = (POOL, ACCOUNT, CRYPTO, STORE)


@dataclass(frozen=True)
class Rule:
    code: str
    filename: str
    pattern: str
    category: str
    description: str
    remedy: str


# Precise known-source anchors, not a general-purpose secret scanner.
RULES = (
    Rule("WB_PRIVATE_CHAT_ENDPOINT", POOL,
         r'const\s+UPSTREAM\s*:\s*&str\s*=\s*"https://copilot\.tencent\.com/v2/chat/completions"',
         "authorization",
         "Chat proxy uses a desktop-service endpoint, not an evidenced server-side Open API.",
         "Obtain permitted provider API access before any server deployment."),
    Rule("WB_TAURI_COMMAND", POOL, r"#\[tauri::command\]",
         "desktop",
         "Account pool lifecycle is implemented as Tauri desktop commands.",
         "Extract HTTP lifecycle and state into a standalone service crate."),
    Rule("WB_LOOPBACK_LISTENER", POOL,
         r'Server::http\(\("127\.0\.0\.1",\s*PORT\)\)',
         "desktop",
         "Service only binds desktop loopback.",
         "Keep loopback while testing; use a separately secured HTTPS gateway if authorized."),
    Rule("WB_LOCAL_CLIENT_MODELS", POOL,
         r'\.join\("\.workbuddy/models\.json"\)',
         "desktop",
         "Model registration mutates the desktop WorkBuddy configuration.",
         "Remove client-model-file mutation from the future headless service."),
    Rule("WB_SINGLE_REQUEST_WORKER", POOL,
         r'for\s+request\s+in\s+worker\.server\.incoming_requests\(\)',
         "operations",
         "One service worker handles the entire incoming-request loop.",
         "Use bounded concurrent request handling before production."),
    Rule("WB_DESKTOP_AUTH_PATH", ACCOUNT,
         r'join\("workbuddy-desktop\.info"\)',
         "credentials",
         "Account import defaults to a desktop auth snapshot path.",
         "Use only provider-issued, explicitly authorized server credentials."),
    Rule("WB_WINDOWS_LOCALAPPDATA", ACCOUNT,
         r'var_os\("LOCALAPPDATA"\)',
         "desktop",
         "Default auth discovery assumes Windows LOCALAPPDATA.",
         "Replace with a platform-neutral configuration and official authorization flow."),
    Rule("WB_PRIVATE_REFRESH_ENDPOINT", ACCOUNT,
         r'/v2/plugin/auth/token/refresh',
         "authorization",
         "Credential refresh targets an internal plugin endpoint.",
         "Do not use internal desktop refresh as a public server authorization mechanism."),
    Rule("WB_PRIVATE_ACCOUNT_ENDPOINT", ACCOUNT,
         r'/v2/plugin/account',
         "authorization",
         "Credential verification uses an internal plugin account endpoint.",
         "Use a supported officially documented account or model API."),
    Rule("WB_TAURI_REFRESH_RUNTIME", ACCOUNT,
         r'tauri::async_runtime::spawn_blocking',
         "desktop",
         "Credential refresh is wired into the Tauri runtime.",
         "Extract runtime scheduling or remove it from the provider adapter."),
    Rule("WB_ELECTRON_LINKED_BINDING", CRYPTO,
         r'process\._linkedBinding\(',
         "credentials",
         "Credential decoding depends on a desktop Electron native binding.",
         "Avoid migrating private key material; use a supported provider API."),
    Rule("WB_ELECTRON_NODE_HELPER", CRYPTO,
         r'ELECTRON_RUN_AS_NODE',
         "desktop",
         "Secret key helper launches a WorkBuddy Electron executable.",
         "Do not require a GUI client executable in a headless service."),
    Rule("WB_LOCAL_STORAGE_KEY", STORE,
         r'const\s+KEY_FILE\s*:\s*&str\s*=\s*"secure-account-storage\.key"',
         "credentials",
         "Encrypted account records rely on a separate local symmetric key file.",
         "Do not copy account details or key files to a new host."),
)


def _source_line(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def inspect_sources(root: Path) -> dict:
    """Read ONLY known source files; report rule IDs and line numbers, never excerpts."""
    root = root.resolve()
    missing = [str(f) for f in SOURCE_FILES if not (root / f).is_file()]
    if missing:
        return {
            "schema_version": 1,
            "project": "workbuddy-original-pool",
            "decision": "insufficient_source",
            "cloud_deploy_allowed": False,
            "missing_files": missing,
            "observations": [],
            "unverified_requirements": ["server_api_authorization", "real_linux_e2e"],
        }
    observations = []
    for rule in RULES:
        contents = (root / rule.filename).read_text(encoding="utf-8")
        match = re.search(rule.pattern, contents)
        observations.append({
            "code": rule.code,
            "category": rule.category,
            "status": "present" if match else "manual_review_required",
            "file": rule.filename,
            "line": _source_line(contents, match.start()) if match else None,
            "finding": rule.description,
            "next_action": rule.remedy,
        })
    # There is no reliable machine-verifiable proof that a consumer app session
    # is licensed to act as a cloud model provider. Never auto-approve.
    return {
        "schema_version": 1,
        "project": "workbuddy-original-pool",
        "baseline": "v0.1.22",
        "decision": "blocked_authorization_and_headless_dependencies",
        "cloud_deploy_allowed": False,
        "server_api_authorization": "unverified",
        "real_linux_e2e": "not_performed",
        "private_endpoint_usage": "observed" if any(
            v["code"] == "WB_PRIVATE_CHAT_ENDPOINT" and v["status"] == "present"
            for v in observations
        ) else "manual_review_required",
        "observations": observations,
        "unverified_requirements": [
            "provider_server_api_permission",
            "provider_supported_authentication",
            "no_desktop_runtime",
            "production_security",
            "china_network_and_zero_budget_hosting",
            "android_real_device_test",
        ],
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--json", action="store_true", help="Print only safe structured evidence")
    parser.add_argument("--require-cloud-ready", action="store_true",
                        help="Fail closed when cloud deploy is not authorized/verified")
    args = parser.parse_args(argv)
    report = inspect_sources(args.root)
    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
    else:
        print("WorkBuddy original pool audit — cloud deployment:", "BLOCKED")
        for v in report["observations"]:
            location = f"{v['file']}:{v['line']}" if v["line"] else v["file"]
            print(f"  {v['code']}: {v['status']} ({location})")
        print("  Independent, permitted model API authorization: UNVERIFIED")
        print("  This report reads repository source only; no real credentials.")
    if args.require_cloud_ready or report["decision"] == "insufficient_source":
        return 0 if report["cloud_deploy_allowed"] else 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
