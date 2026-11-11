"""Unit tests for source-only headless readiness evidence and fail-closed gate."""
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "workbuddy-headless" / "audit.py"
spec = importlib.util.spec_from_file_location("workbuddy_headless_audit", SCRIPT)
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)


class AuditTests(unittest.TestCase):
    def test_actual_baseline_sources_and_key_risks(self):
        report = module.inspect_sources(ROOT)
        self.assertEqual(report["baseline"], "v0.1.22")
        self.assertFalse(report["cloud_deploy_allowed"])
        by_code = {item["code"]: item for item in report["observations"]}
        for code in (
            "WB_PRIVATE_CHAT_ENDPOINT", "WB_TAURI_COMMAND",
            "WB_ELECTRON_LINKED_BINDING", "WB_PRIVATE_REFRESH_ENDPOINT",
            "WB_DESKTOP_AUTH_PATH", "WB_LOOPBACK_LISTENER",
            "WB_SINGLE_REQUEST_WORKER", "WB_LOCAL_STORAGE_KEY",
        ):
            with self.subTest(code=code):
                self.assertEqual(by_code[code]["status"], "present")
                self.assertGreater(by_code[code]["line"], 0)
        self.assertEqual(report["server_api_authorization"], "unverified")
        self.assertEqual(report["real_linux_e2e"], "not_performed")

    def test_json_is_safe_and_contains_no_source_excerpts(self):
        report = module.inspect_sources(ROOT)
        raw = json.dumps(report)
        self.assertNotIn("ELECTRON_RUN_AS_NODE", raw)
        self.assertNotIn("X-User-Id", raw)
        self.assertNotIn("process._linkedBinding", raw)
        self.assertNotIn("Bearer ", raw)
        self.assertNotIn("secure-account-storage.key", raw)

    def test_no_runtime_or_credentials_discovery(self):
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertNotIn("LOCALAPPDATA", source)
        self.assertNotIn("os.environ.get", source)
        self.assertNotIn("subprocess.run", source)
        self.assertNotIn("requests.get", source)
        self.assertNotIn("urllib.request", source)

    def test_missing_sources_fail_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            report = module.inspect_sources(Path(temp))
        self.assertEqual(report["decision"], "insufficient_source")
        self.assertFalse(report["cloud_deploy_allowed"])
        self.assertGreaterEqual(len(report["missing_files"]), 4)

    def test_if_sensitive_dependency_is_removed_it_demands_review(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for source in module.SOURCE_FILES:
                file = root / source
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("/* safe test fixture */\n", encoding="utf-8")
            report = module.inspect_sources(root)
        self.assertFalse(report["cloud_deploy_allowed"])
        self.assertTrue(all(v["status"] == "manual_review_required" for v in report["observations"]))

    def test_cli_machine_readable_and_exit_codes(self):
        proc = subprocess.run([sys.executable, str(SCRIPT), "--root", str(ROOT), "--json"],
                              capture_output=True, text=True, timeout=12)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        report = json.loads(proc.stdout)
        self.assertFalse(report["cloud_deploy_allowed"])
        gate = subprocess.run([sys.executable, str(SCRIPT), "--root", str(ROOT),
                               "--require-cloud-ready"], capture_output=True, text=True, timeout=12)
        self.assertEqual(gate.returncode, 3)
        self.assertIn("BLOCKED", gate.stdout)

    def test_line_numbers_stay_in_declared_file(self):
        report = module.inspect_sources(ROOT)
        for item in report["observations"]:
            if item["line"] is None:
                continue
            num_lines = len((ROOT / item["file"]).read_text(encoding="utf-8").splitlines())
            self.assertTrue(1 <= item["line"] <= num_lines, item["code"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
