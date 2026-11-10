# Changelog

All notable ExhiOS release changes are recorded here.

## 0.1.0-alpha.1 — unreleased

Initial x86-64 UEFI technical preview.

- Native UEFI bootloader and fixed-address ELF64 kernel handoff.
- Kernel GDT/IDT/TSS, exception diagnostics, guarded stacks and double-fault IST1.
- Physical page allocator (initially below 4 GiB), four-level paging, kernel heap and private user address spaces.
- Kernel scheduler/idle/reaper foundations, Ring 3 execution, common SYSCALL path and capability-gated services.
- Init/worker process lifecycle, worker fault recovery and bounded volatile RAMFS configuration.
- COM1 serial shell, basic GOP framebuffer text output, PIT timer and PS/2 keyboard decoding.
- Release packaging scripts for a raw GPT/FAT32 UEFI image, SHA-256 manifests and QEMU/OVMF launch.

Release candidate validation (2026-11-11): the Release-profile GPT/FAT32 image booted through QEMU/OVMF, reached the Ring 3 serial shell, and passed manual command and worker lifecycle checks. After the first worker exited, four further workers were each spawned successfully after deferred reclamation of the previous Finished record. The run also validates that the TaskSpawn capability check occurs before reclamation.

The serial shell does not yet support cursor-key navigation or command history.

This alpha does not include SMP, persistent storage, a graphical desktop, networking or broad device support. ExhiOS source is licensed under Apache-2.0. Publication remains pending final formatting/Clippy checks and independent archive/image checksum verification.
