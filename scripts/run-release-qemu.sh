#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(tr -d '\r\n' < "$ROOT_DIR/VERSION")"
PACKAGE_NAME="exhios-$VERSION-x86_64-uefi"
PACKAGE_DIR="$ROOT_DIR/dist/$PACKAGE_NAME"
DEFAULT_IMAGE="$PACKAGE_DIR/$PACKAGE_NAME.img"
IMAGE="$(printenv EXHIOS_RELEASE_IMAGE 2>/dev/null || true)"
if [[ -z "$IMAGE" ]]; then
    IMAGE="$DEFAULT_IMAGE"
fi
if [[ "$IMAGE" != /* ]]; then
    IMAGE="$ROOT_DIR/$IMAGE"
fi

if [[ ! -s "$IMAGE" ]]; then
    echo "Release image not found: $IMAGE" >&2
    echo "Create it first with: ./scripts/package-release.sh" >&2
    exit 1
fi

if [[ -f "$(dirname "$IMAGE")/SHA256SUMS" ]]; then
    (
        cd "$(dirname "$IMAGE")"
        sha256sum --check SHA256SUMS
    )
fi

for tool in qemu-system-x86_64; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Missing required tool: $tool" >&2
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

OVMF_CODE_ENV="$(printenv OVMF_CODE 2>/dev/null || true)"
OVMF_VARS_ENV="$(printenv OVMF_VARS 2>/dev/null || true)"
OVMF_CODE_PATH="$(find_file \
    "$OVMF_CODE_ENV" \
    /usr/share/OVMF/OVMF_CODE.fd \
    /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/edk2/x64/OVMF_CODE.fd \
    /usr/share/qemu/OVMF_CODE.fd)" || {
        echo "Could not find OVMF_CODE.fd. Set OVMF_CODE to its path." >&2
        exit 1
    }

OVMF_VARS_PATH="$(find_file \
    "$OVMF_VARS_ENV" \
    /usr/share/OVMF/OVMF_VARS.fd \
    /usr/share/OVMF/OVMF_VARS_4M.fd \
    /usr/share/edk2/x64/OVMF_VARS.fd)" || {
        echo "Could not find OVMF_VARS.fd. Set OVMF_VARS to its path." >&2
        exit 1
    }

mkdir -p "$ROOT_DIR/target"
VARS_COPY="$(mktemp "$ROOT_DIR/target/exhios-release-vars.XXXXXX")"
cp "$OVMF_VARS_PATH" "$VARS_COPY"
cleanup() {
    rm -f "$VARS_COPY"
}
trap cleanup EXIT

QEMU_MEMORY="$(printenv EXHIOS_QEMU_MEMORY 2>/dev/null || true)"
if [[ -z "$QEMU_MEMORY" ]]; then
    QEMU_MEMORY="512M"
fi

QEMU_ARGS=(
    -machine q35
    -m "$QEMU_MEMORY"
    -drive "if=pflash,format=raw,readonly=on,file=$OVMF_CODE_PATH"
    -drive "if=pflash,format=raw,file=$VARS_COPY"
    -drive "if=none,id=esp,format=raw,file=$IMAGE"
    -device qemu-xhci,id=xhci
    -device usb-storage,bus=xhci.0,drive=esp,bootindex=1
    -net none
    -no-reboot
    -serial stdio
)

HEADLESS="$(printenv EXHIOS_HEADLESS 2>/dev/null || true)"
if [[ "$HEADLESS" == "1" ]]; then
    QEMU_ARGS+=(-display none -monitor none)
fi

echo "Starting ExhiOS $VERSION Release image in QEMU/OVMF."
echo "Image: $IMAGE"
echo "Use the terminal for the COM1 serial shell. Stop QEMU with Ctrl+C."
qemu-system-x86_64 "${QEMU_ARGS[@]}"
