# ExhiOS — журнал контрольных точек

## P0.1 — Repository and firmware entry
Cargo workspace, UEFI app, kernel ELF, shared ABI, build/QEMU scripts и CI. QEMU + OVMF подтвердили вход в loader.

## P0.2 — ELF loading and validation
Loader читает `\exhi-kernel.elf`. Исправлен UEFI path separator. PIE/ET_DYN заменён на fixed ET_EXEC через `relocation-model=static`. ELF parser проверяет заголовки, PT_LOAD bounds, размеры, alignment, W^X, overlap и entry point.

## P0.3 — Segment loading
Точные физические allocations для PT_LOAD; file bytes copy; zero-tail; byte checks; GOP и ACPI RSDP. QEMU подтвердил фактическую загрузку.

## P0.4 — ExitBootServices and kernel handoff
Final UEFI map, BootInfo, 64 KiB stack, ExitBootServices, kernel entry через SysV ABI. QEMU serial подтвердил kernel entry и BootInfo metadata.

## P1.1 — GDT
В QEMU проверены CS=0x08, DS=ES=SS=0x10.

## P1.2 — IDT and exceptions
SIDT base/limit проверены; INT3 вызвал vector 3 (Breakpoint); handler вывел error code 0, RIP, CS и RFLAGS, затем halted.

## P1.3 — PIC and controlled shutdown
PIC masks master/slave прочитаны как 0xFF. isa-debug-exit завершил QEMU с status 33.

## P2.1 — Physical allocator
Отдельный no_std crate `exhi-memory`; descriptor version 1, реальный stride 48; 7 host tests. QEMU: 127 descriptors, 118 837 managed Conventional pages, 159 low pages ignored, frame 0x14C000 выделен/освобождён/повторно выдан. Debug-exit status 33.

## P2.2 — Bootstrap page tables (integrated; runtime pending)
Commit [6ae6826](https://github.com/mindwork64/exhiOS/commit/6ae6826ac5e9ad9969efa907d438571444942dec) экспортирует paging module и подключает его к kernel startup после P2.1.
- Six 4 KiB page-table pages in aligned kernel BSS (not allocator frames).
- PML4 → PDPT → 4 PD; 2 MiB identity map of 0..4 GiB.
- Preflight checks: CPU paging state, IF clear and bounds for code, stack, BootInfo, UEFI map, framebuffer, ACPI RSDP, GDT, IDT and table storage.
- Build/translate checks run before CR3 switch; the same ranges are checked again after switch.
- Kernel reports CR3/root and separately translates its entry point.

CI run 42204902818 succeeded for ABI, ELF, memory allocator tests, UEFI build, default kernel, exception variant, debug-exit variant and ET_EXEC check. The first successful run showed one redundant assignment warning and two unused allocator helper warnings; a cleanup commit removes them.

Headless QEMU/OVMF runtime passed in CI run [42205457670](https://github.com/mindwork64/exhiOS/actions/runs/42205457670). Serial evidence:
- PML4 root and active CR3 both 0x14E000.
- 6 page-table pages; 4 GiB mapped using 2 MiB pages.
- 9 critical address ranges verified before and after CR3 switch.
- Stack address 0x1E70FA00; kernel entry translation 0x100000 → 0x100000.
- Kernel printed the P2.2 success line and QEMU debug-exit returned status 33.
P2.2 is runtime-verified and closed.


## 2026-11-06 — Project Wiki initialized and published

- Added durable context pages: Current State, Architecture, Roadmap, Decisions, Development Log and New Chat Handoff.
- Added `.github/workflows/publish-wiki.yml`, which mirrors managed Markdown pages from `wiki/` into the repository's separate `.wiki.git` using the workflow GITHUB_TOKEN with `contents: write`.
- Existing Home page content outside the managed ExhiOS block is preserved; unrelated Wiki pages are not deleted.
- First publication succeeded. Wiki branch `master`, commit `4181995`.
- Main repository commit: `e3804349338541486fc6b305ed05ca4f1d3700f2`.


## 2026-11-06 — P2.2 Paging integration
- Replaced unsafe assumption that any allocated physical frame can be dereferenced under unknown UEFI page tables.
- Bootstrap page tables now occupy six page-aligned pages in kernel BSS, which is already part of the loaded kernel image.
- Connected paging module to _start after physical allocator initialization.
- Added pre-switch checks for all kernel/firmware handoff ranges and post-switch translation verification.
- CI run 42204902818 succeeded; QEMU runtime confirmation remains required.


## 2026-11-06 — P2.2 runtime verification
- Added a headless QEMU/OVMF CI job to the main workflow; it runs after the compile/test job on each push.
- CI run 42205457670 booted the EFI loader and kernel, switched CR3 to 0x14E000, verified nine critical ranges and the kernel entry translation, then exited with status 33.
- P2.2 is closed. Next milestone: P2.3 Virtual Memory Manager (4 KiB mappings, map/unmap, page flags, TLB invalidation and dynamic page-table frames).


## 2026-11-06 — P2.2 confirmed in user's local QEMU
- User ran the latest local kernel through UEFI/OVMF and supplied serial output.
- PML4 root and active CR3 both 0x10C000.
- Identity map: 4 GiB via 2 MiB pages; 9 critical ranges verified.
- Kernel entry translation 0x100000 → 0x100000.
- Kernel emitted the P2.2 success line; isa-debug-exit returned status 33.
- Root address differs from CI (0x14E000) because firmware memory allocation/layout differs. No functional discrepancy.
- P2.2 confirmed both in CI and locally; next milestone is P2.3 VMM.
