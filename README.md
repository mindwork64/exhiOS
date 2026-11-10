# ExhiOS

**Current release: [0.1.0-alpha.1](https://github.com/mindwork64/exhiOS/releases/tag/0.1.0-alpha.1) (published pre-release; Release image booted and exercised in QEMU/OVMF).**

ExhiOS is an experimental independent x86-64 operating system written in Rust. It has a native UEFI bootloader, a protected-mode kernel with virtual memory and guarded stacks, isolated Ring 3 processes, a syscall interface, and a small serial shell.

This is an early technical preview, not a general-purpose desktop operating system. The published alpha targets UEFI boot on x86-64, with QEMU/OVMF as the reference environment.

## Current capabilities

- Native UEFI bootloader loads and validates a fixed-address ELF64 kernel.
- GOP framebuffer console and COM1 serial diagnostics.
- Physical page allocator, four-level paging, 4 KiB mappings, kernel heap, and separate user address spaces.
- GDT/TSS/IDT, CPU exception handling, guarded kernel stacks, and dedicated double-fault IST1.
- Scheduler, idle handling, syscall entry/return, Ring 3 isolation, and capability-checked services.
- Init process and one dynamically spawned worker, with process cleanup and fault recovery.
- Interactive COM1 shell: help, hostname, version, echo, clear, config, spawn, and exit.
- Bounded in-memory RAMFS configuration; settings are not persistent across reboot.

## Download

- [UEFI disk image (.img)](https://github.com/mindwork64/exhiOS/releases/download/0.1.0-alpha.1/exhios-0.1.0-alpha.1-x86_64-uefi.img)
- [Release bundle (.tar.gz)](https://github.com/mindwork64/exhiOS/releases/download/0.1.0-alpha.1/exhios-0.1.0-alpha.1-x86_64-uefi.tar.gz)
- [Archive checksum](https://github.com/mindwork64/exhiOS/releases/download/0.1.0-alpha.1/exhios-0.1.0-alpha.1-x86_64-uefi.tar.gz.sha256) · [Image checksums](https://github.com/mindwork64/exhiOS/releases/download/0.1.0-alpha.1/SHA256SUMS)

## Build and run

Install Rust Stable with the targets listed in rust-toolchain.toml and the host tools documented in [Build instructions](documentation/BUILD.md).

Create the Release UEFI disk image:

    ./scripts/package-release.sh

Boot it with QEMU/OVMF:

    ./scripts/run-release-qemu.sh

For a headless QEMU session:

    EXHIOS_HEADLESS=1 ./scripts/run-release-qemu.sh

See [Changelog](CHANGELOG.md), [Release notes](RELEASE_NOTES.md), [Build instructions](documentation/BUILD.md), and [Userland guide](documentation/USERLAND.md).

## Known limitations

- x86-64 UEFI only; current runtime and syscall CPU-local context are BSP/single-CPU.
- The initial physical allocator manages frames below 4 GiB.
- The in-memory RAMFS is volatile; no persistent disk filesystem or userland file-management interface is included.
- The shell is serial-based. GOP is a text framebuffer console, not a graphical desktop.
- General-purpose USB, networking, audio, and broad device-driver support are not implemented.
- SMP, AP-local GS, user TLS, and advanced interrupt-controller support are future work.
- Real-hardware compatibility has not been qualified; use QEMU/OVMF first.

The release image is an experimental alpha. Do not use it as the only boot medium or on a system containing data you need to preserve.

## License and distribution

ExhiOS source code is licensed under the Apache License, Version 2.0 (Apache-2.0); see [LICENSE](LICENSE). Third-party dependencies remain subject to their own license terms.
