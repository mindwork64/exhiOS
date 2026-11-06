# Building ExhiOS

## Toolchain

The workspace uses Rust Stable and requires Rust 1.91 or newer for the current UEFI crate dependency.

The repository's `rust-toolchain.toml` selects the Stable channel and requests:

- `x86_64-unknown-uefi` for the UEFI application;
- `x86_64-unknown-none` for the freestanding ELF64 kernel;
- `rustfmt` and `clippy`.

Install Rust through rustup if it is not already installed. Entering the repository lets rustup select the configured toolchain.

## Build and unit tests

From the repository root:

```bash
./scripts/build.sh
```

The script installs the two compilation targets, runs the `exhi-abi` unit tests, and builds the UEFI application and kernel.

Expected artifacts:

- `target/x86_64-unknown-uefi/debug/exhi-boot.efi`
- `target/x86_64-unknown-none/debug/exhi-kernel`

The kernel output is an ELF64 executable. Its initial link address is `0x00100000`.

## UEFI entry smoke test in QEMU

On Debian/Ubuntu-based systems, install the runtime tools:

```bash
sudo apt install qemu-system-x86 ovmf mtools dosfstools
```

Then run:

```bash
./scripts/run-qemu.sh
```

The script creates a FAT EFI System Partition image, copies the application to the standard removable-media path `EFI/BOOT/BOOTX64.EFI`, creates a writable copy of OVMF variables, and starts QEMU.

The UEFI console should show the ExhiOS Bootloader banner. The application then returns to firmware.

## Scope of this checkpoint

This is only the firmware-entry smoke test. The current loader does **not** read the kernel ELF, allocate its load segments, build the final memory map, call `ExitBootServices`, or transfer control to the kernel. Those tasks belong to the remaining P0 milestones.

The kernel can be built and its ELF header inspected at this stage, but it is not yet launched by the UEFI application.
