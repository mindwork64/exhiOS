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

This alpha does not include SMP, persistent storage, a graphical desktop, networking or broad device support. Release-profile build and packaged-image boot validation are pending.
