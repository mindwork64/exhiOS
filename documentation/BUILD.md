# Building ExhiOS

## Toolchain

The workspace uses Rust Stable and requires Rust 1.91 or newer for the current UEFI crate dependency.

The repository's `rust-toolchain.toml` selects Stable and requests:

- `x86_64-unknown-uefi` for the UEFI application;
- `x86_64-unknown-none` for the freestanding ELF64 kernel;
- `rustfmt` and `clippy`.

The kernel is linked as a fixed-address `ET_EXEC` image. The loader requires physical and virtual addresses in each loadable segment to match and loads at those addresses using UEFI page allocation.

## Build and unit tests

Install the host-side inspection utility on Debian/Ubuntu-based systems:

```bash
sudo apt install binutils
```

From the repository root:

```bash
./scripts/build.sh
```

The script runs the ABI and ELF parser tests, builds the UEFI application and kernel, and verifies that the kernel output is an ELF64 `ET_EXEC` image.

Expected artifacts:

- `target/x86_64-unknown-uefi/debug/exhi-boot.efi`
- `target/x86_64-unknown-none/debug/exhi-kernel`

## P0.3: load and verify kernel segments in QEMU

On Debian/Ubuntu-based systems, install the runtime tools:

```bash
sudo apt install qemu-system-x86 ovmf gdisk mtools dosfstools
```

Then run:

```bash
./scripts/run-qemu.sh
```

The script creates a GPT disk with a FAT32 EFI System Partition, copies the loader to `EFI/BOOT/BOOTX64.EFI`, copies the kernel to `\\exhi-kernel.elf`, and starts QEMU.

The loader:

1. Reads and validates the kernel ELF64 image.
2. For each `PT_LOAD`, computes the page-rounded physical range.
3. Requests those exact physical pages from UEFI as `LOADER_DATA`.
4. Clears the allocation, copies `p_filesz` bytes, and zeroes the tail through `p_memsz`.
5. Verifies the copied bytes and zero-filled tail.
6. Collects GOP framebuffer information when direct access is supported and finds the ACPI RSDP.
7. Allocates and populates a `BootInfo` structure.

The loader releases these temporary allocations before returning to firmware. Kernel handoff and `ExitBootServices` remain part of P0.4.

Expected output includes:

```text
ExhiOS Bootloader 0.3
P0.3: loading kernel segments into physical memory.
ELF64 validation: OK
Reserving PT_LOAD #0: physical=0x..., pages=..., bytes=...
PT_LOAD #0: loaded at 0x...; ... verified.
Reserving PT_LOAD #1: physical=0x..., pages=..., bytes=...
PT_LOAD #1: loaded at 0x...; ... verified.
GOP framebuffer: ...
ACPI RSDP: ...
BootInfo prepared at 0x...
P0.3 preparation complete.
Kernel segments are loaded at their fixed physical addresses.
Segment contents and zero-filled tails were verified.
Kernel has not been entered; UEFI Boot Services remain active.
```

## P0.4: final memory map and kernel handoff

The final memory map must be retrieved only after the loader has completed its UEFI allocations. P0.4 will reserve the memory-map and BootInfo storage first, obtain the current map key, populate the handoff metadata, exit Boot Services, and enter the kernel.

## Scope of this checkpoint

P0.3 does not call `ExitBootServices` or execute the kernel. The temporary segment and BootInfo allocations are freed after successful verification so the current smoke test leaves firmware in a clean state.
