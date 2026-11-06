#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$ROOT_DIR"

"$ROOT_DIR/scripts/build.sh"

for tool in qemu-system-x86_64 sgdisk mformat mmd mcopy; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Missing required tool: $tool" >&2
        echo "On Debian/Ubuntu install: qemu-system-x86 ovmf gdisk mtools dosfstools" >&2
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
ESP_OFFSET=1048576

mkdir -p "$ROOT_DIR/target"
rm -f "$ESP_IMAGE"
truncate -s 128M "$ESP_IMAGE"

sgdisk --clear \
    --new=1:2048:0 \
    --typecode=1:EF00 \
    --change-name=1:EFI-System \
    "$ESP_IMAGE"

ESP_MTOOLS_IMAGE="${ESP_IMAGE}@@${ESP_OFFSET}"
mformat -i "$ESP_MTOOLS_IMAGE" -F ::
mmd -i "$ESP_MTOOLS_IMAGE" ::/EFI
mmd -i "$ESP_MTOOLS_IMAGE" ::/EFI/BOOT
mcopy -i "$ESP_MTOOLS_IMAGE" \
    "$ROOT_DIR/target/x86_64-unknown-uefi/debug/exhi-boot.efi" \
    ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$ESP_MTOOLS_IMAGE" \
    "$ROOT_DIR/target/x86_64-unknown-none/debug/exhi-kernel" \
    ::/exhi-kernel.elf

cp "$OVMF_VARS_PATH" "$VARS_COPY"

echo "Starting ExhiOS P0.2 ELF validation test in QEMU."
echo "EFI application and kernel ELF are on the same GPT EFI System Partition."

qemu-system-x86_64 \
    -machine q35 \
    -m 512M \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE_PATH" \
    -drive if=pflash,format=raw,file="$VARS_COPY" \
    -drive if=none,id=esp,format=raw,file="$ESP_IMAGE" \
    -device qemu-xhci,id=xhci \
    -device usb-storage,bus=xhci.0,drive=esp,bootindex=1 \
    -net none \
    -serial stdio \
    -no-reboot \
    -no-shutdown
