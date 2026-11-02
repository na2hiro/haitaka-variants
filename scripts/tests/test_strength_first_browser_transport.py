"""Browser completion must only publish reports that are already persisted."""

from contextlib import contextmanager
import http.client
import http.server
import json
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import strength_first_step14_browser as transport


class BrowserResultTests(unittest.TestCase):
    @contextmanager
    def server(self, output):
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), transport.BrowserHandler)
        server.browser_state = transport.BrowserState()
        server.browser_state.ack = {"verified": True}
        server.browser_config = {"expectedRawGames": 0}
        server.browser_output_dir = output
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

    def post_result(self, server, responses):
        connection = http.client.HTTPConnection(*server.server_address, timeout=5)
        try:
            connection.request("POST", "/result", json.dumps({"status": "passed"}),
                               {"Content-Type": "application/json"})
            response = connection.getresponse()
            responses.append((response.status, response.read()))
        finally:
            connection.close()

    def test_slow_write_does_not_publish_completion_before_report_exists(self):
        entered, release = threading.Event(), threading.Event()
        original_write = transport.atomic_json
        responses = []
        with tempfile.TemporaryDirectory() as directory, self.server(Path(directory)) as server:
            report = Path(directory) / "report.json"

            def delayed_write(path, value):
                entered.set()
                if not release.wait(5):
                    raise RuntimeError("test did not release report write")
                original_write(path, value)

            with mock.patch.object(transport, "atomic_json", side_effect=delayed_write):
                client = threading.Thread(target=self.post_result, args=(server, responses))
                client.start()
                try:
                    self.assertTrue(entered.wait(5))
                    self.assertFalse(report.exists())
                    self.assertFalse(server.browser_state.event.is_set())
                    self.assertIsNone(server.browser_state.result)
                finally:
                    release.set()
                    client.join(timeout=5)
                self.assertFalse(client.is_alive())
            self.assertEqual(responses, [(200, b"result-durable\n")])
            self.assertTrue(server.browser_state.event.is_set())
            self.assertEqual(json.loads(report.read_text()), server.browser_state.result)

    def test_failed_write_reports_failure_without_publishing_success(self):
        with tempfile.TemporaryDirectory() as directory, self.server(Path(directory)) as server:
            responses = []
            with mock.patch.object(transport, "atomic_json", side_effect=OSError("disk full")):
                self.post_result(server, responses)
            self.assertEqual(responses, [(500, b"result-write-failed\n")])
            self.assertTrue(server.browser_state.event.is_set())
            self.assertEqual(server.browser_state.error, {"category": "result-write-failed"})
            self.assertIsNone(server.browser_state.result)
            self.assertFalse((Path(directory) / "report.json").exists())


class Step14PackageTests(unittest.TestCase):
    def test_refresh_packages_match_worker_and_updates_its_bindings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, value in (("manifest.json", {"experimentId": transport.EXPERIMENT_ID}),
                                ("execution-bindings.json", {"host": {"fixture": True}}),
                                ("source-identity.json", {"sourceSnapshot": {}})):
                (root / name).write_text(json.dumps(value))
            result = transport.prepare(root)
            self.assertEqual(result["status"], "existing")
            self.assert_worker_bound(root)
            bindings = json.loads((root / "execution-bindings.json").read_text())
            self.assertEqual(bindings["host"], {"fixture": True})

    def assert_worker_bound(self, root):
        name = "strength_first_step14_browser_worker.js"
        worker = root / "package" / name
        self.assertEqual(worker.read_bytes(), (transport.repo_root() / "scripts" / name).read_bytes())
        binding = transport.artifact(worker, root)
        bindings = json.loads((root / "execution-bindings.json").read_text())
        identity = json.loads((root / "source-identity.json").read_text())
        self.assertEqual(bindings["package"][name], binding)
        self.assertEqual(identity["sourceSnapshot"]["relevantFiles"][name], binding)
        manifest = json.loads((root / "manifest.json").read_text())
        self.assertEqual(manifest["sourceIdentity"], transport.artifact(root / "source-identity.json", root))
        self.assertEqual(manifest["executionBindings"], transport.artifact(root / "execution-bindings.json", root))

    def test_fresh_preparation_packages_match_worker_with_frozen_inputs(self):
        import hashlib
        import shutil

        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory) / "repo"
            (repo / "scripts").mkdir(parents=True)
            for name in ("strength_first_step14_browser.py", "strength_first_step14_contract.py",
                         "strength_first_step14_browser_worker.js", "strength_first_step14_browser_page.html",
                         "strength_first_step14_terminal_worker.js"):
                shutil.copy2(transport.repo_root() / "scripts" / name, repo / "scripts" / name)
            rows = [{"cell": "early-selected", "leg": 0, "pairIndex": index,
                     "openingId": f"step12-development-{index + 1:03d}",
                     "startSfen": f"fixture-{index} {'b' if index % 2 == 0 else 'w'} - 1",
                     "sideToMove": "black" if index % 2 == 0 else "white", "sourceGameId": f"source-{index}"}
                    for index in range(64)]
            anchors = {}
            for name in {**transport.EXPECTED, **transport.EXTRA_INPUTS}:
                path = repo / "inputs" / f"{name}.json"
                path.parent.mkdir(exist_ok=True)
                path.write_text(json.dumps({"rows": rows} if name == "s12-schedule" else {}))
                anchors[name] = (str(path.relative_to(repo)), transport.sha256(path), path.stat().st_size)
            tsv = "".join(f"{row['openingId']}\t{row['startSfen']}\n" for row in rows).encode()
            root = Path(directory) / "package"
            with mock.patch.object(transport, "repo_root", return_value=repo), \
                 mock.patch.object(transport, "EXPECTED", {name: anchors[name] for name in transport.EXPECTED}), \
                 mock.patch.object(transport, "EXTRA_INPUTS", {name: anchors[name] for name in transport.EXTRA_INPUTS}), \
                 mock.patch.object(transport, "TSV_BYTES", len(tsv)), \
                 mock.patch.object(transport, "TSV_HASH", hashlib.sha256(tsv).hexdigest()), \
                 mock.patch.object(transport, "run_capture", return_value=(0, "fixture-head", "")):
                result = transport.prepare(root)
                self.assertEqual(result["status"], "passed")
                self.assertEqual(result["browserLaunches"], 0)
                self.assert_worker_bound(root)


if __name__ == "__main__":
    unittest.main()
