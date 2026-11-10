#!/usr/bin/env python3
"""Verify that a faulting Ring 3 worker returns control to its live init parent."""

from __future__ import annotations

import socket
import sys
import time


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


def read_until(
    client: socket.socket,
    buffer: bytearray,
    marker: bytes,
    timeout: float = 8.0,
) -> bytes:
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
            raise TimeoutError(
                f"timed out waiting for {marker!r}; buffered={bytes(buffer[-1000:])!r}"
            )
        client.settimeout(min(0.5, remaining))
        try:
            chunk = client.recv(4096)
        except socket.timeout:
            continue
        if not chunk:
            raise ConnectionError(
                f"QEMU closed serial socket before {marker!r}; "
                f"buffered={bytes(buffer[-1000:])!r}"
            )
        buffer.extend(chunk)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: qemu-send-worker-fault-smoke.py SOCKET", file=sys.stderr)
        return 2

    try:
        with connect(sys.argv[1]) as client:
            buffer = bytearray()
            output = bytearray()

            output.extend(read_until(client, buffer, b"exhios> "))
            client.sendall(b"spawn\r")
            output.extend(read_until(client, buffer, b"worker process started.\r\n"))
            output.extend(read_until(client, buffer, b"exhios> "))
            output.extend(
                read_until(
                    client,
                    buffer,
                    b"ExhiOS worker: second Ring 3 process started.\r\n",
                )
            )
            output.extend(
                read_until(
                    client,
                    buffer,
                    b"ExhiOS worker: triggering invalid-opcode fault.\r\n",
                )
            )
            output.extend(
                read_until(
                    client,
                    buffer,
                    b"ExhiOS: terminating user process after exception; RIP=0x",
                )
            )
            output.extend(read_until(client, buffer, b" user RSP=0x"))
            output.extend(read_until(client, buffer, b".\r\n"))

            # The init shell must still be alive after the worker's CPL3 fault.
            client.sendall(b"hostname\r")
            output.extend(read_until(client, buffer, b"exhios\r\n"))
            output.extend(read_until(client, buffer, b"exhios> "))
            client.sendall(b"exit\r")
            output.extend(
                read_until(
                    client,
                    buffer,
                    b"P7.1: worker fault returned to init; reaper reclaimed both processes.\r\n",
                    timeout=12.0,
                )
            )

            print(output.decode("ascii", errors="replace"), end="")
        print("Worker fault recovery and parent continuation verified.")
        return 0
    except (AssertionError, ConnectionError, OSError, RuntimeError, TimeoutError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
