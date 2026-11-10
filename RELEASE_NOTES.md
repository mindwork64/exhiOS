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

The Release-profile package was built locally on Linux Mint and booted successfully through QEMU/OVMF. The GPT/FAT32 image passed partition-table verification; the UEFI loader and kernel were read back from the EFI System Partition and matched their source artifacts byte-for-byte. The Release kernel reached the Ring 3 serial shell.

The shell's help, hostname, version, config and echo commands were exercised. Worker creation, the worker's expected TaskSpawn denial, yield/exit, and repeated worker replacement after deferred cleanup were confirmed. After the first worker exited, four subsequent workers each reused the freed process slot and exited normally. The run was performed after the TaskSpawn capability check had been ordered before reclamation.

The current serial line editor handles CR/LF, Backspace/DEL and Ctrl+C; arrow-key navigation and command history are not implemented. Terminal up-arrow escape sequences may therefore appear as literal `[A` input.

This is QEMU/OVMF validation, not physical-hardware qualification. Cargo.lock is committed in the source repository. Before publication, complete final formatting and Clippy checks for userland and the kernel `process-runtime` path, independently verify both the archive and disk-image checksums, and choose/add project redistribution terms. Rebuild and repeat the Release QEMU check after any final source changes.

## Distribution terms

ExhiOS is licensed under the Apache License, Version 2.0 (Apache-2.0); see the repository's `LICENSE` file. The release packaging script includes this file in the source bundle. Third-party dependencies remain subject to their respective license terms.
