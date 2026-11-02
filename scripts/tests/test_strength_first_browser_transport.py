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


if __name__ == "__main__":
    unittest.main()
