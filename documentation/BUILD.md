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

## P0.4: exit UEFI Boot Services and enter ExhiKernel

On Debian/Ubuntu-based systems, install the runtime tools:

```bash
sudo apt install qemu-system-x86 ovmf gdisk mtools dosfstools
```

Then run:

```bash
./scripts/run-qemu.sh
```

The loader reads and validates the kernel ELF, loads each `PT_LOAD` segment into its declared physical address, and reserves a 64 KiB kernel stack and a persistent `BootInfo` page.

Before exiting Boot Services, it closes the filesystem and drops temporary heap-backed buffers. The `uefi` crate's `boot::exit_boot_services(None)` helper obtains the final memory map and retries the ExitBootServices operation if the map key becomes stale. The loader stores the returned memory-map buffer address, size, descriptor size and descriptor version in BootInfo.

The loader then switches to its dedicated kernel stack, disables interrupts, and calls the kernel entry point using the x86-64 System V ABI. No UEFI protocol, console, allocator or Boot Services API is used after the transition.

## P1.1: install the kernel GDT

At kernel entry, ExhiOS replaces the firmware-provided GDT with its own three-entry table:

- selector 0x08: 64-bit kernel code segment;
- selector 0x10: kernel data/stack segment;
- selector 0x00: mandatory null descriptor.

The GDT setup loads GDTR, performs a far return to reload CS, and reloads DS, ES and SS. The kernel reads CS, DS, ES and SS back and verifies the expected selector values. The QEMU runtime check is required before P1.1 is considered complete.

Expected output includes:

```text
GDT: installing kernel GDT.
GDT selectors: CS=0x0008 DS=0x0010 ES=0x0010 SS=0x0010
GDT: installed and verified.
```

## P1.2: kernel IDT and CPU exception diagnostics

The kernel installs a 256-entry IDT with gates for the 32 x86 CPU exception vectors. Each assembly stub normalizes the stack layout so the common handler receives the vector, error code, RIP, CS and RFLAGS consistently. The fatal handler reports those values over COM1; for a page fault it also reads CR2 to show the faulting linear address.

The default QEMU run verifies that LIDT/SIDT agree on the active IDT base and limit. To exercise the actual exception entry path, run the optional INT3 test:

```bash
EXHIOS_EXCEPTION_TEST=1 ./scripts/run-qemu.sh
```

The kernel deliberately executes INT3 after installing the IDT. The expected serial output includes:

```text
IDT smoke test: triggering INT3 breakpoint.

ExhiOS CPU EXCEPTION
Vector: 0x0000000000000003 (Breakpoint)
Error code: 0x0000000000000000
RIP: 0x...
CS: 0x0000000000000008
RFLAGS: 0x...
ExhiOS: CPU halted after fatal exception.
```

This test intentionally halts the guest after printing diagnostics. Stop QEMU with Ctrl+C in the terminal.

P1.2 remains open until both the normal IDT installation and the optional exception smoke test have been observed in QEMU.
