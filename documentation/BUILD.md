# Building and running ExhiOS

## Supported build environment

ExhiOS is an experimental x86-64 UEFI operating system. Rust Stable is used for the UEFI bootloader, freestanding kernel, and Ring 3 userland. The repository toolchain file selects:

- x86_64-unknown-uefi
- x86_64-unknown-none
- rustfmt
- clippy

The kernel and user executables are fixed-address ELF64 ET_EXEC images. The bootloader loads the kernel at its declared physical addresses; the kernel maps user executables into private address spaces.

## Host dependencies

On Debian/Ubuntu/Linux Mint:

    sudo apt update
    sudo apt install binutils qemu-system-x86 ovmf gdisk mtools dosfstools

Rustup and Cargo must be installed. The rust-toolchain.toml file selects the required targets and components.

## Debug build and QEMU smoke tests

Build the current development images and run the standard UEFI boot:

    ./scripts/build.sh
    ./scripts/run-qemu.sh

The debug build artifacts are placed under target/. The runner creates a temporary GPT/FAT EFI System Partition and boots it with QEMU/OVMF.

The repository also contains feature-specific smoke-test modes. Their environment variables and expected serial markers are documented in the ExhiOS Wiki roadmap and development log.

## Release image

The release candidate version is stored in VERSION. Build a Release userland, then the kernel with that Release userland embedded, then the native UEFI bootloader, and package the result as a raw GPT disk image:

    ./scripts/package-release.sh

The first invocation generates Cargo.lock if the repository does not yet contain one. The resulting lockfile is copied into the release bundle; commit it to the source repository before creating a final release tag so future builds use the same resolved dependency versions.

Output:

- dist/exhios-0.1.0-alpha.1-x86_64-uefi/exhios-0.1.0-alpha.1-x86_64-uefi.img
- dist/exhios-0.1.0-alpha.1-x86_64-uefi/SHA256SUMS
- dist/exhios-0.1.0-alpha.1-x86_64-uefi/RELEASE_NOTES.md
- dist/exhios-0.1.0-alpha.1-x86_64-uefi/CHANGELOG.md
- dist/exhios-0.1.0-alpha.1-x86_64-uefi.tar.gz
- dist/exhios-0.1.0-alpha.1-x86_64-uefi.tar.gz.sha256

The image contains one GPT EFI System Partition formatted as FAT32, the fallback boot path EFI/BOOT/BOOTX64.EFI, and /exhi-kernel.elf. The first user executables are embedded into the kernel; separate init/worker files are not required on the ESP.

### Boot the packaged Release image in QEMU

From the source checkout:

    ./scripts/run-release-qemu.sh

The graphical QEMU window uses GOP; the terminal remains attached to COM1 and accepts shell input. To run without a display:

    EXHIOS_HEADLESS=1 ./scripts/run-release-qemu.sh

The default image path is the package produced by package-release.sh. Override it with EXHIOS_RELEASE_IMAGE=/absolute/path/to/image.img. OVMF_CODE and OVMF_VARS can be set when firmware files are installed in non-standard locations. QEMU memory defaults to 512 MiB and can be changed with EXHIOS_QEMU_MEMORY.

The guest does not have a QEMU debug-exit device in this release runner. Stop it with Ctrl+C after checking the boot messages and serial shell.

## USB media

The raw image is intended for UEFI removable media. Writing it to a USB device destroys the existing contents of the selected device. On Linux, first identify the correct whole-disk device with lsblk, then write the image (replace /dev/sdX with the verified device):

    sudo dd if=dist/exhios-0.1.0-alpha.1-x86_64-uefi/exhios-0.1.0-alpha.1-x86_64-uefi.img of=/dev/sdX bs=4M status=progress conv=fsync

Do not use a partition path such as /dev/sdX1. Re-check the destination before pressing Enter. The current alpha has not been qualified on a broad set of physical machines; QEMU/OVMF is the reference test environment.

## Pre-release verification

Before publishing a release, run from a clean checkout:

    cargo fmt --all -- --check
    cargo clippy -p exhi-init --bins --target x86_64-unknown-none
    cargo clippy -p exhi-kernel --bin exhi-kernel --features process-runtime --target x86_64-unknown-none
    ./scripts/package-release.sh
    ./scripts/run-release-qemu.sh

Confirm the serial log reaches the interactive Ring 3 shell, run help, version, config and spawn, confirm the worker exits cleanly, and stop QEMU with Ctrl+C. Also verify the archive checksum:

    cd dist
    sha256sum --check exhios-0.1.0-alpha.1-x86_64-uefi.tar.gz.sha256

Do not publish until the Release build and packaged-image boot have been verified. The existing debug/QEMU acceptance evidence does not substitute for this final Release-profile check.
