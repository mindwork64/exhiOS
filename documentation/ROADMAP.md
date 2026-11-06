# ExhiOS implementation roadmap

## Project rules

- ExhiOS is an independent x86-64 operating system. The Linux kernel is not used.
- Rust Stable is the implementation toolchain.
- The bootloader is a native UEFI application.
- The kernel is a freestanding ELF64 image built for `x86_64-unknown-none`.
- Every milestone must have reproducible build or runtime evidence before it is marked complete.
- A successful compile is not proof that a subsystem works at runtime.

## P0 — UEFI boot chain

### P0.1 — Repository and firmware-entry smoke test

- [x] Initialize the repository and Cargo workspace.
- [x] Add the UEFI and bare-metal Rust targets to the toolchain configuration.
- [x] Add a minimal UEFI application entry point.
- [x] Add a minimal ELF64 kernel entry point and linker script.
- [x] Define the initial shared `BootInfo` data layout.
- [x] Add build, QEMU launch, and CI scripts.
- [x] Build both target images and pass the GitHub Actions checks.
- [x] Run through QEMU + OVMF and confirm that the UEFI firmware enters Exhi Bootloader.

### P0.2 — Kernel file loading and ELF64 validation

- [x] Locate the boot volume through the executing UEFI image handle.
- [x] Read `\\exhi-kernel.elf` from the EFI System Partition.
- [x] Validate the ELF64 magic, class, byte order, version, executable type, and x86-64 machine.
- [x] Bounds-check the program-header table and all loadable segment file ranges.
- [x] Validate segment memory sizes, address overflow, alignment, W^X, and overlap.
- [x] Require the entry point to lie inside an executable loadable segment.
- [x] Add host unit tests for valid and malformed ELF images.
- [x] Build and run in QEMU; confirmed the actual kernel ELF is read and accepted.

### P0.3 — Load segments and prepare the handoff

- [x] Require physical and virtual segment addresses to match for the initial identity-loaded kernel.
- [x] Reject PT_LOAD segments whose page-rounded physical ranges overlap.
- [x] Allocate the exact physical page ranges for each PT_LOAD segment as UEFI LoaderData.
- [x] Copy file-backed segment contents and zero the memory tail through p_memsz.
- [x] Verify copied bytes and zero-filled tails before accepting the loaded segment.
- [x] Collect GOP framebuffer information when direct framebuffer access is available.
- [x] Locate ACPI 2.0 RSDP, falling back to the ACPI 1.0 table.
- [x] Allocate and populate a persistent BootInfo handoff structure.
- [x] Run the P0.3 segment-load path in QEMU; exact-address allocations and memory checks passed.
- [x] Confirm temporary allocations are released by the P0.3 smoke-test path.

### P0.4 — Final memory map, ExitBootServices and enter ExhiKernel

- [x] Reserve the kernel stack, segment pages and BootInfo page before the final memory map.
- [x] Drop filesystem/protocol handles and temporary UEFI heap allocations before ExitBootServices.
- [x] Obtain the final memory map and handle a stale map key using the uefi crate helper.
- [x] Populate BootInfo memory-map address, size, descriptor size and descriptor version.
- [x] Switch to a dedicated kernel stack with the System V x86-64 entry ABI.
- [x] Disable interrupts until the kernel has installed its own IDT.
- [ ] Run the P0.4 handoff in QEMU and confirm the kernel serial banner and BootInfo fields.

**P0 acceptance:** QEMU + OVMF starts the ExhiOS UEFI loader, the loader loads the project's ELF64 kernel, Boot Services are exited, and the kernel prints `ExhiOS Kernel 0.1` through COM1 with valid BootInfo metadata.

## P1 — Kernel foundation

- [ ] GDT and IDT.
- [ ] CPU exception handlers with readable diagnostics.
- [ ] PIC/APIC interrupt strategy for the initial QEMU platform.
- [ ] Controlled test termination through QEMU's debug-exit device.

## P2 — Memory management

- [ ] Physical page allocator based on the UEFI memory map.
- [ ] Kernel-owned page-table management.
- [ ] Virtual memory manager.
- [ ] Kernel heap allocator.
- [ ] Tests for allocation, release, reuse, and invalid ranges.

## P3 — Basic devices

- [ ] Serial output.
- [ ] Framebuffer output through the handoff data.
- [ ] Timer source.
- [ ] Keyboard input.
- [ ] Initial driver registration model.

## P4 — Tasks and scheduling

- [ ] Task representation and kernel stacks.
- [ ] CPU context save/restore.
- [ ] Cooperative scheduler.
- [ ] Timer-driven preemption after the interrupt foundation is stable.

## P5 — User-mode isolation

- [ ] Ring 3 transition.
- [ ] Per-process address spaces.
- [ ] System-call entry and return.
- [ ] User pointer and argument validation.
- [ ] Capabilities and default-deny access rules.

## P6 — Storage

- [ ] VFS interfaces.
- [ ] Initial RAMFS.
- [ ] File and directory operations.

## P7 — Userland

- [ ] First user process (`init`).
- [ ] System-call support library (`libexhi`).
- [ ] Serial shell.
- [ ] Basic utilities and configuration format.
