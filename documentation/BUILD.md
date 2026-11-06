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

## P1.1: kernel GDT — QEMU verified

At kernel entry, ExhiOS replaces the firmware-provided GDT with its own three-entry table:

- selector 0x08: 64-bit kernel code segment;
- selector 0x10: kernel data/stack segment;
- selector 0x00: mandatory null descriptor.

The GDT setup loads GDTR, performs a far return to reload CS, and reloads DS, ES and SS. The kernel reads CS, DS, ES and SS back and verifies the expected selector values. This was confirmed in QEMU.

Expected output:

```text
GDT selectors: CS=0x0008 DS=0x0010 ES=0x0010 SS=0x0010
GDT: installed and verified.
```

## P1.2: kernel IDT and CPU exception diagnostics — QEMU verified

The kernel installs a 256-entry IDT with gates for the 32 x86 CPU exception vectors. Each assembly stub normalizes the stack layout so the common handler receives the vector, error code, RIP, CS and RFLAGS consistently. The fatal handler reports those values over COM1; for a page fault it also reads CR2 to show the faulting linear address.

The normal QEMU run verified the active IDT base and 0x0FFF limit with LIDT/SIDT. The optional INT3 test also ran successfully:

```bash
EXHIOS_EXCEPTION_TEST=1 ./scripts/run-qemu.sh
```

The kernel deliberately executes INT3 after installing the IDT. QEMU confirmed vector 3 (Breakpoint), error code 0, the kernel CS selector and the fatal halt path. This test intentionally halts the guest after printing diagnostics; stop QEMU with Ctrl+C.

## P1.3: PIC setup and controlled QEMU exit

The initial interrupt-controller strategy is deliberately conservative:

- Remap the legacy dual 8259 PIC so IRQ0–7 use vectors 0x20–0x27 and IRQ8–15 use 0x28–0x2F, away from CPU exceptions.
- Mask all 16 PIC inputs and keep IF cleared. The kernel must not enable interrupts until gates and handlers for the corresponding IRQ vectors are installed.
- Keep local APIC/IOAPIC activation for the later ACPI MADT and device-interrupt stage. The current q35 QEMU profile uses the legacy PIC only as a masked compatibility baseline.

To verify PIC mask readback and the QEMU debug-exit device, run:

```bash
EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh
```

The test build writes 0x10 to I/O port 0xF4, where the script attaches QEMU's `isa-debug-exit` device. QEMU exits with status 33; the script recognizes that exact status as a successful controlled shutdown and returns zero. A different exit status is reported as a failed test.

The QEMU serial output should include:

```text
PIC: remapping legacy IRQs to vectors 0x20-0x2F.
PIC master mask: 0x00000000000000FF slave mask: 0x00000000000000FF
PIC: remap sequence issued; all IRQ masks verified.
QEMU debug-exit: requesting controlled shutdown.
QEMU debug-exit confirmed (exit status 33).
```

P1.3 remains open until this runtime test has been observed in QEMU.
