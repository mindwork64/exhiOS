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
- [x] Drop filesystem/protocol handles and temporary heap-backed buffers before ExitBootServices.
- [x] Obtain the final memory map and handle a stale map key using the uefi crate helper.
- [x] Populate BootInfo memory-map address, size, descriptor size and descriptor version.
- [x] Switch to a dedicated kernel stack with the System V x86-64 entry ABI.
- [x] Disable interrupts until the kernel has installed its own IDT.
- [x] Run the P0.4 handoff in QEMU and confirm the kernel serial banner and BootInfo fields.

**P0 acceptance:** QEMU + OVMF starts the ExhiOS UEFI loader, the loader loads the project's ELF64 kernel, Boot Services are exited, and the kernel prints `ExhiOS Kernel 0.1` through COM1 with valid BootInfo metadata.

## P1 — Kernel foundation

### P1.1 — Kernel-owned GDT

- [x] Define null, 64-bit kernel code and kernel data descriptors.
- [x] Load the kernel GDT with LGDT.
- [x] Reload CS with a far return and reload DS, ES and SS.
- [x] Read back and verify the active segment selectors.
- [x] Run the GDT installation and selector check in QEMU; CS/DS/ES/SS matched the expected selectors.

### P1.2 — IDT and CPU exceptions

- [x] Define and load a kernel-owned 256-entry IDT.
- [x] Add exception stubs for CPU exception vectors 0–31.
- [x] Normalize exception stack frames, including CPU-pushed error codes.
- [x] Add fatal exception diagnostics with vector, name, error code, RIP, CS and RFLAGS.
- [x] Report CR2 when a page fault occurs.
- [x] Verify IDT installation and SIDT readback in QEMU (base and 0x0FFF limit matched).
- [x] Trigger INT3 in QEMU and verify vector 3 diagnostics and fatal halt.

### P1.3 — Interrupt controllers and test exit

- [x] Define the initial interrupt strategy: remap the legacy 8259 PIC to vectors 0x20–0x2F; defer local APIC/IOAPIC activation until ACPI MADT parsing and IRQ handlers exist.
- [x] Initialize the PIC and mask all IRQ lines while keeping the CPU interrupt flag clear.
- [x] Add controlled QEMU test termination through the isa-debug-exit device at I/O port 0xF4.
- [x] Verify PIC mask readback (0xFF on both controllers) and QEMU debug-exit behavior (status 33) in QEMU.

## P2 — Memory management

### P2.1 — Initial physical page allocator

- [x] Parse UEFI memory descriptor version 1 using the firmware-reported descriptor stride.
- [x] Make only EfiConventionalMemory pages available; retain loader, firmware, runtime, ACPI and MMIO ranges as reserved.
- [x] Reserve physical addresses below 1 MiB and cap the first bitmap allocator at 4 GiB.
- [x] Add managed/allocated bitmaps, single-page allocation, release and reuse checks.
- [x] Add host tests for descriptor stride, malformed ranges, overlaps, reserved types, allocation, release and reuse.
- [x] Verify memory-map initialization and allocation/reuse in QEMU (118,837 managed pages; frame 0x14C000 was freed and reused).

### P2.2 — Bootstrap page tables

- [ ] Review and integrate the initial four-level page-table builder.
- [ ] Build a 0–4 GiB identity map using 2 MiB pages.
- [ ] Switch CR3 and verify kernel, stack and page-table translations in QEMU.
- [ ] Add controlled failure/rollback checks.
- [ ] Integrate the virtual memory manager.
- [ ] Integrate the kernel heap allocator.
- [ ] Add tests for allocation, release, reuse, and invalid ranges.

## P3 — Basic devices

- [ ] Serial output.
- [ ] Framebuffer output through the handoff data.
- [ ] Timer source.
- [ ] Keyboard input.
- [ ] Initial driver registration model.

## P4 — Tasks and scheduling

- [x] Task representation and kernel stacks.
- [x] CPU context save/restore for cooperative Ring 0 tasks.
- [x] Cooperative round-robin scheduler.
- [x] Timer-driven preemption for Ring 0 tasks (local Linux Mint/QEMU + self-hosted CI accepted).
- [x] Preemptive task termination and Finished-state handoff (local Linux Mint/QEMU + self-hosted CI accepted 2026-11-07).
- [x] Idle-context dispatch and wake-up of blocked tasks: dedicated idle interrupt frame/stack, `INT 0x30` block path, `sti; hlt` idle, PIT deadline wake-up and IRETQ dispatch. Self-hosted build + QEMU runtime accepted in [run 42337327289](https://github.com/mindwork64/exhiOS/actions/runs/42337327289).

## P5 — User-mode isolation

The implementation and dedicated headless QEMU smoke-test paths now exist for P5.1–P5.6. Keep these milestones open until the self-hosted QEMU runtime results have been reviewed and accepted.

- [ ] **P5.1 — Ring 3 entry and CPL3 exception return:** user selectors, TSS.RSP0, isolated address space, and processor-originated exception return.
- [ ] **P5.2 — Supervisor isolation:** verify that CPL3 cannot read supervisor-only kernel mappings.
- [ ] **P5.3 — System-call entry/return:** configure SYSCALL MSRs, switch to a dedicated kernel stack, validate the return frame, and return to CPL3.
- [ ] **P5.4 — User-pointer validation:** validate canonical ranges, mapped pages, and read/write/execute permissions before dereferencing.
- [ ] **P5.5 — Safe user copies:** validate the complete range before copying and reject cross-boundary copies without partial writes.
- [ ] **P5.6 — User-task lifecycle:** run two isolated Ring 3 tasks with private CR3 roots and stacks; exercise SYSCALL yield/exit, idle dispatch, kernel reaping, address-space/frame reclamation, and removal of Finished scheduler records. Host scheduler lifecycle tests cover the exit-to-idle-to-reaper transition and safe task-record reclamation.
- [ ] **P5.7 — Capabilities and default-deny access rules:** define process capabilities and enforce access checks at kernel service boundaries.

  - Progress (2026-11-07): `exhi-task` now has a typed capability set, rejects unknown serialized bits, gives new user tasks an empty set, and exposes a scheduler authorization check for the current task. This is the policy foundation, not completion: production syscall dispatch and actual kernel service boundaries still need to consume the check.

## P6 — Storage

- [ ] VFS interfaces.
- [ ] Initial RAMFS.
- [ ] File and directory operations.

## P7 — Userland

- [ ] First user process (`init`).
- [ ] System-call support library (`libexhi`).
- [ ] Serial shell.
- [ ] Basic utilities and configuration format.
