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
- [ ] Build and run on a developer machine; retain the first successful CI result.

**Current code limitation:** the UEFI application only proves that firmware can enter the ExhiOS application. It does not yet load the kernel or leave UEFI Boot Services.

### P0.2 — Kernel file loading and ELF64 validation

- [ ] Locate the ExhiOS kernel on the EFI System Partition.
- [ ] Read the ELF64 image.
- [ ] Validate the ELF header, machine type, program-header table, segment sizes, and address ranges.
- [ ] Reject unsupported or malformed ELF images with a readable diagnostic.
- [ ] Add parser tests using valid and deliberately malformed ELF fixtures.

### P0.3 — Load segments and prepare the handoff

- [ ] Allocate physical pages for each loadable segment.
- [ ] Copy file-backed segment data and zero the memory tail.
- [ ] Validate alignment, overlap, and arithmetic overflow.
- [ ] Obtain the final memory map, GOP framebuffer information, and ACPI RSDP.
- [ ] Populate the shared `BootInfo` structure.

### P0.4 — ExitBootServices and enter ExhiKernel

- [ ] Finalize all boot-service allocations before requesting the final memory map.
- [ ] Exit UEFI Boot Services with the final memory-map key.
- [ ] Transfer control using the x86-64 System V kernel entry ABI.
- [ ] Verify the kernel banner through COM1 in QEMU.
- [ ] Verify the passed `BootInfo` header and memory-map metadata.

**P0 acceptance:** QEMU + OVMF starts the ExhiOS UEFI loader, the loader loads the project's ELF64 kernel, Boot Services are exited, and the kernel prints `ExhiOS Kernel 0.1` through COM1.

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
