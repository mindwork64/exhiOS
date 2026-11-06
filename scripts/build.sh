#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$ROOT_DIR"

if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup is required. Install the official Rust toolchain first." >&2
    exit 1
fi

rustup target add x86_64-unknown-uefi x86_64-unknown-none

cargo test -p exhi-abi
cargo build -p exhi-boot --target x86_64-unknown-uefi
cargo build -p exhi-kernel --target x86_64-unknown-none

echo
echo "Build completed."
echo "UEFI application: target/x86_64-unknown-uefi/debug/exhi-boot.efi"
echo "Kernel ELF:       target/x86_64-unknown-none/debug/exhi-kernel"
