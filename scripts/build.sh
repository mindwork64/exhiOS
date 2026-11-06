#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$ROOT_DIR"

if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup is required. Install the official Rust toolchain first." >&2
    exit 1
fi

if ! command -v readelf >/dev/null 2>&1; then
    echo "readelf is required to verify the kernel image type. Install binutils." >&2
    exit 1
fi

if [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" && "${EXHIOS_QEMU_EXIT_TEST:-0}" == "1" ]]; then
    echo "Choose either EXHIOS_EXCEPTION_TEST or EXHIOS_QEMU_EXIT_TEST, not both." >&2
    exit 1
fi
if [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] && {
    [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] || [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" == "1" ]];
}; then
    echo "EXHIOS_SERIAL_RX_TEST cannot be combined with exception or debug-exit tests." >&2
    exit 1
fi

if [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_TIMER_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with exception or serial RX tests." >&2
        exit 1
    fi
fi

rustup target add x86_64-unknown-uefi x86_64-unknown-none

cargo test -p exhi-abi
cargo test -p exhi-elf
cargo test -p exhi-memory
cargo test -p exhi-console
cargo build -p exhi-boot --target x86_64-unknown-uefi

KERNEL_FEATURES=()
if [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(exception-smoke-test)
    echo "Building the kernel with the optional INT3 exception smoke test."
fi
if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(qemu-debug-exit-smoke-test)
    echo "Building the kernel with the QEMU debug-exit smoke test."
fi
if [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(serial-rx-smoke-test)
    echo "Building the kernel with the interactive COM1 RX smoke test."
fi
if [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(timer-smoke-test)
    echo "Building the kernel with the PIT IRQ0 timer smoke test."
fi

FEATURE_ARGS=()
if (( ${#KERNEL_FEATURES[@]} > 0 )); then
    FEATURE_LIST="$(IFS=,; printf '%s' "${KERNEL_FEATURES[*]}")"
    FEATURE_ARGS+=(--features "$FEATURE_LIST")
fi

cargo build -p exhi-kernel --target x86_64-unknown-none "${FEATURE_ARGS[@]}"

KERNEL_ELF="$ROOT_DIR/target/x86_64-unknown-none/debug/exhi-kernel"
echo
echo "Verifying kernel ELF type..."
LC_ALL=C readelf -h "$KERNEL_ELF"
if ! LC_ALL=C readelf -h "$KERNEL_ELF" | grep -Eq 'Type:[[:space:]]+EXEC[[:space:]]+\(Executable file\)'; then
    echo "ERROR: kernel must be ET_EXEC, not PIE/ET_DYN." >&2
    exit 1
fi

echo
echo "Build completed."
echo "UEFI application: target/x86_64-unknown-uefi/debug/exhi-boot.efi"
echo "Kernel ELF:       target/x86_64-unknown-none/debug/exhi-kernel"
