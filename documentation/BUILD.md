# Building ExhiOS

## Toolchain

The workspace uses Rust Stable and requires Rust 1.91 or newer for the current UEFI crate dependency.

The repository's `rust-toolchain.toml` selects the Stable channel and requests:

- `x86_64-unknown-uefi` for the UEFI application;
- `x86_64-unknown-none` for the freestanding ELF64 kernel;
- `rustfmt` and `clippy`.

Install Rust through rustup if it is not already installed. Entering the repository lets rustup select the configured toolchain.

The `x86_64-unknown-none` target supports PIE by default. The initial ExhiOS loader expects fixed-address `ET_EXEC` images, so `.cargo/config.toml` explicitly selects `-C relocation-model=static` for the kernel. Do not remove this setting unless the loader is extended to apply ELF relocations.

## Build and unit tests

Install the host-side inspection utility on Debian/Ubuntu-based systems:

```bash
sudo apt install binutils
```

From the repository root:

```bash
./scripts/build.sh
```

The script installs the two compilation targets, runs the `exhi-abi` and `exhi-elf` unit tests, and builds the UEFI application and kernel. It also uses `readelf` to verify that the resulting kernel is `ET_EXEC` rather than `ET_DYN`.

Expected artifacts:

- `target/x86_64-unknown-uefi/debug/exhi-boot.efi`
- `target/x86_64-unknown-none/debug/exhi-kernel`

The kernel output is an ELF64 executable. Its initial link address is `0x00100000`.

## P0.2: read and validate the kernel ELF in QEMU

On Debian/Ubuntu-based systems, install the runtime tools:

```bash
sudo apt install qemu-system-x86 ovmf gdisk mtools dosfstools
```

Then run:

```bash
./scripts/run-qemu.sh
```

The script creates a GPT disk with a FAT32 EFI System Partition, copies the application to `EFI/BOOT/BOOTX64.EFI`, copies the kernel to `/exhi-kernel.elf`, and starts QEMU with the disk attached as emulated USB storage.

The loader obtains the filesystem associated with its own UEFI image handle and reads the kernel from the EFI System Partition root using the UEFI path `\\exhi-kernel.elf` (UEFI paths use backslash separators). It then validates the ELF64 headers and loadable segments. The diagnostic remains visible for 15 seconds before the application returns to firmware.

Expected output includes:

```text
ExhiOS Bootloader 0.2
P0.2: loading and validating the kernel ELF64 image.
Kernel file found: ... bytes.
Kernel file read successfully.
ELF64 validation: OK
Entry point: 0x...
Program headers: ...
PT_LOAD #...: ...
P0.2 complete: kernel image located, read and validated.
No segments have been loaded into physical memory yet.
```

## Scope of this checkpoint

P0.2 only locates, reads, and validates the ELF image. The loader does **not** yet allocate physical memory for its segments, copy the image into memory, obtain the final boot information, call `ExitBootServices`, or transfer control to ExhiKernel.
