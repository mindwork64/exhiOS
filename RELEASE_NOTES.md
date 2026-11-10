# ExhiOS 0.1.0-alpha.1

**Status: release candidate preparation; not yet published.**

ExhiOS is an experimental x86-64 operating system with a native UEFI loader, a Rust kernel, protected user-mode execution, and a minimal COM1 serial shell. This alpha is intended for technical evaluation in QEMU/OVMF.

## Included

- UEFI boot path with ELF64 validation, GOP discovery, ACPI RSDP discovery, and kernel handoff.
- Kernel-owned GDT, IDT, TSS and dedicated double-fault IST1.
- Physical page allocation below 4 GiB, four-level paging, 4 KiB virtual memory management, kernel heap, and separate user address spaces.
- Supervisor/user page permissions, W^X user mappings, user-pointer validation, safe user copies, and a common SYSCALL entry/return path.
- Scheduler and idle/reaper paths, isolated Ring 3 init and worker processes, capability-gated worker creation, worker exit and fault cleanup.
- Guarded kernel stacks, including the process/reaper stack guard checks and controlled double-fault delivery through IST1.
- COM1 serial diagnostics and shell, basic GOP framebuffer text output, PIT timer, PS/2 keyboard input decoding, and a fixed-capacity driver registry.
- Volatile in-memory RAMFS configuration.

## Known limitations

- x86-64 UEFI is the only boot target covered by this release.
- The production process/syscall CPU-local context is BSP/single-CPU. SMP scheduling, AP-local GS, user TLS and APIC/IOAPIC support are not included.
- The initial physical allocator manages memory below 4 GiB.
- RAMFS is volatile. There is no persistent disk filesystem, and general userland file-management syscalls are not included.
- The interactive shell is available over COM1 serial. GOP is a basic text framebuffer console, not a desktop or window manager.
- General-purpose USB, network, audio and broad device-driver support are not included.
- Physical machine compatibility has not been qualified. QEMU/OVMF is the reference environment.

## Release validation status

The development milestones P0–P7.4 have prior build and runtime acceptance evidence recorded in the repository wiki. P2.6 process/reaper stack guards also passed a local Linux Mint/QEMU run, and the feature-specific Clippy check passed.

The Release-profile artifacts and packaged GPT image are produced by scripts/package-release.sh. This preparation step does not itself prove that the Release image boots. Before publishing this alpha, run scripts/run-release-qemu.sh and verify the serial shell as described in documentation/BUILD.md.

## Distribution terms

No project license or redistribution terms have been selected yet, and the repository currently has no LICENSE/COPYING file. This alpha is still in release-candidate preparation; define the terms before public distribution.
