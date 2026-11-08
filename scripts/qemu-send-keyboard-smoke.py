#!/usr/bin/env python3
"""Inject deterministic PS/2 key transitions through QEMU's QMP interface."""

from __future__ import annotations

import json
import socket
import sys
import time
from pathlib import Path
from typing import Any


def read_message(reader: Any) -> dict[str, Any]:
    line = reader.readline()
    if not line:
        raise RuntimeError("QMP connection closed unexpectedly")
    message = json.loads(line)
    if not isinstance(message, dict):
        raise RuntimeError("QMP returned a non-object message")
    return message


def command(
    sock: socket.socket,
    reader: Any,
    command_id: int,
    name: str,
    arguments: dict[str, Any] | None = None,
) -> dict[str, Any]:
    request: dict[str, Any] = {"execute": name, "id": command_id}
    if arguments is not None:
        request["arguments"] = arguments
    sock.sendall((json.dumps(request) + "\r\n").encode("utf-8"))
    while True:
        response = read_message(reader)
        if response.get("id") != command_id:
            continue
        if "error" in response:
            raise RuntimeError(f"QMP command {name} failed: {response['error']}")
        return response


def connect_qmp(path: Path, timeout_seconds: float = 20.0) -> socket.socket:
    deadline = time.monotonic() + timeout_seconds
    last_error: OSError | None = None
    while time.monotonic() < deadline:
        candidate = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        candidate.settimeout(10.0)
        try:
            candidate.connect(str(path))
            return candidate
        except OSError as error:
            last_error = error
            candidate.close()
            time.sleep(0.1)
    raise TimeoutError(f"QMP socket did not become ready: {last_error}")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: qemu-send-keyboard-smoke.py QMP_SOCKET", file=sys.stderr)
        return 2

    socket_path = Path(sys.argv[1])
    try:
        with connect_qmp(socket_path) as qmp:
            reader = qmp.makefile("r", encoding="utf-8", newline="\n")
            greeting = read_message(reader)
            if "QMP" not in greeting:
                raise RuntimeError("QMP greeting missing")
            command(qmp, reader, 1, "qmp_capabilities")

            # Allow UEFI boot and the kernel's i8042 initialization to finish.
            time.sleep(5.0)
            command(
                qmp,
                reader,
                2,
                "human-monitor-command",
                {"command-line": "sendkey a"},
            )
            time.sleep(0.2)
            command(
                qmp,
                reader,
                3,
                "human-monitor-command",
                {"command-line": "sendkey shift-b"},
            )
            time.sleep(0.2)
            command(
                qmp,
                reader,
                4,
                "human-monitor-command",
                {"command-line": "sendkey ctrl-d"},
            )
        print("QMP keyboard smoke input injected: A, Shift+B, Ctrl+D.")
        return 0
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        print(f"QMP keyboard injection failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
