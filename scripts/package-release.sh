#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(tr -d '\r\n' < "$ROOT_DIR/VERSION")"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+-alpha\.[0-9]+$ ]]; then
    echo "ERROR: VERSION must use the expected alpha version format." >&2
    exit 1
fi

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Missing required tool: $1" >&2
        exit 1
    fi
}

for tool in cargo rustup readelf sgdisk mformat mmd mcopy mdir sha256sum truncate tar cmp; do
    require_tool "$tool"
done

if [[ ! -s "$ROOT_DIR/Cargo.lock" ]]; then
    echo "Cargo.lock is missing; generating a lockfile for this release package."
    cargo generate-lockfile
fi

rustup target add x86_64-unknown-uefi x86_64-unknown-none

echo
echo "Building ExhiOS userland (Release)..."
cargo build --locked --release -p exhi-init --bin exhi-init --bin exhi-worker --target x86_64-unknown-none

INIT_ELF="$ROOT_DIR/target/x86_64-unknown-none/release/exhi-init"
WORKER_ELF="$ROOT_DIR/target/x86_64-unknown-none/release/exhi-worker"
KERNEL_ELF="$ROOT_DIR/target/x86_64-unknown-none/release/exhi-kernel"
BOOT_EFI="$ROOT_DIR/target/x86_64-unknown-uefi/release/exhi-boot.efi"

verify_et_exec() {
    local image="$1"
    if [[ ! -s "$image" ]]; then
        echo "ERROR: expected build artifact is missing: $image" >&2
        exit 1
    fi
    if ! LC_ALL=C readelf -h "$image" | grep -Eq 'Type:[[:space:]]+EXEC[[:space:]]+\(Executable file\)'; then
        echo "ERROR: $image is not an ELF64 ET_EXEC image." >&2
        exit 1
    fi
}

verify_et_exec "$INIT_ELF"
verify_et_exec "$WORKER_ELF"

echo
echo "Building ExhiOS kernel (Release, production process runtime)..."
cargo build --locked --release -p exhi-kernel --bin exhi-kernel --features process-runtime --target x86_64-unknown-none
verify_et_exec "$KERNEL_ELF"

echo
echo "Building native UEFI bootloader (Release)..."
cargo build --locked --release -p exhi-boot --bin exhi-boot --target x86_64-unknown-uefi
if [[ ! -s "$BOOT_EFI" ]]; then
    # Cargo's x86_64-unknown-uefi target emits PE/COFF executables with .efi.
    # Keep a no-extension fallback for toolchains that use the older naming.
    LEGACY_BOOT_EFI="$ROOT_DIR/target/x86_64-unknown-uefi/release/exhi-boot"
    if [[ -s "$LEGACY_BOOT_EFI" ]]; then
        BOOT_EFI="$LEGACY_BOOT_EFI"
    else
        echo "ERROR: UEFI bootloader artifact is missing." >&2
        echo "Checked: $BOOT_EFI" >&2
        echo "Checked: $LEGACY_BOOT_EFI" >&2
        exit 1
    fi
fi

DIST_DIR="$ROOT_DIR/dist"
PACKAGE_NAME="exhios-$VERSION-x86_64-uefi"
PACKAGE_DIR="$DIST_DIR/$PACKAGE_NAME"
IMAGE="$PACKAGE_DIR/$PACKAGE_NAME.img"
ESP_OFFSET=1048576

mkdir -p "$DIST_DIR"
rm -rf "$PACKAGE_DIR"
mkdir -p "$PACKAGE_DIR"

echo
echo "Creating GPT/UEFI disk image..."
truncate -s 128M "$IMAGE"
sgdisk --clear \
    --new=1:2048:0 \
    --typecode=1:EF00 \
    --change-name=1:EFI-System \
    "$IMAGE"

ESP_MTOOLS_IMAGE="$IMAGE@@$ESP_OFFSET"
mformat -i "$ESP_MTOOLS_IMAGE" -F ::
mmd -i "$ESP_MTOOLS_IMAGE" ::/EFI
mmd -i "$ESP_MTOOLS_IMAGE" ::/EFI/BOOT
mcopy -i "$ESP_MTOOLS_IMAGE" "$BOOT_EFI" ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$ESP_MTOOLS_IMAGE" "$KERNEL_ELF" ::/exhi-kernel.elf

echo
echo "Verifying partition table and files on the EFI System Partition..."
sgdisk --verify "$IMAGE"
mdir -i "$ESP_MTOOLS_IMAGE" ::/EFI/BOOT
mdir -i "$ESP_MTOOLS_IMAGE" ::/

mkdir -p "$ROOT_DIR/target"
VERIFY_DIR="$(mktemp -d "$ROOT_DIR/target/exhios-release-verify.XXXXXX")"
cleanup_verify() {
    rm -rf "$VERIFY_DIR"
}
trap cleanup_verify EXIT
mcopy -i "$ESP_MTOOLS_IMAGE" ::/EFI/BOOT/BOOTX64.EFI "$VERIFY_DIR/BOOTX64.EFI"
mcopy -i "$ESP_MTOOLS_IMAGE" ::/exhi-kernel.elf "$VERIFY_DIR/exhi-kernel.elf"
cmp "$BOOT_EFI" "$VERIFY_DIR/BOOTX64.EFI"
cmp "$KERNEL_ELF" "$VERIFY_DIR/exhi-kernel.elf"
rm -rf "$VERIFY_DIR"
trap - EXIT

cp "$ROOT_DIR/RELEASE_NOTES.md" "$PACKAGE_DIR/RELEASE_NOTES.md"
cp "$ROOT_DIR/CHANGELOG.md" "$PACKAGE_DIR/CHANGELOG.md"
for license_file in LICENSE LICENSE.md COPYING; do
    if [[ -s "$ROOT_DIR/$license_file" ]]; then
        cp "$ROOT_DIR/$license_file" "$PACKAGE_DIR/$license_file"
    fi
done
cp "$ROOT_DIR/VERSION" "$PACKAGE_DIR/VERSION"
cp "$ROOT_DIR/Cargo.lock" "$PACKAGE_DIR/Cargo.lock"

BUILD_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
GIT_COMMIT="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || printf 'unknown')"
cat > "$PACKAGE_DIR/README.txt" <<EOF
ExhiOS $VERSION — x86-64 UEFI alpha image

Image: $PACKAGE_NAME.img
Format: raw disk image, GPT partition table, one FAT32 EFI System Partition.
UEFI fallback loader: EFI/BOOT/BOOTX64.EFI
Kernel: /exhi-kernel.elf
Userland: exhi-init and exhi-worker are embedded in the Release kernel.

Build commit: $GIT_COMMIT
Package created (UTC): $BUILD_UTC

Boot in QEMU:
  From an ExhiOS source checkout, run:
    ./scripts/run-release-qemu.sh

The image is an experimental alpha. Disable Secure Boot. Hardware support
outside the documented QEMU/OVMF profile has not been qualified. Do not use
this image on a machine containing data you need to preserve.
No project license/redistribution terms have been selected yet. Do not publicly
redistribute this package until the project owner adds the intended terms.
EOF

(
    cd "$PACKAGE_DIR"
    sha256sum "$PACKAGE_NAME.img" > SHA256SUMS
)

ARCHIVE="$DIST_DIR/$PACKAGE_NAME.tar.gz"
rm -f "$ARCHIVE" "$ARCHIVE.sha256"
tar -czf "$ARCHIVE" -C "$DIST_DIR" "$PACKAGE_NAME"
(
    cd "$DIST_DIR"
    sha256sum "$(basename "$ARCHIVE")" > "$(basename "$ARCHIVE").sha256"
)

echo
echo "Release package created:"
echo "  Disk image: $IMAGE"
echo "  Archive:    $ARCHIVE"
echo "  Checksum:   $ARCHIVE.sha256"
echo
echo "The Release build and image have not been runtime-tested by this script."
echo "Boot the packaged image with scripts/run-release-qemu.sh before publishing."
