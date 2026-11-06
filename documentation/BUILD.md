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

Expected QEMU output is split between the UEFI display and serial console. The UEFI display should end with the handoff message; the QEMU terminal should show:

```text
ExhiOS Kernel 0.1
Kernel entry point reached.
BootInfo: valid
Memory map address: 0x...
Memory map size: 0x...
Memory descriptor size: 0x...
Memory descriptor version: 0x...
Memory descriptors: 0x...
Framebuffer address: 0x...
Framebuffer resolution: 0x...
ACPI RSDP address: 0x...
ExhiOS: kernel handoff successful.
ExhiOS: interrupts remain disabled until the IDT is ready.
ExhiOS: halt loop entered.
```

The kernel intentionally leaves interrupts disabled and halts after printing the handoff information. It does not yet install an IDT or initialize its own page tables.

## Scope of this checkpoint

P0.4 validates the complete UEFI-to-kernel handoff and memory-map delivery. GDT/IDT setup, exception handling, kernel-owned page tables, physical memory manager and drivers are subsequent milestones.


## P1.1: install the kernel GDT

At kernel entry, ExhiOS replaces the firmware-provided GDT with its own three-entry table:

- selector 0x08: 64-bit kernel code segment;
- selector 0x10: kernel data/stack segment;
- selector 0x00: mandatory null descriptor.

The loader already enters the kernel with interrupts disabled. The GDT setup loads GDTR, performs a far return to reload CS, and reloads DS, ES and SS. It does not enable interrupts or change the active paging structures.

After installation, the kernel reads CS, DS, ES and SS back and verifies the expected selector values. The QEMU serial output should include:

    GDT: installing kernel GDT.
    GDT selectors: CS=0x0008 DS=0x0010 ES=0x0010 SS=0x0010
    GDT: installed and verified.

The QEMU runtime check is still required before P1.1 is considered complete.
