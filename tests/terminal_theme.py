"""Exercise live terminal colors and interleaved input in the packaged Codex TUI."""

import fcntl
import http.server
import json
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
from pathlib import Path

import pyte


class Gateway(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        events = [
            {"type": "response.created", "response": {"id": "theme-response"}},
            {
                "type": "response.output_item.done",
                "item": {
                    "type": "message",
                    "role": "assistant",
                    "id": "theme-message",
                    "content": [{"type": "output_text", "text": "theme-answer"}],
                },
            },
            {"type": "response.completed", "response": {"id": "theme-response"}},
        ]
        body = "".join(f"data: {json.dumps(event)}\n\n" for event in events).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class Terminal:
    def __init__(self, binary, home):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        self.screen = pyte.Screen(120, 32)
        self.parser = pyte.ByteStream(self.screen)
        self.output = bytearray()
        self.counts = {}
        self.colors = ("e0e0e0", "121212")
        self.answer_colors = True
        self.child = subprocess.Popen(
            [binary, "--no-alt-screen", "-C", str(home), "theme-history"],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env={
                "PATH": os.environ["PATH"],
                "HOME": str(home),
                "CODEX_HOME": str(home),
                "TERM": "xterm-256color",
                "COLORTERM": "truecolor",
            },
            start_new_session=True,
        )
        os.close(slave)

    def send(self, data):
        os.write(self.master, data)

    def color_reply(self, slot):
        color = self.colors[slot - 10]
        rgb = "/".join(color[i : i + 2] * 2 for i in (0, 2, 4))
        return f"\x1b]{slot};rgb:{rgb}\x1b\\".encode()

    def read(self):
        if select.select([self.master], [], [], 0.05)[0]:
            data = os.read(self.master, 65536)
            self.output.extend(data)
            self.parser.feed(data)
        replies = {
            b"\x1b[6n": b"\x1b[1;1R",
            b"\x1b[?u": b"\x1b[?0u",
            b"\x1b[c": b"\x1b[?1;2c",
            b"\x1b]10;?": self.color_reply(10) if self.answer_colors else b"",
            b"\x1b]11;?": self.color_reply(11) if self.answer_colors else b"",
        }
        for query, reply in replies.items():
            count = self.output.count(query)
            for _ in range(count - self.counts.get(query, 0)):
                if reply:
                    self.send(reply)
            self.counts[query] = count

    def wait(self, condition, description, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.read()
            if condition():
                return
            if self.child.poll() is not None:
                break
        raise AssertionError(f"{description}\n" + "\n".join(self.screen.display))

    def text_background(self, text):
        for row, line in enumerate(self.screen.display):
            column = line.find(text)
            if column >= 0:
                return self.screen.buffer[row][column].bg
        return None

    def close(self):
        if self.child.poll() is None:
            os.killpg(self.child.pid, signal.SIGTERM)
            try:
                self.child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.child.pid, signal.SIGKILL)
                self.child.wait(timeout=5)
        os.close(self.master)


def run(binary):
    with (
        http.server.ThreadingHTTPServer(("127.0.0.1", 0), Gateway) as gateway,
        tempfile.TemporaryDirectory(prefix="codex-theme-") as directory,
    ):
        threading.Thread(target=gateway.serve_forever, daemon=True).start()
        home = Path(directory).resolve()
        (home / "config.toml").write_text(
            'model = "gpt-6-astra"\n'
            'model_provider = "test"\n'
            "check_for_update_on_startup = false\n"
            "suppress_unstable_features_warning = true\n"
            "analytics.enabled = false\n"
            "[model_providers.test]\n"
            'name = "Test"\n'
            f'base_url = "http://127.0.0.1:{gateway.server_port}/v1"\n'
            'wire_api = "responses"\n'
            "requires_openai_auth = false\n"
            "supports_websockets = false\n"
            "[tui]\n"
            "animations = false\n"
            'theme = "ansi"\n'
            f'[projects."{home}"]\ntrust_level = "trusted"\n'
        )
        terminal = Terminal(binary, home)
        try:
            terminal.wait(
                lambda: b"\x1b[?2031h" in terminal.output
                and terminal.text_background("theme-answer") is not None,
                "The TUI did not start with color notifications enabled.",
                timeout=30,
            )
            assert terminal.output.rfind(b"\x1b[?2031h") > terminal.output.rfind(
                b"\x1b[?2031l"
            )
            terminal.send(b"theme-draft")
            terminal.wait(
                lambda: terminal.text_background("theme-draft") == "2e2e2e",
                "The composer did not use the initial dark palette.",
            )
            for colors, notification, background in [
                (("1a1a1a", "f5f5f5"), b"\x1b[?997;2n", "ebebeb"),
                (("e0e0e0", "121212"), b"\x1b[?997;1n", "2e2e2e"),
                (("1a1a1a", "f5f5f5"), b"\x1b[I", "ebebeb"),
            ]:
                terminal.colors = colors
                terminal.send(notification)
                terminal.wait(
                    lambda: terminal.text_background("theme-draft") == background
                    and terminal.text_background("theme-history") == background,
                    "The composer and transcript did not follow the terminal palette.",
                )

            terminal.answer_colors = False
            terminal.send(b"\x1b[?997;1n-input")
            terminal.wait(
                lambda: terminal.text_background("theme-draft-input") == "ebebeb",
                "An unanswered color query blocked or discarded input.",
            )
            terminal.colors = ("e0e0e0", "121212")
            terminal.send(
                terminal.color_reply(10) + b"-reply" + terminal.color_reply(11)
            )
            terminal.wait(
                lambda: terminal.text_background("theme-draft-input-reply") == "2e2e2e",
                "Interleaved color replies changed the draft or failed to repaint.",
            )
            disabled = terminal.output.count(b"\x1b[?2031l")
            terminal.send(b"\x03\x03")
            terminal.wait(
                lambda: terminal.output.count(b"\x1b[?2031l") > disabled,
                "The TUI did not disable color notifications on exit.",
            )
        finally:
            terminal.close()
            gateway.shutdown()


if __name__ == "__main__":
    run(sys.argv[1])
