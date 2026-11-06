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

## 2026-11-06 — P2.3 initial VMM implementation

Commits:
- [b066efa](https://github.com/mindwork64/exhiOS/commit/b066efaaddb755822caf1c9c002d4bdeb588c8d0): 4 KiB map/unmap/query, 2 MiB split, NX enablement, TLB invalidation and runtime test.
- [08f179a](https://github.com/mindwork64/exhiOS/commit/08f179af25190092597dead6ad918082fa79b79b): safe 2 MiB coalescing and page-table-frame reclamation.

Implementation:
- New \`arch/x86_64/vmm.rs\` with PageFlags, PageMapping, VmmStatus, map_page, unmap_page, query_page and try_coalesce_large_page.
- Physical allocator exposes allocate_frame/release_frame for paging structures.
- 4 KiB mapping granularity within the initial 0..4 GiB identity window.
- A 2 MiB PDE is split into 512 4 KiB PTEs on the first unmap of a leaf. Existing mappings and cache/permission attributes are preserved, including PAT bit conversion.
- NX support is checked through CPUID and EFER.NXE is enabled only when supported.
- Page mapping supports RW, USER, NX, PWT, PCD and GLOBAL flags. Mutations require IF=0.
- INVLPG is issued for changed mappings. A page-table frame is retained while referenced.
- Coalescing verifies all 512 entries are present, contiguous, 2 MiB-aligned and have matching access/cache attributes. Accessed/dirty bits are merged. The large PDE is installed, all 512 addresses are invalidated, and only then is the child table frame returned to the allocator.

Runtime smoke test in kernel startup:
- Temporarily unmaps VA 0x40000000 (splitting its original 2 MiB mapping).
- Maps a managed physical frame there with supervisor RW+NX permissions.
- Verifies page translation and permission metadata.
- Writes and reads a byte pattern through the virtual address.
- Unmaps it, verifies the translation reports NotMapped, and releases the data frame.
- Restores the original identity mapping, coalesces the 512 PTEs back to a 2 MiB PDE and releases the page-table frame.

Headless QEMU/OVMF CI [run 42213581974](https://github.com/mindwork64/exhiOS/actions/runs/42213581974) passed. Serial markers confirm NX enabled, 2 MiB split, RW supervisor NX read/write, unmap/TLB verification, identity restoration, safe page-table reclamation, and QEMU debug-exit status 33.

Local Linux Mint/QEMU validation by the user is still pending. Do not mark P2.3 fully closed before receiving that serial report.


## 2026-11-06 — P2.3 confirmed by user's local Linux Mint/QEMU run

- User supplied serial output from UEFI/OVMF run after commit 08f179a.
- P2.2 active CR3/PML4: 0x110000; bootstrap identity map covers 4 GiB; 9 address ranges verified.
- VMM reports NX support enabled.
- 2 MiB mapping split into 4 KiB entries.
- RW supervisor NX map, direct write/read test, unmap, NotMapped check and TLB invalidation passed.
- Original identity mapping restored; the temporary 4 KiB page-table frame was safely reclaimed.
- VMM smoke test success line printed; QEMU debug-exit returned status 33.
- P2.3 is closed with both CI and local runtime evidence.

## 2026-11-06 — P2.4 initial Kernel Heap implementation

Commits:
- [e7c7ab6](https://github.com/mindwork64/exhiOS/commit/e7c7ab62c05d0492426688a7ff85afe433a5ed53): initial exhi-heap crate, kernel global allocator, virtual mapping extension and CI unit-test step.
- [dcf6d9c](https://github.com/mindwork64/exhiOS/commit/dcf6d9c2b08e6d4ab0b89d6591f3031a1333a89b): expose std only for host unit tests.
- [97a4859](https://github.com/mindwork64/exhiOS/commit/97a4859693a45a638f00f911cf208c043e7f2199): wire root heap module and clean CPUID unsafe blocks.
- [890d0be](https://github.com/mindwork64/exhiOS/commit/890d0be274f61e02bbfcba933a9699983df53fc9): integrate heap initialize/smoke test into kernel startup.
- [f03e92b](https://github.com/mindwork64/exhiOS/commit/f03e92b7239cd75b95ff1dffc1a4dce053a74f30): fix mutable startup status after smoke test.

Design:
- New no_std `exhi-heap`: first-fit free-list allocator with aligned allocations, block splitting, deallocation, adjacent-region and free-block coalescing.
- Kernel allocator implements `GlobalAlloc`; its initial mapped heap range begins at virtual address 4 GiB, outside the initial identity mapping.
- VMM now accepts lower canonical virtual addresses below 512 GiB while physical mapping targets remain below 4 GiB. Missing PDPT entries are created on demand.
- Heap starts with 4 pages and can grow up to 64 MiB. Physical frames are obtained via `exhi-memory` and mapped RW+NX through VMM.
- An IRQ-safe spinlock serializes heap changes on the current single CPU; local interrupts are disabled while the heap lock is held.
- Page growth is rolled back if a later frame mapping fails.
- Startup smoke test covers 64-byte alignment, a 48 KiB aligned allocation that forces heap growth, write/read across several 4 KiB boundaries, free, pointer reuse and complete free-list coalescing.

Host tests passed: exhi-heap 4/4; existing ABI 2/2, ELF 12/12, physical memory 7/7. Full UEFI/default kernel/exception/debug-exit builds also passed in CI run [42215253674](https://github.com/mindwork64/exhiOS/actions/runs/42215253674). The separate QEMU runtime job was still in its package-install step at this snapshot; do not claim heap runtime verification until its serial output confirms the P2.4 smoke test and debug-exit 33.



## 2026-11-06 — P2.4 Kernel Heap: local and CI runtime verification

Local Linux Mint/QEMU user run after checkpoint 659a260:
- Heap virtual base 0x100000000.
- Initial mapped region: 4 pages / 0x4000 bytes.
- align64 small allocation and 48 KiB align4096 allocation forced growth to 18 pages / 0x12000 bytes.
- Cross-page write/read, free, reuse and coalescing passed.
- All 0x12000 bytes were free after the test.
- Kernel printed P2.4 ready; QEMU debug-exit status 33.

Headless QEMU/OVMF CI [run 42215755990](https://github.com/mindwork64/exhiOS/actions/runs/42215755990) passed the same P2.4 test with identical page/free-byte counts and debug-exit 33. P2.4 is closed with host, local and CI evidence.

## 2026-11-06 — P2.5 initial Kernel Address Spaces implementation

Commits:
- [33b4962](https://github.com/mindwork64/exhiOS/commit/33b49627e8b691a6168d5c0b9860029ff0af0b72) — first implementation and QEMU smoke test.
- [a856090](https://github.com/mindwork64/exhiOS/commit/a856090c0d6e25215e5b56d269894b00696bd01d) — unsafe ownership contract for exposing user frames; supervisor-only shared map assertions.

Implementation:
- New arch/x86_64/address_space.rs.
- Initializes after VMM and heap so the current kernel root includes the heap mapping.
- Creates independent PML4 and PDPT pages. Each process PDPT copies only kernel entries 0..4: physical identity region 0..4 GiB and kernel heap at 4 GiB. Entries 5..7 are reserved.
- Private user mappings use PDPT entries 8..511, VA range 8..512 GiB. Each new space gets a distinct PML4 and PDPT; user lower-level tables are allocated on demand.
- User map flags include writable and NX/executable. map_user_page is unsafe: caller must own and retain the physical frame and scrub it before exposing it to untrusted code.
- unmap_user_page invalidates active translations and reclaims empty PT/PD levels; destroy frees private page tables, PDPT and PML4, but leaves mapped data-frame ownership with the caller.
- activate/restore_kernel switch CR3 with IF=0. Current kernel mappings remain shared and supervisor-only; private user mappings are not shared.

Headless QEMU/OVMF CI [run 42217123278](https://github.com/mindwork64/exhiOS/actions/runs/42217123278) passed:
- Kernel root 0x15B000; address-space A PML4 0x177000; B PML4 0x179000.
- Same VA 0x200000000 resolves to 0x17B000 in A and 0x17C000 in B.
- A wrote/read byte 0xA1; B wrote/read 0xB2 after switching roots.
- Shared kernel code and heap translations were preserved.
- Private tables reclaimed; data frames returned; QEMU debug-exit 33.

Commit a856090 adds explicit validation that kernel code, current stack and heap remain supervisor-only while the PML4 entry allows user mappings elsewhere; map_user_page now has an unsafe caller-ownership/scrub contract. Headless QEMU/OVMF CI [run 42217369804](https://github.com/mindwork64/exhiOS/actions/runs/42217369804) passed the hardened source. User's local QEMU verification of P2.5 is pending. Ring 3 execution and hardware privilege fault tests remain future work.


## 2026-11-06 — P2.5 confirmed by user's local Linux Mint/QEMU run

User ran the safety-hardened address-space implementation (code checkpoint [a856090](https://github.com/mindwork64/exhiOS/commit/a856090c0d6e25215e5b56d269894b00696bd01d)) under local QEMU/OVMF and supplied the serial log.

- Kernel address-space root: 0x11B000.
- Address-space A PML4: 0x177000; B PML4: 0x179000.
- Shared kernel translation: 0x104000.
- Shared heap translation: 0x163000.
- User virtual test page 0x200000000 maps to physical frame 0x17B000 in A and 0x17C000 in B.
- The serial success marker confirms separate roots, shared kernel maps, private user mappings and CR3 switching.
- Private page tables were reclaimed; data frames were safely returned.
- Kernel handoff succeeded; QEMU debug-exit returned status 33.

The local addresses differ from CI addresses as expected because UEFI memory allocation/layout varies between boots. The relationships and tested behavior match CI. P2.5's initial address-space layer is closed with both CI and user-local runtime evidence. This test runs at Ring 0; it does not claim CPL3 execution or hardware-enforced user/supervisor isolation.

Next hardening: add lifecycle/error-path and negative permission tests; keep TSS/IST double-fault stack work in P2.6 ahead of any kernel-stack guard page. Ring 3 remains a later user-mode milestone.

## 2026-11-06 — P2.5.1 address-space lifecycle and mapping hardening (runtime pending)

Commits:
- [dae333c](https://github.com/mindwork64/exhiOS/commit/dae333cf9cee1bcfd9fc0a76ef35a96177291eac) — reject use-after-destroy, make destroy idempotent, reject destruction of the active address space; add QEMU negative checks.
- [9650df1](https://github.com/mindwork64/exhiOS/commit/9650df1c5a2a3fc63e02752cbef53cde39c8cdda) — reject writable+executable user mappings (W^X).
- [3ed5994](https://github.com/mindwork64/exhiOS/commit/3ed599473023912940c20f45f0a1e054824caa46) and [b8e16b9](https://github.com/mindwork64/exhiOS/commit/b8e16b9b05916bba8cbe4d417c613d5e9ad3f4be) — cover invalid/alignment, duplicate map and double-unmap rejection paths; keep test frame ownership valid.
- [ba18080](https://github.com/mindwork64/exhiOS/commit/ba180801d1d33f0a6821e8730f7c7e46c091bd4e) — validate physical table-frame range/alignment before nested page-table dereferences.
- [e19db15](https://github.com/mindwork64/exhiOS/commit/e19db15bf891805c2e22cb3251ebade7872bf515) — preflight complete private tree before teardown; unlink child entries only after successful frame release; support retry if final PML4 release fails.

QEMU startup smoke test now exercises invalid virtual alignment/range, invalid physical alignment, W+X denial before table allocation, duplicate mapping, active-space destroy rejection, double unmap, post-destroy translate/unmap/activate/map rejection and idempotent destroy. It also exercises the normal two-space map/switch/read/reclaim path.

This is a code checkpoint only. CI build, headless QEMU/OVMF and user-local QEMU output have not yet been confirmed for P2.5.1. Do not mark this substage complete until runtime evidence is reviewed.
