#!/usr/bin/env python3
"""Drive the ExhiOS user-space serial shell over QEMU's Unix serial socket."""

from __future__ import annotations

import socket
import sys
import time
from pathlib import Path


def connect(path: str, timeout: float = 15.0) -> socket.socket:
    deadline = time.monotonic() + timeout
    last_error: OSError | None = None
    while time.monotonic() < deadline:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(0.5)
        try:
            client.connect(path)
            return client
        except OSError as error:
            last_error = error
            client.close()
            time.sleep(0.1)
    raise RuntimeError(f"could not connect to QEMU serial socket {path}: {last_error}")


def read_until(client: socket.socket, buffer: bytearray, marker: bytes, timeout: float = 8.0) -> bytes:
    deadline = time.monotonic() + timeout
    while True:
        position = buffer.find(marker)
        if position >= 0:
            end = position + len(marker)
            received = bytes(buffer[:end])
            del buffer[:end]
            return received
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"timed out waiting for {marker!r}; buffered={bytes(buffer[-1000:])!r}")
        client.settimeout(min(0.5, remaining))
        try:
            chunk = client.recv(4096)
        except socket.timeout:
            continue
        if not chunk:
            raise ConnectionError(f"QEMU closed serial socket before {marker!r}; buffered={bytes(buffer[-1000:])!r}")
        buffer.extend(chunk)


def issue_input(
    client: socket.socket,
    buffer: bytearray,
    input_bytes: bytes,
    expected: tuple[bytes, ...],
) -> None:
    client.sendall(input_bytes)
    received = bytearray()
    for marker in expected:
        received.extend(read_until(client, buffer, marker))
    received.extend(read_until(client, buffer, b"exhios> "))
    for marker in expected:
        if marker not in received:
            raise AssertionError(
                f"input {input_bytes!r} did not produce {marker!r}; output={bytes(received)!r}"
            )


def issue(
    client: socket.socket,
    buffer: bytearray,
    command: bytes,
    expected: tuple[bytes, ...],
) -> None:
    issue_input(client, buffer, command + b"\r", expected)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: qemu-send-shell-smoke.py SOCKET", file=sys.stderr)
        return 2

    try:
        with connect(sys.argv[1]) as client:
            buffer = bytearray()
            read_until(client, buffer, b"exhios> ")
            issue(
                client,
                buffer,
                b"help",
                (b"commands: help hostname version echo clear config spawn exit\r\n",),
            )
            issue(client, buffer, b"hostname", (b"exhios\r\n",))
            issue(client, buffer, b"version", (b"ExhiOS userland 0.1.0-alpha.1\r\n",))
            issue(client, buffer, b"echo ci-pass", (b"ci-pass\r\n",))
            issue_input(
                client,
                buffer,
                b"echo ci-pasx\x7fs\r",
                (b"ci-pass\r\n",),
            )
            issue(client, buffer, b"clear", (b"\x1B[2J\x1B[H",))
            issue(client, buffer, b"missing", (b"unknown command: missing\r\n",))
            client.sendall(b"partial\x03")
            read_until(client, buffer, b"^C\r\n")
            read_until(client, buffer, b"exhios> ")
            issue(
                client,
                buffer,
                b"config",
                (
                    b"hostname=exhios\r\n",
                    b"prompt=exhios>\r\n",
                    b"version=0.1.0-alpha.1\r\n",
                ),
            )
            client.sendall(b"spawn\r")
            read_until(client, buffer, b"worker process started.\r\n")
            read_until(client, buffer, b"exhios> ")
            worker_output = bytearray(
                read_until(
                    client,
                    buffer,
                    b"ExhiOS worker: second Ring 3 process started.\r\n",
                )
            )
            worker_output.extend(
                read_until(
                    client,
                    buffer,
                    b"ExhiOS worker: TaskSpawn denied as expected.\r\n",
                )
            )
            worker_output.extend(
                read_until(
                    client,
                    buffer,
                    b"ExhiOS worker: completed yield sequence; exiting.\r\n",
                    timeout=8.0,
                )
            )
            print(worker_output.decode("ascii", errors="replace"), end="")
            # The v1 runtime retains a Finished worker until init exits, so a
            # second spawn must fail without allocating another process slot.
            client.sendall(b"spawn\r")
            read_until(client, buffer, b"worker process could not be started.\r\n")
            read_until(client, buffer, b"exhios> ")
            client.sendall(b"exit\r")
            output = read_until(
                client,
                buffer,
                b"P7.3: interactive serial shell smoke test passed.\r\n",
                timeout=8.0,
            )
            print(output.decode("ascii", errors="replace"), end="")
        print("Serial shell command sequence verified.")
        return 0
    except (AssertionError, ConnectionError, OSError, RuntimeError, TimeoutError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
