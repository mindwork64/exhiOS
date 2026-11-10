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

- [x] Review and integrate the initial four-level page-table builder.
- [x] Build a 0–4 GiB identity map using 2 MiB pages.
- [x] Switch CR3 and verify kernel, stack and page-table translations in QEMU.
- [x] Add controlled failure/rollback checks; CI verifies a post-CR3 failure restores the exact original root and leaves initialization retryable.
- [x] Integrate the virtual memory manager.
- [x] Integrate the kernel heap allocator.
- [x] Add tests for allocation, release, reuse, and invalid ranges.
  
  - Runtime evidence: the self-hosted QEMU run verifies the six-page bootstrap hierarchy, CR3 read-back, kernel/stack/table identity translations, rejection of unaligned and out-of-window mappings, VMM split/map/unmap/TLB invalidation, heap growth/free/reuse, and address-space frame reclamation. A separate controlled rollback run confirms restoration of the original CR3 after a simulated post-switch failure.
  - CI: [run 380](https://github.com/mindwork64/exhiOS/actions/runs/42432075760) completed successfully, including the rollback scenario.

## P3 — Basic devices

- [x] Serial output: COM1 polling 16550 driver is initialized during boot and used for kernel diagnostics.
- [x] Framebuffer output through the handoff data: GOP format and stride are validated and the unified COM1/framebuffer writer is initialized.
- [x] Timer source: PIT channel 0 runs at 100 Hz; QEMU verifies monotonic tick advancement and clean IRQ0 masking.
- [x] Keyboard input: i8042/PS/2 IRQ1 is initialized; headless QEMU receives actual A, Shift+B and Ctrl+D transitions over QMP, verifies eight decoded events, zero dropped scancodes and masks IRQ1 on exit.
- [x] Initial driver registration model: UART, PIT and PS/2 keyboard are registered; the runtime registry smoke test verifies initialization state and zero failures.

  - Runtime evidence: [CI run 384](https://github.com/mindwork64/exhiOS/actions/runs/42432439209) completed successfully. Its QEMU runtime job passed all device and system smoke scenarios, including automated keyboard input injection. The keyboard log reports events=8, dropped-scancodes=0, IRQ1-mask=0xFF and confirms A, Shift+B and Ctrl+D were observed.

## P4 — Tasks and scheduling

- [x] Task representation and kernel stacks.
- [x] CPU context save/restore for cooperative Ring 0 tasks.
- [x] Cooperative round-robin scheduler.
- [x] Timer-driven preemption for Ring 0 tasks (local Linux Mint/QEMU + self-hosted CI accepted).
- [x] Preemptive task termination and Finished-state handoff (local Linux Mint/QEMU + self-hosted CI accepted 2026-11-07).
- [x] Idle-context dispatch and wake-up of blocked tasks: dedicated idle interrupt frame/stack, `INT 0x30` block path, `sti; hlt` idle, PIT deadline wake-up and IRETQ dispatch. Self-hosted build + QEMU runtime accepted in [run 42337327289](https://github.com/mindwork64/exhiOS/actions/runs/42337327289).

## P5 — User-mode isolation

P5.1–P5.7 are implemented and accepted by the full self-hosted build and headless QEMU runtime suite. The first production process entry and syscall context are currently BSP/single-CPU; AP-local GS setup, user TLS, NMI/exception SWAPGS-window handling, and SMP process scheduling remain explicitly outside this milestone.

- [x] **P5.1 — Ring 3 entry and CPL3 exception return:** user selectors, TSS.RSP0, isolated address space, and processor-originated exception return.
- [x] **P5.2 — Supervisor isolation:** verify that CPL3 cannot read supervisor-only kernel mappings.
- [x] **P5.3 — System-call entry/return:** configure SYSCALL MSRs, switch to a dedicated kernel stack, validate the return frame, and return to CPL3.
- [x] **P5.4 — User-pointer validation:** validate canonical ranges, mapped pages, and read/write/execute permissions before dereferencing.
- [x] **P5.5 — Safe user copies:** validate the complete range before copying and reject cross-boundary copies without partial writes.
- [x] **P5.6 — User-task lifecycle:** run two isolated Ring 3 tasks with private CR3 roots and stacks; exercise SYSCALL yield/exit, idle dispatch, kernel reaping, address-space/frame reclamation, and removal of Finished scheduler records. Host scheduler lifecycle tests cover the exit-to-idle-to-reaper transition and safe task-record reclamation.
- [x] **P5.7 — Capabilities and default-deny access rules:** define process capabilities and enforce access checks at kernel service boundaries.

  - Progress (2026-11-07): `exhi-task` now has a typed capability set, rejects unknown serialized bits, gives new user tasks an empty set, and exposes a scheduler authorization check for the current task. This is the policy foundation, not completion: production syscall dispatch and actual kernel service boundaries still need to consume the check.
  - Runtime progress (2026-11-07): the P5.6 Ring 3 lifecycle smoke path checks `TaskYield` and `TaskExit` before scheduler dispatch. Task A receives both grants; task B receives only `TaskExit`, attempts `SYS_YIELD`, verifies the `-1` denial result, and continues without a scheduler transition.
  - Service progress (2026-11-07): `exhi-abi` defines stable syscall numbers, the six-register argument convention and negative return statuses. The feature-gated Ring 3 runtime implements `SYS_CONSOLE_WRITE` with a `ConsoleWrite` grant, complete user-range validation/copy, and byte-count return. QEMU verifies task A's authorized write and task B's no-side-effect denial.
  - Dispatch-core progress (2026-11-07): a new allocation-free `exhi-syscall` crate owns the syscall-to-capability mapping, default-deny checks, result/error conversion, and service dispatch. The Ring 3 smoke adapter now supplies scheduler and console services through `SyscallServices`; it no longer chooses syscall numbers or capability grants itself. Host tests cover unknown calls, empty grants, denied no-side-effect behavior, authorized console ranges, yield/exit switching, and service errors. At this checkpoint, architecture entry/frame validation and user-copy still lived in the feature-gated smoke runtime; these were subsequently promoted into always-built architecture modules (see the 2026-11-08 acceptance notes below).
  - Architecture progress (2026-11-07): x86-64 SYSCALL MSR programming has been extracted into the always-available `arch::x86_64::syscall` module. It validates the entry address, CPU SYSCALL support, GDT selector relationship, writes STAR/LSTAR/FMASK, enables EFER.SCE last, and rolls SCE back if read-back verification fails. Both P5.3 and P5.6 smoke paths now use this shared setup. At this checkpoint, a production entry stub and per-task kernel-stack selection were still missing; both are covered by the common-entry work below.
  - Entry-stub progress (2026-11-07): the P5.6 assembly entry, its normalized 20-qword syscall frame, and the assembly-visible user-RSP/kernel-stack/FX-state slots now live in `arch::x86_64::syscall`, rather than the general `user_mode` smoke harness. The scheduler-specific Rust handler remains in the smoke harness and is called through a narrow symbol boundary. At this checkpoint the stub was feature-gated; it was later replaced by the always-built shared BSP entry and dispatcher context described below.
  - GS-entry progress (2026-11-07): the lifecycle smoke adapter now stores its entry context in a 64-byte-aligned GS-relative boot-CPU structure. Initialization sets `IA32_GS_BASE` to the smoke harness's user-GS baseline and `IA32_KERNEL_GS_BASE` to the trusted context; the entry executes `SWAPGS` before reading the user RSP or switching stacks, and task selection updates the active kernel-stack/FX-state fields. This removed the previous assembly-visible global slots. At that checkpoint the adapter was still single-CPU test scaffolding; the shared entry was subsequently made always available, while AP-local allocation, user TLS and NMI/exception SWAPGS-window handling remain SMP follow-up work.
  - Production-entry progress (2026-11-08): `arch::x86_64::user` now exposes the common IRETQ entry, validates the active address space, RX user code, RW+NX user stack, supervisor kernel stack, TSS.RSP0, FXSAVE state and the full processor exception frame. The P5.1 QEMU test explicitly rejects inactive address spaces, writable/non-executable code, non-writable user stacks and malformed kernel-stack bounds before confirming a successful INT3 exception return.
  - Production user-memory progress (2026-11-08): `validate_user_range`, `copy_from_user` and `copy_to_user` now live in the always-built `arch::x86_64::user` module. They require IF=0 and the target address space active, validate every page and permission before dereferencing, cap non-zero buffers at 16 MiB, and reject invalid cross-boundary copies without partial writes.
  - Runtime evidence for P5.1/P5.2/P5.4/P5.5: [CI run 396](https://github.com/mindwork64/exhiOS/actions/runs/42433321881) completed successfully. QEMU verified Ring 3 entry/exception return, supervisor-only page fault, pointer permission/guard/overflow checks, inactive-address-space rejection, copy round-trip, and atomic rejection of an invalid destination range. This implementation is currently BSP/single-CPU and does not claim SMP readiness.
  - Common syscall entry acceptance (2026-11-08): `arch::x86_64::syscall` now owns one always-built SYSCALL entry and a 64-byte-aligned GS-relative CPU-local context containing the user RSP slot, trusted kernel stack, FXSAVE image and dispatcher pointer. Both the one-task syscall round-trip and the two-task lifecycle use this same 20-qword frame, `SWAPGS`, kernel-stack switch, FXSAVE/FXRSTOR and IRETQ return. Task selection updates the current task's trusted stack/FX image from inside the swapped-to kernel GS state.
  - Capability-boundary acceptance (2026-11-08): `exhi-syscall` now asks the current runtime for its `CapabilitySet` and performs the required-capability check itself before invoking any service. Missing/current-kernel tasks resolve to the empty set; user tasks are empty by default; only kernel scheduler APIs can replace grants; kernel tasks reject user-service grants; unknown capability bits are rejected. The runtime adapter cannot override the dispatcher's allow/deny decision.
  - Full P5 runtime evidence: [CI run 405](https://github.com/mindwork64/exhiOS/actions/runs/42434078573) completed successfully, including all build/unit-test steps and all 20 headless QEMU runtime scenarios. QEMU verified the common SYSCALL entry/return, two separate user CR3/code/stack contexts, two yields and exits, authorized ConsoleWrite by A, denied Yield and ConsoleWrite by B with no unauthorized output, kernel reaping, page/frame reclamation, task-record removal, and TSS.RSP0 cleanup. Counters: six syscalls, two exits, two denials, one authorized console write, three task switches. The accepted configuration remains BSP-only: no AP-local GS installation or user TLS is claimed.

## P6 — Storage

- [x] VFS interfaces: allocator-free `FileSystem` contract, stable generation-tagged `NodeId`, metadata and paginated directory-entry API.
- [x] Initial RAMFS: bounded `RamFs<NODE_COUNT, FILE_CAPACITY>` implementation with a fixed node table and fixed per-node data capacity; no allocator dependency.
- [x] File and directory operations: lookup, nested path resolution, create/read/write/remove, directory pagination, sparse zero-fill, capacity/overflow checks, non-empty directory protection and stale-handle rejection.

  - Unit evidence: CI #412 ran `cargo test -p exhi-vfs`; all eight VFS/RAMFS tests passed.
  - Kernel integration: `exhi-vfs` is a workspace dependency of `exhi-kernel`. The initial root RAMFS path is exercised from the kernel after handoff: create `/etc/exhi.conf`, write/read its binary contents, resolve absolute and relative paths, enumerate its directory, reject an oversized write without mutation, protect a non-empty directory, remove nodes, and verify a reused slot does not revive a stale handle.
  - QEMU evidence: [CI run 412](https://github.com/mindwork64/exhiOS/actions/runs/42546613316) completed successfully. The build job and the entire headless QEMU runtime job passed. Its VFS-specific serial output includes `P6: VFS/RAMFS runtime smoke test passed.` and QEMU debug-exit status 33; the P2.6 guard test no longer intercepts this feature path.
## P7 — Userland

- [ ] **P7.1 — Production process manager:** implemented code now contains a two-slot registry and capability-gated `SpawnWorker`. Init can create one built-in worker with an independent address space, guarded kernel stack, FXSAVE image and scheduler record; the worker becomes runnable while init remains active. Init alone receives `TaskSpawn`; the worker receives only `TaskYield`, `TaskExit` and `ConsoleWrite`. The shell QEMU driver issues `spawn`, waits for worker startup/completion and requires both process records to be reaped. Runtime acceptance remains pending.
- [ ] **P7.2 — libexhi:** `no_std` wrappers cover `console_write`, `console_read`, `file_read`, `spawn_worker`, `yield_now` and `exit`. Dispatcher tests for denied/authorized spawn are present but were not run.
- [ ] **P7.3 — Serial shell:** the bounded COM1 shell includes `spawn`; the serial QEMU scenario has been extended but not executed.
- [ ] **P7.4 — Configuration:** init reads `/etc/exhi.conf` from kernel RAMFS through capability-gated `FileRead`, applies `hostname`, `prompt` and `version`, and falls back to defaults on malformed input. RAMFS is volatile and kernel-populated; persistent storage and FileWrite are outside P7. Runtime validation remains pending.

**Validation:** user-side `cargo build -p exhi-init --bin exhi-init --bin exhi-worker --target x86_64-unknown-none` succeeded. Clippy completed successfully for the workspace library/binary targets and the `process-runtime`, `process-shell-smoke-test` and `process-multi-smoke-test` kernel configurations; existing warnings remain in legacy modules. No unit tests, GitHub Actions or QEMU runs were performed. P7 remains unchecked until runtime evidence is available.
