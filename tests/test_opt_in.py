"""Black-box opt-in and heartbeat tests; run after `cargo build`.

Uses only the Python standard library and a loopback mock ActivityWatch API.
Set AW_NOTIFY_BIN to test a binary outside target/debug/aw-notify.
"""

import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import threading
import time
import unittest
from datetime import datetime
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit


ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("AW_NOTIFY_BIN", ROOT / "target/debug/aw-notify"))
ALERT_TYPES = {
    "threshold",
    "checkin",
    "productivity_score",
    "new_day",
    "server_status",
    "external",
}
QUIET_CONFIG = {
    "alerts": [],
    "hourly_checkins": False,
    "new_day_greetings": False,
    "productivity_score": False,
    "server_monitoring": False,
    "http_port": 0,
}


def unused_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def probe_listener(port, stop, observed):
    while not stop.is_set():
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.05):
                observed.append(True)
                return
        except OSError:
            stop.wait(0.002)


class MockActivityWatch(ThreadingHTTPServer):
    def __init__(self, settings):
        self.settings = settings
        self.requests = []
        self.changed = threading.Condition()
        self.release_heartbeat = None
        super().__init__(("127.0.0.1", 0), MockHandler)

    def matching(self, suffix):
        with self.changed:
            return [r for r in self.requests if urlsplit(r[1]).path.endswith(suffix)]

    def wait_for_heartbeats(self, count, timeout=30):
        with self.changed:
            return self.changed.wait_for(
                lambda: len(self.matching("/heartbeat")) >= count, timeout
            )


class MockHandler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        self.respond()

    def do_POST(self):
        self.respond()

    def respond(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length)) if length else None
        path = urlsplit(self.path).path
        with self.server.changed:
            self.server.requests.append(
                (self.command, self.path, body, time.monotonic())
            )
            self.server.changed.notify_all()
        if path == "/api/0/info":
            response = {
                "hostname": socket.gethostname(),
                "version": "test",
                "testing": True,
                "device_id": "test",
            }
        elif path == "/api/0/settings/aw-notify":
            response = self.server.settings
        elif path == "/api/0/query":
            response = [
                {
                    "duration": 3600,
                    "cat_events": [{"duration": 3600, "data": {"$category": ["Work"]}}],
                }
            ]
        elif path.startswith("/api/0/settings/"):
            response = None
        elif self.command == "POST" and path.startswith("/api/0/buckets/"):
            response = body
        else:
            self.send_error(404)
            return
        if path.endswith("/heartbeat") and self.server.release_heartbeat is not None:
            self.server.release_heartbeat.wait(timeout=10)
        encoded = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        try:
            self.wfile.write(encoded)
        except BrokenPipeError:
            # The stall test deliberately exits before this response is released.
            pass


class OptInTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(BINARY.is_file(), f"Build aw-notify first: {BINARY}")
        self.temp = tempfile.TemporaryDirectory(prefix="aw-notify-opt-in-")
        self.addCleanup(self.temp.cleanup)
        self.config = Path(self.temp.name) / "config.toml"

    def server(self, settings):
        server = MockActivityWatch(settings)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(thread.join)
        self.addCleanup(server.shutdown)
        return server

    def launch(self, server, command, local_enabled=None, http_port=0):
        lines = [
            "alerts = []",
            "hourly_checkins = false",
            "new_day_greetings = false",
            "productivity_score = false",
            "server_monitoring = false",
            f"http_port = {http_port}",
        ]
        if local_enabled is not None:
            lines.append(f"enabled = {str(local_enabled).lower()}")
        self.config.write_text("\n".join(lines) + "\n")
        env = dict(os.environ, XDG_CACHE_HOME=self.temp.name)
        process = subprocess.Popen(
            [
                str(BINARY),
                "--config",
                str(self.config),
                "--output-only",
                "--port",
                str(server.server_port),
                command,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
        self.addCleanup(self.stop, process)
        return process

    @staticmethod
    def stop(process):
        if process.poll() is None:
            process.kill()
        process.communicate(timeout=3)

    def assert_disabled(self, settings, local_enabled=None):
        for command in ("start", "checkin", "checkin-detailed"):
            with self.subTest(command=command):
                server = self.server(settings)
                port = unused_port()
                if settings is not None:
                    server.settings = {**settings, "http_port": port}
                observed = []
                stop = threading.Event()
                observer = threading.Thread(
                    target=probe_listener, args=(port, stop, observed), daemon=True
                )
                observer.start()
                process = self.launch(server, command, local_enabled, http_port=port)
                try:
                    try:
                        stdout, stderr = process.communicate(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        stdout, stderr = process.communicate()
                        self.fail(f"Disabled {command} stayed running: {stderr}")
                finally:
                    stop.set()
                    observer.join(timeout=1)
                self.assertEqual(process.returncode, 0, stderr)
                self.assertEqual(stdout, "", stderr)
                self.assertFalse(observed, "Disabled command opened an HTTP listener")
                with socket.socket() as connection:
                    self.assertNotEqual(connection.connect_ex(("127.0.0.1", port)), 0)
                paths = [urlsplit(r[1]).path for r in server.requests]
                self.assertIn("/api/0/settings/aw-notify", paths)
                self.assertTrue(
                    all(
                        p in ("/api/0/info", "/api/0/settings/aw-notify") for p in paths
                    ),
                    f"Disabled {command} performed work: {paths}",
                )

    def test_missing_setting_defaults_off(self):
        self.assert_disabled(None)

    def test_explicit_false_is_off(self):
        self.assert_disabled({"enabled": False, **QUIET_CONFIG})

    def test_server_false_overrides_local_true(self):
        self.assert_disabled({"enabled": False, **QUIET_CONFIG}, local_enabled=True)

    def test_missing_server_enabled_does_not_inherit_local_true(self):
        self.assert_disabled(QUIET_CONFIG, local_enabled=True)

    def test_enabled_one_shot_commands_deliver(self):
        for command in ("checkin", "checkin-detailed"):
            with self.subTest(command=command):
                server = self.server({"enabled": True, **QUIET_CONFIG})
                process = self.launch(server, command, local_enabled=False)
                stdout, stderr = process.communicate(timeout=3)
                self.assertEqual(process.returncode, 0, stderr)
                self.assertIn("Work", json.loads(stdout)["message"])
                self.assertEqual(len(server.matching("/query")), 1)
                self.assertEqual(server.matching("/heartbeat"), [])

    def test_enabled_daemon_heartbeats_and_stops_on_sigint(self):
        server = self.server({"enabled": True, **QUIET_CONFIG})
        port = unused_port()
        server.settings["http_port"] = port
        process = self.launch(server, "start", local_enabled=False)
        self.assertTrue(
            server.wait_for_heartbeats(1, timeout=3), "No initial heartbeat"
        )
        deadline = time.monotonic() + 3
        while True:
            connection = HTTPConnection("127.0.0.1", port, timeout=1)
            try:
                connection.request(
                    "POST",
                    "/notify",
                    body=json.dumps(
                        {
                            "title": "External test",
                            "message": "Hello",
                            "sender": "test-watcher",
                        }
                    ),
                    headers={"Content-Type": "application/json"},
                )
                response = connection.getresponse()
                self.assertEqual(response.status, 200, response.read())
                break
            except ConnectionRefusedError:
                if time.monotonic() >= deadline:
                    self.fail("Enabled HTTP listener did not start")
                time.sleep(0.01)
            finally:
                connection.close()
        # One-shot checkins have their own client lock and must coexist with the daemon.
        checkin = self.launch(server, "checkin", local_enabled=False)
        checkin_stdout, checkin_stderr = checkin.communicate(timeout=3)
        self.assertEqual(checkin.returncode, 0, checkin_stderr)
        self.assertIn("Work", json.loads(checkin_stdout)["message"])
        self.assertTrue(
            server.wait_for_heartbeats(2), "No two daemon heartbeats within 30s"
        )
        process.send_signal(signal.SIGINT)
        stdout, stderr = process.communicate(timeout=3)
        self.assertEqual(process.returncode, 0, stderr)
        notifications = [json.loads(line) for line in stdout.splitlines()]
        self.assertEqual(
            {n["title"] for n in notifications},
            {"Time today", "Time yesterday", "External test"},
        )
        external = next(n for n in notifications if n["title"] == "External test")
        self.assertEqual(external["sender"], "test-watcher")

        bucket_path = f"/api/0/buckets/aw-notify_{socket.gethostname()}"
        creates = [r for r in server.requests if urlsplit(r[1]).path == bucket_path]
        heartbeats = server.matching("/heartbeat")
        self.assertGreaterEqual(len(creates), 1)
        self.assertGreaterEqual(len(creates), len(heartbeats))
        for creation, heartbeat in zip(creates, heartbeats):
            self.assertEqual(creation[0], "POST")
            self.assertEqual(creation[2]["type"], "app.aw-notify.status")
            self.assertLess(creation[3], heartbeat[3])
        self.assertGreaterEqual(heartbeats[1][3] - heartbeats[0][3], 4)
        sessions = set()
        for _, path, event, _ in heartbeats:
            self.assertEqual(urlsplit(path).path, bucket_path + "/heartbeat")
            self.assertEqual(float(parse_qs(urlsplit(path).query)["pulsetime"][0]), 10)
            data = event["data"]
            self.assertEqual(data["schema_version"], 1)
            self.assertIs(data["enabled"], True)
            self.assertIs(data["output_only"], True)
            self.assertIsNotNone(
                datetime.fromisoformat(
                    data["session_started"].replace("Z", "+00:00")
                ).tzinfo
            )
            sessions.add(data["session_started"])
            self.assertEqual(set(data["counts"]), ALERT_TYPES)
            for count in data["counts"].values():
                self.assertEqual(count["shown"], 0)
                self.assertIsNone(count["dismissed"])
        self.assertEqual(len(sessions), 1)
        for kind, count in heartbeats[-1][2]["data"]["counts"].items():
            expected = {"checkin": 2, "external": 1}.get(kind, 0)
            self.assertEqual(count["forwarded"], expected, kind)

    def test_sigint_does_not_wait_for_stalled_heartbeat_response(self):
        server = self.server({"enabled": True, **QUIET_CONFIG})
        server.release_heartbeat = threading.Event()
        self.addCleanup(server.release_heartbeat.set)
        process = self.launch(server, "start")
        self.assertTrue(
            server.wait_for_heartbeats(1, timeout=3), "No initial heartbeat"
        )
        # The startup query follows signal-handler installation in the main thread.
        with server.changed:
            self.assertTrue(
                server.changed.wait_for(lambda: server.matching("/query"), 3)
            )
        process.send_signal(signal.SIGINT)
        try:
            _, stderr = process.communicate(timeout=3)
            self.assertEqual(process.returncode, 0, stderr)
        finally:
            server.release_heartbeat.set()


if __name__ == "__main__":
    unittest.main()
