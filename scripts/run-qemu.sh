#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$ROOT_DIR"

"$ROOT_DIR/scripts/build.sh"

for tool in qemu-system-x86_64 mformat mmd mcopy; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Missing required tool: $tool" >&2
        echo "On Debian/Ubuntu install: qemu-system-x86 ovmf mtools dosfstools" >&2
        exit 1
    fi
done

find_file() {
    for candidate in "$@"; do
        if [[ -n "$candidate" && -f "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    return 1
}

OVMF_CODE_PATH="$(find_file \
    "${OVMF_CODE:-}" \
    /usr/share/OVMF/OVMF_CODE.fd \
    /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/edk2/x64/OVMF_CODE.fd \
    /usr/share/qemu/OVMF_CODE.fd)" || {
        echo "Could not find OVMF_CODE.fd. Set OVMF_CODE to its path." >&2
        exit 1
    }

OVMF_VARS_PATH="$(find_file \
    "${OVMF_VARS:-}" \
    /usr/share/OVMF/OVMF_VARS.fd \
    /usr/share/OVMF/OVMF_VARS_4M.fd \
    /usr/share/edk2/x64/OVMF_VARS.fd \
    /usr/share/qemu/OVMF_VARS.fd)" || {
        echo "Could not find OVMF_VARS.fd. Set OVMF_VARS to its path." >&2
        exit 1
    }

ESP_IMAGE="$ROOT_DIR/target/exhios-esp.img"
VARS_COPY="$ROOT_DIR/target/exhios-ovmf-vars.fd"
mkdir -p "$ROOT_DIR/target"
rm -f "$ESP_IMAGE"
truncate -s 64M "$ESP_IMAGE"

mformat -i "$ESP_IMAGE" -F ::
mmd -i "$ESP_IMAGE" ::/EFI
mmd -i "$ESP_IMAGE" ::/EFI/BOOT
mcopy -i "$ESP_IMAGE" \
    "$ROOT_DIR/target/x86_64-unknown-uefi/debug/exhi-boot.efi" \
    ::/EFI/BOOT/BOOTX64.EFI

cp "$OVMF_VARS_PATH" "$VARS_COPY"

echo "Starting ExhiOS UEFI entry smoke test in QEMU."
echo "The UEFI console should display the ExhiOS Bootloader banner."
qemu-system-x86_64 \
    -machine q35 \
    -m 512M \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE_PATH" \
    -drive if=pflash,format=raw,file="$VARS_COPY" \
    -drive format=raw,file="$ESP_IMAGE" \
    -net none \
    -serial stdio \
    -no-reboot \
    -no-shutdown
