#!/usr/bin/env python3
"""Verify one-shot worker setup rollback and subsequent process-slot reuse."""

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
        print("usage: qemu-send-worker-rollback-smoke.py SOCKET", file=sys.stderr)
        return 2

    try:
        with connect(sys.argv[1]) as client:
            buffer = bytearray()
            output = bytearray()
            output.extend(read_until(client, buffer, b"exhios> "))

            # The kernel injects exactly one failure after the worker address
            # space and entry stack have become active. The failed setup must
            # be destroyed and the parent context restored before this prompt.
            client.sendall(b"spawn\r")
            output.extend(
                read_until(
                    client,
                    buffer,
                    b"P7.1: injected worker setup failure after address-space activation.\r\n",
                )
            )
            output.extend(
                read_until(client, buffer, b"worker process could not be started.\r\n")
            )
            output.extend(read_until(client, buffer, b"exhios> "))

            # A second attempt must reuse the rolled-back slot and run normally.
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
                    b"ExhiOS worker: TaskSpawn denied as expected.\r\n",
                )
            )
            output.extend(
                read_until(
                    client,
                    buffer,
                    b"ExhiOS worker: completed yield sequence; exiting.\r\n",
                    timeout=8.0,
                )
            )

            # The Finished worker intentionally owns its slot until init exits.
            client.sendall(b"spawn\r")
            output.extend(
                read_until(client, buffer, b"worker process could not be started.\r\n")
            )
            output.extend(read_until(client, buffer, b"exhios> "))
            client.sendall(b"exit\r")
            output.extend(
                read_until(
                    client,
                    buffer,
                    b"P7.3: interactive serial shell smoke test passed.\r\n",
                    timeout=10.0,
                )
            )
            print(output.decode("ascii", errors="replace"), end="")

        print("Worker rollback, slot reuse and normal reaping verified.")
        return 0
    except (ConnectionError, OSError, RuntimeError, TimeoutError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
