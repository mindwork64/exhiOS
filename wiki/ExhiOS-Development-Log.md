# ExhiOS — журнал контрольных точек

## 2026-11-06 — P3.3 local acceptance and P3.4 keyboard implementation

- P3.3 PIT timer accepted locally: user's QEMU serial log shows IRQ0 unmasked, five monotonic ticks received, IRQ0 remasked and debug-exit status 33.
- Added the no_std `exhi-input` crate: fixed-capacity SPSC scan-code queue, overflow counter, Set 1 make/break decoder, HID-usage key identifiers, modifier snapshots and lock-key state.
- Added the x86-64 i8042 driver. It configures translated Set 1 mode, enables scanning after validating keyboard ACK, and unmasks only PIC IRQ1.
- Added IDT vector 0x21 gate with read-back validation and an assembly-only IRQ1 handler that reads port 0x60, publishes a byte to the queue, counts overflow and sends EOI. No Rust call or heap allocation occurs in the ISR.
- Host tests passed 6/6 and the keyboard-enabled kernel build passed CI [run 42278708100](https://github.com/mindwork64/exhiOS/actions/runs/42278708100). User QEMU log (2026-11-06): config 0x65, IDT IRQ1 valid, 28 key transitions, Ctrl+D recognized, zero dropped scan codes, IRQ1 remasked and debug-exit 33. No standalone Shift event was visible in that log; modifier decoding is host-tested.

## 2026-11-07 — P3.5 initial driver registry

- Added the no_std `exhi-driver` crate with immutable descriptors, stable IDs/classes, Registered/Initialized/Failed lifecycle and a fixed-capacity registry.
- Registry validates capacity, duplicate IDs, unknown IDs and state transitions; supports lookup, iteration and counts without heap allocation.
- COM1, PIT channel 0 and PS/2 keyboard now expose descriptors. Kernel startup registers all three; serial is initialized early, while PIT and keyboard transition only after their respective hardware init succeeds.
- Boot diagnostics print registry summary and each entry.
- Host tests passed 8/8; CI `bash -n` validation, all kernel feature variants and headless QEMU smoke tests (standard boot, PIT timer and registry runtime) passed in [run 42299723267](https://github.com/mindwork64/exhiOS/actions/runs/42299723267). User local Linux Mint/QEMU log (2026-11-06): registered=3/initialized=1/failed=0; COM1 Initialized, PIT and PS/2 Registered; registry runtime pass marker and QEMU status 33. P3.5 is closed.



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

## 2026-11-06 — P2.5.1 address-space lifecycle and mapping hardening (local + CI runtime verified)

Commits:
- [dae333c](https://github.com/mindwork64/exhiOS/commit/dae333cf9cee1bcfd9fc0a76ef35a96177291eac) — reject use-after-destroy, make destroy idempotent, reject destruction of the active address space; add QEMU negative checks.
- [9650df1](https://github.com/mindwork64/exhiOS/commit/9650df1c5a2a3fc63e02752cbef53cde39c8cdda) — reject writable+executable user mappings (W^X).
- [3ed5994](https://github.com/mindwork64/exhiOS/commit/3ed599473023912940c20f45f0a1e054824caa46) and [b8e16b9](https://github.com/mindwork64/exhiOS/commit/b8e16b9b05916bba8cbe4d417c613d5e9ad3f4be) — cover invalid/alignment, duplicate map and double-unmap rejection paths; keep test frame ownership valid.
- [ba18080](https://github.com/mindwork64/exhiOS/commit/ba180801d1d33f0a6821e8730f7c7e46c091bd4e) — validate physical table-frame range/alignment before nested page-table dereferences.
- [e19db15](https://github.com/mindwork64/exhiOS/commit/e19db15bf891805c2e22cb3251ebade7872bf515) — preflight complete private tree before teardown; unlink child entries only after successful frame release; support retry if final PML4 release fails.

QEMU startup smoke test now exercises invalid virtual alignment/range, invalid physical alignment, W+X denial before table allocation, duplicate mapping, active-space destroy rejection, double unmap, post-destroy translate/unmap/activate/map rejection and idempotent destroy. It also exercises the normal two-space map/switch/read/reclaim path.

Headless CI/OVMF [run 42238977281](https://github.com/mindwork64/exhiOS/actions/runs/42238977281) passed both build and QEMU runtime jobs. The user's local Linux Mint/QEMU log (2026-11-06) reaches the address-space smoke-test success line and debug-exit status 33. The negative checks execute inside that smoke test before the success marker, so P2.5.1 is closed with local + CI evidence.

## 2026-11-06 — P2.6 TSS, double-fault IST and boot-stack guard page (CI runtime verified)

Commits:
- [5ceab05](https://github.com/mindwork64/exhiOS/commit/5ceab059cea53eb85a4559e474cf69b0222016ff) — initial 64-bit TSS descriptor in GDT, dedicated 16 KiB aligned IST1 stack, LTR and TR readback.
- [9c78d80](https://github.com/mindwork64/exhiOS/commit/9c78d80f96843856d30db6b5093be456660ef25a) — IDT vector 8 configured with IST index 1 and readback in IdtStatus.
- [8e27dec](https://github.com/mindwork64/exhiOS/commit/8e27dec596fd778f7b69fdfa7969b6ad208980b1) — exception frame pointer passed as the sixth SysV argument.
- [e0bbdf0](https://github.com/mindwork64/exhiOS/commit/e0bbdf021693f25a80a35d3f9d92a9f18e328f8f) — controlled #DF/IST1 test path and warning cleanup.
- [cf93050](https://github.com/mindwork64/exhiOS/commit/cf930507a626004691169a5a02a6c6e444ce7e39) — BootInfo ABI v2 includes kernel stack base/size.
- [0140362](https://github.com/mindwork64/exhiOS/commit/014036220e27854a81acaf74dcb46f283935bfc9) and [92f64aa](https://github.com/mindwork64/exhiOS/commit/92f64aa5f263245cabbeef96d725bc751928029d) — physical page reservation for the guard frame.
- [c31f516](https://github.com/mindwork64/exhiOS/commit/c31f516f0aa6934756ece4069bf4ad053fc8b03c) — validate and map the full boot stack range in bootstrap paging.
- [aa434e8](https://github.com/mindwork64/exhiOS/commit/aa434e8b36b4d6a0c4d353eb6a818c916e9b2812), [d941d95](https://github.com/mindwork64/exhiOS/commit/d941d95c12f7ba7b9634177994fba74db7b1878b), [18d686c](https://github.com/mindwork64/exhiOS/commit/18d686cf83615cbe2515155dbee9bfd5024e5db7), [0bc9bc3](https://github.com/mindwork64/exhiOS/commit/0bc9bc3468a833f8870aefe8e76fce46396ccf0f) and [2749f78](https://github.com/mindwork64/exhiOS/commit/2749f78de1bbdbc449af40a7d0c43701d08c3665) — guard page installation, deliberate stack-overflow #DF trigger, and dedicated contiguous loader allocation.
- Latest cleanup checkpoint: [b46c870](https://github.com/mindwork64/exhiOS/commit/b46c8702e3725281e9a42654524649d1d04b4fe8).

Implementation:
- GDT contains null, kernel code, kernel data and a two-qword 64-bit available-TSS descriptor (selector 0x18).
- TSS is 104 bytes; IST1 points to a dedicated 16 KiB page-aligned static stack in kernel BSS. RSP0 is intentionally left unset until task-specific kernel stacks and privilege transitions are added.
- IDT vector 8 uses IST1. The exception stub passes the normalized exception-frame pointer to the Rust handler; #DF verifies it is inside IST1.
- BootInfo magic/version advanced to EXHIOS02/version 2 and now transfers the exact boot stack base/size.
- UEFI loader reserves one contiguous 17-page LOADER_DATA allocation: first page is the guard reservation, following 16 pages are the 64 KiB usable stack.
- Kernel reserves the guard frame in the physical allocator before later allocations, then removes its virtual identity mapping through VMM after parsing the UEFI map.
- QEMU debug-exit build sets RSP to the first byte of the mapped stack and reads the guard page. The first #PF's exception-frame push faults into the guard page and forces a processor #DF, which must switch to IST1. Handler exits with status 33 only if the exception frame lies inside IST1.

Headless CI/OVMF [run 42256850017](https://github.com/mindwork64/exhiOS/actions/runs/42256850017) passed build and QEMU runtime:
- Loader guard physical address 0x1E6FF000; usable stack 0x1E700000..0x1E710000.
- Kernel confirms guard is unmapped and first/last stack pages remain supervisor mapped.
- Exception stack pointer 0x16AFC8 is inside IST1.
- P2.6 double-fault IST runtime success marker and QEMU debug-exit status 33.

Local Linux Mint/QEMU verification (2026-11-06, user-supplied serial log) closed the milestone:
- Guard page 0x1E0D3000 is unmapped; 64 KiB kernel stack is 0x1E0D4000..0x1E0E4000 and remains supervisor-only.
- A controlled guard fault produced vector 8 (Double Fault). Exception frame pointer 0x122FC8 lies inside IST1 0x11F000..0x123000.
- The P2.6 runtime success marker printed and QEMU debug-exit returned status 33.
P2.6 is now locally and CI runtime verified.

## 2026-11-06 — P3.1 polling serial driver implementation

Commits:
- [eb60bdf](https://github.com/mindwork64/exhiOS/commit/eb60bdf377f4f28211504fe4152bd26db10d1eee) — introduced a reusable 16550-compatible SerialPort abstraction.
- [97f8a9b](https://github.com/mindwork64/exhiOS/commit/97f8a9b43d4e3d9453ed46fd5956aee5adfd4851) — exported the serial driver module from x86_64 architecture.
- [11251d5](https://github.com/mindwork64/exhiOS/commit/11251d5c57151a86f153dec96493c1f686101979) — migrated main.rs boot/exception diagnostics from direct COM1 I/O helpers to serial::COM1; first headless QEMU regression passed.
- [14f2268](https://github.com/mindwork64/exhiOS/commit/14f22683bf1f5c02f7740ccecbf330dae43e2b27) — added explicit P3.1 readiness marker in the boot log.
- [d1ea614](https://github.com/mindwork64/exhiOS/commit/d1ea6145e4fa2c10a04e03725d7677b679cbede0) — added non-blocking try_read_byte and polling read_byte_blocking.

Implementation:
- SerialPort accepts a configurable UART I/O base; COM1 at 0x3F8 is the early kernel console.
- Initialization configures 38400 baud, 8N1, FIFO and modem-control lines.
- TX uses bounded polling on the 16550 line-status register; string and 64-bit hexadecimal output are provided.
- RX offers a non-blocking byte read and a blocking polling read for a later serial shell. The driver is deliberately single-core and non-interrupt-driven while IF remains clear.
- main.rs no longer owns UART register constants or raw port I/O helpers; GDT, IDT, memory, VMM and exception diagnostics all use the serial module.

Headless QEMU/OVMF [run 42257896620](https://github.com/mindwork64/exhiOS/actions/runs/42257896620) passed and printed the P3.1 COM1 readiness marker. The same run confirmed the P2.6 guard-page/#DF/IST1 path and debug-exit status 33. The RX-inclusive build and QEMU regression [run 42258141119](https://github.com/mindwork64/exhiOS/actions/runs/42258141119) both passed. Boot output included the P3.1 readiness marker and the P2.6 guard-page/#DF/IST1 success marker. CI does not inject terminal input, so actual interactive RX still requires local QEMU stdio validation.

Interactive serial input on the user's local Linux Mint/QEMU terminal has not yet been verified.


## 2026-11-06 — Interactive COM1 RX diagnostic mode added

Commits:
- [a58220e](https://github.com/mindwork64/exhiOS/commit/a58220e5fda9afb069b71f08529ebc68f95a4d36) — adds the `serial-rx-smoke-test` kernel feature, build-script switch `EXHIOS_SERIAL_RX_TEST=1`, and a dedicated CI compile step.
- [6014dec](https://github.com/mindwork64/exhiOS/commit/6014dec505682a4d91ff7165d222f274a15561da) — isolates the interactive RX path from the automated debug-exit path without introducing unreachable-code warnings.

The opt-in mode blocks on `SerialPort::read_byte_blocking`, echoes printable ASCII, supports Enter and Backspace echo, and reports the number of received bytes when Ctrl+D is received. It is intentionally a diagnostic byte-echo loop, not a command shell or line editor. Normal boot and automated QEMU debug-exit behavior remain separate.

The user's 2026-11-06 local log confirms COM1 TX and the full P2.6 guard/#DF/IST1 test. Actual typed-byte RX validation is still pending. The interactive RX feature build passed in workflow run [42268731034](https://github.com/mindwork64/exhiOS/actions/runs/42268731034); its headless QEMU regression job is still in progress. Neither build nor headless regression validates receipt of keyboard bytes.


## 2026-11-06 — P3.1 interactive COM1 RX confirmed locally

The user ran `EXHIOS_SERIAL_RX_TEST=1 ./scripts/run-qemu.sh` on Linux Mint/QEMU and supplied the serial log:
- Kernel reached `P3.1 RX test ready`.
- Input `hello exhiOS` was echoed back exactly.
- Enter produced a new `RX>` prompt.
- Ctrl+D ended the loop and printed `received input bytes: 0x000000000000000D` (13 bytes including the Enter terminator).
This proves actual UART receive, echo and control-character handling in local QEMU, not just driver compilation. The P3.1 driver/build/QEMU regression was already green in [run 42270059656](https://github.com/mindwork64/exhiOS/actions/runs/42270059656). Backspace remains implemented but was not clearly exercised by this supplied log.

## 2026-11-06 — P3.2 initial GOP framebuffer console

Commits in this implementation:
- [d8e2b21](https://github.com/mindwork64/exhiOS/commit/d8e2b219bed087e5bf92ab74bfa1c7c5bc27aa62) — created the `exhi-console` no_std crate.
- [4fd68df](https://github.com/mindwork64/exhiOS/commit/4fd68dfe2ebc1d6a83cc8d332698b2962eeee8dd) — added the allocation-free renderer, ASCII glyphs, RGB/BGR format packing, stride-aware writes, wrapping/scrolling and host tests.
- [a90d01e](https://github.com/mindwork64/exhiOS/commit/a90d01ed8ef1a01272584b5f4424d5169811d938) — kernel-side GOP range validation and framebuffer mapping.
- [84eb361](https://github.com/mindwork64/exhiOS/commit/84eb361133f2f1ec76c26f513a372dda0d3480e6), [2596970](https://github.com/mindwork64/exhiOS/commit/25969701b1d8580a6c07101306cf43292ab8d84c), [c717853](https://github.com/mindwork64/exhiOS/commit/c717853e9832942f1f63581ada6823c2fae59f99), [13ac6ff](https://github.com/mindwork64/exhiOS/commit/13ac6fff8532742974c30c7953768c4d6a658237), [b2ebda1](https://github.com/mindwork64/exhiOS/commit/b2ebda1b89ba223bc40c1741976d932f7a774f7d) and [e049446](https://github.com/mindwork64/exhiOS/commit/e04944679dac9de21f2c4038275617f6db494a57) — workspace wiring, kernel startup display and CI host-test/build integration.
- [527dfc3](https://github.com/mindwork64/exhiOS/commit/527dfc3d7802cab3d24918cf2f33a1ad5f19a8d9), [e3e0021](https://github.com/mindwork64/exhiOS/commit/e3e002197f22777390c0b0ba0e9076c79dbcd638), [2248e42](https://github.com/mindwork64/exhiOS/commit/2248e423c1d6ad7074d09a23cf1618e3ce32e9ee) and [2cffee5](https://github.com/mindwork64/exhiOS/commit/2cffee513eda6c63c6cc1d0bd8ee9be0a14c1be9) — fixes to test crate setup and RGB/BGR pixel-position expectations.

The console currently supports GOP format 0 (RGB-reserved) and 1 (BGR-reserved), uses the firmware stride rather than assuming stride equals visible width, validates the framebuffer byte range against the current 4 GiB identity map and writes pixels with volatile stores. The first boot screen reports the kernel banner, resolution, stride and pixel format. BitMask/BltOnly modes and cache policy are not implemented in this first slice.

Host tests, all kernel build variants and headless QEMU/OVMF runtime passed in [run 42272107662](https://github.com/mindwork64/exhiOS/actions/runs/42272107662). The serial trace printed `P3.2: framebuffer console initialized; GOP format and stride validated`, then confirmed P2.6 Double Fault/IST1 and debug-exit status 33. The user's local Linux Mint/QEMU serial log (2026-11-06) now also confirms framebuffer address 0x80000000, resolution 1280x800, stride 1280, GOP format 1, successful console initialization, kernel handoff and halt loop. The screenshot was shared as `https://media.exhi.ru/f/a9cba69c-6d5d-4530-9282-cf2a667cadc7.png`, but it could not be fetched from the assistant runtime because of DNS/network resolution failure; rendered text and layout still require visual inspection.


## 2026-11-06 — P3.2 framebuffer output confirmed by user

The user supplied a description of the screenshot captured from local Linux Mint/QEMU. It identifies the visible screen text:
- `EXHIOS KERNEL 0.1`
- `FRAMEBUFFER CONSOLE READY`
- `RESOLUTION: 1280 X 800`
- `SCANLINE STRIDE: 1280`
- `PIXEL FORMAT: BGRX`
- `COM1 SERIAL: READY`
- `KERNEL HANDOFF: SUCCESS`

Combined with the user's serial log showing GOP address 0x80000000, 1280x800, stride 1280, format 1, successful framebuffer initialization, kernel handoff and halt loop, this confirms that the initial framebuffer console rendered the expected content in local QEMU. The screenshot URL itself could not be fetched by the assistant runtime, so fine visual details such as glyph sharpness, spacing and artifacts were not independently inspected. P3.2's initial functional acceptance is complete with local evidence and CI [run 42272107662](https://github.com/mindwork64/exhiOS/actions/runs/42272107662).


## 2026-11-06 — Unified KernelConsole writer implemented

Commits:
- [c3d2b8c](https://github.com/mindwork64/exhiOS/commit/c3d2b8ccaa34c6b399b7f12905ae11658455916b) — added `exhi-kernel/src/console.rs`, an allocation-free writer that mirrors byte slices and `core::fmt` output to COM1 and the optional GOP framebuffer.
- [558b394](https://github.com/mindwork64/exhiOS/commit/558b394f8b5d8dc66d3cd565e21302bbffde049a) — constructs the writer after GOP setup and routes kernel handoff, unified-writer readiness, QEMU debug-exit and final halt messages through it.

The writer preserves the early-boot ordering: diagnostics before framebuffer setup remain COM1-only. If GOP initialization fails, the unified writer still outputs through COM1. `write_fmt` uses `core::fmt::write` and does not allocate. It remains a single-core, polling-only writer; calls must be serialized and interrupts stay disabled.

CI workflow [42274252754](https://github.com/mindwork64/exhiOS/actions/runs/42274252754): the build job passed all host tests and all kernel build variants. At the time of this log entry, the separate headless QEMU runtime job is still in progress. Local QEMU verification of the mirrored writer output remains pending.


## 2026-11-06 — P3.3 initial PIT timer and monotonic tick interface

Commits:
- [7fc67cc](https://github.com/mindwork64/exhiOS/commit/7fc67cc053b95eeae57beff499228ae5ad41329b) — added the PIT channel 0 timer module and 64-bit monotonic counter.
- [9463bbe](https://github.com/mindwork64/exhiOS/commit/9463bbe10df149ca3fb68fbf3e338b3b91c12822) — added PIC IRQ0 mask/unmask operations while preserving other PIC lines.
- [bb1bdad](https://github.com/mindwork64/exhiOS/commit/bb1bdad91546bb74f5ba5ba95437af45825cc0d1) — installed and validates IDT IRQ0 gate at vector 0x20.
- [a4146d2](https://github.com/mindwork64/exhiOS/commit/a4146d2a30ab72cf51f43bcd12f9e02be0f49aa8) — added opt-in five-tick runtime smoke test.
- [d2ed6b2](https://github.com/mindwork64/exhiOS/commit/d2ed6b2d40260340396af1db8284b607eba1c4ad) — final integration of build/run script and CI coverage.

Implementation:
- PIT channel 0 is programmed in mode 2 with requested rate 100 Hz, input 1,193,182 Hz and divisor 11,932 (approximately 99.9985 Hz).
- The IDT installs the timer interrupt gate at vector 0x20 and checks its selector, attributes, IST field and handler address before reporting IDT valid.
- PIC APIs unmask/mask only IRQ0; remaining master lines and all slave lines preserve their masks.
- The IRQ0 stub is assembly-only: it saves RAX, atomically increments the aligned 64-bit `EXHIOS_PIT_TICKS` symbol, sends EOI to master PIC and executes IRETQ. It avoids Rust calls, heap use and SIMD clobbering.
- Public API: `ticks()` and `elapsed_ticks(start)`, with wrapping subtraction for the 64-bit monotonic counter.
- `EXHIOS_TIMER_TEST=1` combined with `EXHIOS_QEMU_EXIT_TEST=1` selects the isolated timer test variant and controlled QEMU shutdown. It deliberately skips the P2.6 guard-fault trigger so the timer can execute; the ordinary debug-exit variant continues to run the full P2.6 guard/#DF/IST1 test.
- CI run [42275788072](https://github.com/mindwork64/exhiOS/actions/runs/42275788072) passed all host tests, all kernel build variants, the standard headless P2.6 runtime test and the second PIT IRQ0 timer runtime test. Timer output confirmed five delivered ticks, IRQ0 remasked, and debug-exit status 33.
- User-local Linux Mint/QEMU validation of P3.3 remains pending.


## 2026-11-07 — P3.5 local acceptance and self-hosted CI

- User supplied the P3.5 QEMU serial log: exactly three registered drivers, one Initialized (COM1) and zero Failed; PIT and PS/2 stay Registered in registry-only mode.
- The dedicated registry smoke test printed its pass marker and QEMU exited with expected status 33. P3.5 is now closed locally and in CI.
- Main CI moved from GitHub-hosted ubuntu-latest to the user's self-hosted Linux x64 runner. Per-job apt installation was removed; preflight checks verify installed dependencies. ELF inspection uses LC_ALL=C to avoid host locale differences.
- Self-hosted run [42301446350](https://github.com/mindwork64/exhiOS/actions/runs/42301446350) passed host tests/build variants and headless standard, PIT and registry QEMU tests.

## 2026-11-07 — P4.1 task model and scheduler core

- Added the host-testable no_std `exhi-task` crate.
- Added monotonic TaskId allocation, fixed-capacity task table, opaque saved-RSP contexts and Ready/Running/Blocked/Finished lifecycle.
- Added round-robin cooperative yield, block/wake, finish and reap policy. This slice does not yet allocate task stacks or switch CPU context; those belong to P4.2.
- Self-hosted CI passed all 11 task-model tests, every existing kernel build variant and headless QEMU standard/PIT/registry regressions in [run 42302307480](https://github.com/mindwork64/exhiOS/actions/runs/42302307480). P4.1 host-side scheduler policy is accepted.


## 2026-11-07 — P4.1 local host-test acceptance

- User ran `cargo fmt` and `cargo test` after updating to main.
- All 57 host tests passed: ABI 2, console 5, driver 8, ELF 12, heap 4, input 6, memory 9 and task scheduler 11.

## 2026-11-07 — P4.2 x86-64 cooperative context switching

- Added a feature-gated x86-64 task runtime smoke module with two static kernel stacks. Each stack has a 4 KiB unmapped guard page and 16 KiB usable space.
- Added a SysV-aligned initial restore frame and assembly context switch saving RSP plus RBP/RBX/R12–R15. R12 passes the task entry function to the assembly bootstrap.
- Added a task-return path: Finished tasks are switched away from; the last task restores the boot context. TSS.RSP0 tracks the selected task stack and is restored when returning to boot. The existing IST1 path remains independent.
- QEMU runtime test runs two tasks in deterministic A/B alternation; each verifies stack-local state after yields, records a different local-stack address, then exits. Both scheduler entries must be Finished before boot resumes.
- Self-hosted CI [run 42304100157](https://github.com/mindwork64/exhiOS/actions/runs/42304100157) passed all build variants and standard/PIT/registry/P4.2 QEMU runtime tests. The P4.2 trace proves A→B→A→B→A→B, preserved locals, Finished=2, TSS.RSP0 restored and debug-exit 33.
- Local Linux Mint/QEMU confirmation is pending; do not close P4.2 until the user supplies this run's serial output.


## 2026-11-06 — P4.2 local QEMU acceptance

- User supplied the full Linux Mint/QEMU serial log. The P4.2 trace shows task A and task B entering on independent stacks and alternating A→B→A→B→A→B.
- Both task progress counters reached 3; task-local stack addresses were distinct (A 0x17CF38, B 0x181F38 in this run).
- Scheduler reported Finished=2 and TSS.RSP0 restored=yes; the runtime pass marker was printed and QEMU exited with status 33.
- P4.2 is now accepted locally and in self-hosted CI [run 42304100157](https://github.com/mindwork64/exhiOS/actions/runs/42304100157).

## 2026-11-07 — P4.3 PIT-backed cooperative task runtime accounting (implementation)

- Extended `exhi-task::Task` with a per-task saturating `runtime_ticks` counter and added `Scheduler::account_current_ticks()`.
- Added three host tests for independent per-task accounting, saturation and invalid scheduler state. The task crate suite is now expected to contain 14 tests.
- Added a feature-gated kernel runtime test: two guarded Ring 0 tasks wait on actual PIT IRQ0 ticks, alternate cooperatively and account elapsed ticks at yield/finish boundaries.
- IRQ0 assembly remains unchanged and does not call Rust or the scheduler. The P4.3 path clears IF across accounting and call-based stack switching, while tasks enable interrupts during `sti; hlt` waits.
- Added `EXHIOS_TASK_ACCOUNTING_TEST=1` to the build/QEMU scripts and the self-hosted CI workflow, including a dedicated headless QEMU test.
- Code commits: [5bc193b](https://github.com/mindwork64/exhiOS/commit/5bc193be27e7f309b4a2b60fa775b274c2ded7ba), [cca09ab](https://github.com/mindwork64/exhiOS/commit/cca09ab4fe94f4374dd0b620cbab475aa165680c), [d55f2c2](https://github.com/mindwork64/exhiOS/commit/d55f2c201e45a2a159ede57c85f6e6f621dc9d08), [7fa7a10](https://github.com/mindwork64/exhiOS/commit/7fa7a10e1a9312e2038eaee016db4d6c7d62cc0e).
- Corrected self-hosted CI [run 42306726315](https://github.com/mindwork64/exhiOS/actions/runs/42306726315) passed all 14 task tests, all kernel build variants, all QEMU regressions and the actual P4.3 runtime test. Its serial trace shows A/B each completing three PIT intervals, elapsed=0x12, runtime A=0x9/B=0x9, Finished=2, IRQ0 masked, TSS.RSP0 restored, P4.3 pass marker and debug-exit 33.
- An earlier queued runtime run only reached the P2.6 guard/#DF probe, so exit 33 did not prove P4.3. Commit [49191dd](https://github.com/mindwork64/exhiOS/commit/49191dd3421011d0ea4460c5b5f77748904a2fe7) corrected the P2.6 cfg exclusion for task-accounting mode; run 42306726315 is the valid P4.3 result.
- User-local Linux Mint/QEMU (2026-11-07): supplied serial log confirms 18 PIT ticks, runtime A=9/B=9, Finished=2, IRQ0 remasked, TSS.RSP0 restored, P4.3 pass marker and debug-exit 33. P4.3 is closed locally and in CI.


## 2026-11-07 — P4.4 timer-driven preemption: implementation and CI runtime

- Added a dedicated preemption interrupt path for the feature-gated P4.4 test. The ordinary P3.3 IRQ0 handler remains unchanged.
- The initial implementation used an incomplete 18-qword frame. Three QEMU CI runs all reached the same #PF immediately on the initial IRETQ: error code 0, RIP 0x10AE7D, CR2 0x13C000. The missing architectural slots were saved RSP and SS.
- In 64-bit mode the processor saves SS:RSP on every interrupt frame, including same-CPL Ring 0 interrupts; IRETQ consumes those slots on return. The corrected `InterruptContext` is 20 qwords (15 GPR + RIP/CS/RFLAGS/RSP/SS), and the synthetic first-task frame supplies task stack top and kernel data selector 0x10.
- Added a distinct public kernel data selector constant for the synthetic interrupt frame.
- The IRQ0 preemption stub saves all general-purpose registers, saves per-task x87/SSE state with FXSAVE64, increments PIT ticks, sends PIC EOI, invokes the Rust scheduling/accounting handler, restores the selected task's FX state and returns through IRETQ. The handler validates stack-frame bounds and updates TSS.RSP0 before switching.
- The two test tasks busy-loop without cooperative yield. Runtime acceptance checks timer switches, per-task PIT runtime, distinct stack-local storage, XMM15 state preservation, scheduler state, active TSS.RSP0 and IRQ0 masking.
- Fatal CPU exceptions in this opt-in QEMU test now emit the failure debug-exit value so a regression fails promptly instead of hanging in the kernel HLT loop.
- Fix commit: [b0365e3](https://github.com/mindwork64/exhiOS/commit/b0365e30acbeb40143913299cb25a2d072e3546a).
- Self-hosted CI [run 42334341610](https://github.com/mindwork64/exhiOS/actions/runs/42334341610) passed both build and QEMU runtime jobs, including standard boot, PIT, registry, P4.2, P4.3 and the actual P4.4 test. P4.4 serial: elapsed=0x12 PIT ticks, switches=0x12, runtime A=0x9/B=0x9, XMM preserved for A and B, stacks distinct=yes, TSS.RSP0 active=yes, IRQ0 masked=yes, pass marker and debug-exit 33.
- Local Linux Mint/QEMU acceptance is pending. P4.4 remains Ring 0, single-CPU and PIT/legacy-PIC only; Ring 3 and syscalls remain in P5.

## 2026-11-07 — P4.4 local Linux Mint/QEMU acceptance

- User supplied the complete local serial log for `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_TIMER_PREEMPTION_TEST=1 ./scripts/run-qemu.sh`.
- Runtime values: elapsed PIT ticks=0x12, timer preemption switches=0x12, task A runtime=0x9, task B runtime=0x9.
- Both XMM sentinels were preserved, task stack locals had distinct addresses, TSS.RSP0 remained active for the selected task, IRQ0 was masked at shutdown.
- `P4.4: timer-driven preemption runtime smoke test passed.` and QEMU debug-exit status 33. P4.4 is accepted locally and in self-hosted CI.

## 2026-11-07 — P4.5 preemptive task termination: implementation and CI runtime

- Added `preemptive-task-exit-smoke-test`, depending on the existing timer-preemption implementation. Added the separate `EXHIOS_PREEMPTIVE_TASK_EXIT_TEST=1` build/QEMU selector, with mutual-exclusion checks and a bounded QEMU timeout. Existing P4.4 remains a separate regression.
- Added a returning task-entry bootstrap. When an entry returns, assembly calls `exhi_preemptive_task_finished` on the task's own stack. The Rust path clears IF, accounts the task's final PIT interval, calls `Scheduler::finish_current()`, and either dispatches the selected Ready task using its validated interrupt frame or finishes the test when no task remains.
- When a Ready peer exists, the path updates TSS.RSP0, the next task's runtime baseline and active FXSAVE area, then restores that task via the common interrupt-context entry. The final task clears TSS.RSP0 and active FX pointer, masks IRQ0 and reports controlled debug-exit.
- Added an independent runtime smoke test: task A returns first; task B resumes and runs alone before returning. The test checks both Finished states, no current/Ready/Blocked tasks, task runtime, saved frame bounds, XMM state, distinct stack locals, TSS cleanup and IRQ0 mask. Static task stacks are intentionally not freed yet.
- Code commits: [dd10f58](https://github.com/mindwork64/exhiOS/commit/dd10f5882ddb3085595b4d6eef45474b0649e56d), [377e73c](https://github.com/mindwork64/exhiOS/commit/377e73c20e2928116d04cd3141ddbc36db9df89a). The second commit fixes linker visibility of the task-exit trampolines in the timer-preemption test build.
- Self-hosted CI [run 42335136555](https://github.com/mindwork64/exhiOS/actions/runs/42335136555) passed all host tests/build variants and the complete QEMU runtime suite, including prior P4.4. P4.5 serial: elapsed=0x18 PIT ticks, switches=0x10, runtime A=0x8/B=0xA; both tasks Finished; XMM preserved; stacks distinct; scheduler current=none/Finished=2; TSS.RSP0 cleared; IRQ0 masked; P4.5 pass marker; debug-exit 33.
- Local Linux Mint/QEMU acceptance is pending. Idle-context dispatch, blocked-task wake-up and stack reclamation are not yet part of this slice.

## 2026-11-07 — P4.5 local Linux Mint/QEMU acceptance

- User supplied the complete local QEMU serial log for `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_PREEMPTIVE_TASK_EXIT_TEST=1 ./scripts/run-qemu.sh`.
- Runtime values: elapsed PIT ticks=0x18, preemption switches=0x10, task A runtime=0x8, task B runtime=0x10. Both tasks reported Finished=yes; XMM state A/B=preserved; task stacks distinct=yes.
- Scheduler state: current=none, Finished=2; TSS.RSP0 cleared=yes; IRQ0 masked=yes. The P4.5 pass marker appeared and QEMU debug-exit was confirmed with status 33.
- P4.5 is now accepted both locally and in self-hosted CI [run 42335136555](https://github.com/mindwork64/exhiOS/actions/runs/42335136555). Static task stacks are still intentionally unreclaimed.
- Next stage: P4.6 idle-context dispatch and timer-driven wake-up of blocked tasks, with a separate interrupt-format idle frame and no mixing with cooperative P4.2 contexts.

## 2026-11-07 — P4.6 idle context and blocked-task wake-up accepted

- Added explicit idle state to the fixed-capacity scheduler and timed-block API with wrap-safe PIT deadline wake-up. Host tests cover idle transition, deadline handling, tick-counter wraparound and invalid state transitions.
- Added a dedicated guarded idle stack and 20-qword IRETQ frame, separate from cooperative saved-RSP contexts. DPL0 `INT 0x30` saves and blocks the current task; idle executes `sti; hlt`.
- IRQ0 now handles idle-frame save, due-task wake-up and dispatch through the selected task's saved interrupt frame, restoring task FXSAVE and TSS.RSP0. Returning tasks re-enter idle while blocked peers remain.
- P4.6 QEMU runtime test runs two tasks with staggered wake deadlines and verifies idle entry/ticks, wake count, local stack data, XMM state, both Finished states, empty runnable/blocked sets, TSS cleanup, IRQ0 mask and QEMU debug-exit.
- Self-hosted CI [run 42337327289](https://github.com/mindwork64/exhiOS/actions/runs/42337327289) passed the build job and the complete QEMU runtime job, including all prior scheduler regressions and the P4.6 runtime test. Per user instruction, this runner result is accepted as local-equivalent evidence.
- P4.6 is closed. Static task stack reclamation remains out of scope.

## 2026-11-07 — P5.1 Ring 3 entry implementation started

- Added GDT DPL3 user descriptors while retaining kernel code/data and TSS selectors; user selectors are 0x33 (code) and 0x2B (data).
- Added `ring3-smoke-test` with an address space containing one RX user code page and one RW+NX user stack page, separated by an unmapped guard page. A dedicated guarded kernel stack is installed in TSS.RSP0.
- Constructed a 20-qword IRETQ frame to enter CPL3, where a one-byte INT3 instruction should raise #BP. The CPL3 exception verifier checks saved user CS/RIP/RFLAGS/RSP/SS and the effective user page permissions; cleanup restores kernel CR3 and reclaims the private address space and user frames.
- Added `EXHIOS_RING3_TEST=1` build/QEMU selectors and dedicated build/runtime CI steps.
- P5.1 remains implementation-in-progress until the current-HEAD self-hosted QEMU runtime job passes.

## 2026-11-08 — P5.1–P5.7 user-mode isolation accepted

- Production x86-64 user-entry and user-memory primitives were promoted into always-built architecture modules. Ring 3 entry validates active CR3, RX code, RW+NX user stack, supervisor kernel stack, TSS.RSP0, FX state and the full exception-return frame.
- Common SYSCALL entry uses one normalized 20-qword frame and a 64-byte-aligned BSP GS-relative context. The entry switches away from user RSP, saves/restores FXSAVE state and invokes a kernel dispatcher before IRETQ return.
- `exhi-syscall` owns syscall-number/capability mapping and default-deny authorization. The adapter cannot override the authorization decision. User pointers are validated across all pages; copy helpers validate a whole range before mutation.
- The P5.6 runtime executes two isolated Ring 3 tasks with separate CR3/code/stack contexts, syscall yield/exit, idle dispatch and kernel reaper. It checks authorized ConsoleWrite, denied Yield/ConsoleWrite without side effects, frame/page-table reclamation and task-record removal.
- Host/build plus all 20 headless QEMU scenarios passed in [CI #405](https://github.com/mindwork64/exhiOS/actions/runs/42434078573). Accepted scope: BSP/single-CPU; no AP-local GS, user TLS, SWAPGS-window NMI handling or SMP claim.

## 2026-11-08 — P6 VFS/RAMFS accepted

- Added allocator-free `FileSystem` contract and bounded fixed-capacity `RamFs` implementation in `exhi-vfs`, with generation-tagged handles, nested paths, file/directory CRUD, paginated entries, sparse zero-fill and explicit overflow/capacity/stale-handle errors.
- Kernel smoke test exercises `/etc/exhi.conf`, binary write/read, absolute/relative lookup, directory enumeration, no-mutation on oversized write, non-empty directory protection and stale slot generation.
- All eight `exhi-vfs` host tests and the kernel build/QEMU runtime passed in [CI #412](https://github.com/mindwork64/exhiOS/actions/runs/42546613316). Userland filesystem syscalls and persistent storage remain future work.

## 2026-11-09 — P7 userland progress and latest green CI

- P7.1: bounded ET_EXEC loader enters the embedded `init` in Ring 3 with private address space, RX/RW+NX mappings, zeroed BSS, four-page user stack and unmapped guard. Exit and processor-originated user exceptions restore kernel CR3 and reclaim owned user frames/page tables.
- P7.2: `libexhi` exposes `console_write`, non-blocking `console_read`, `yield_now` and `exit` wrappers over the shared stable syscall ABI. CI #435 passed host and QEMU syscall sequence tests.
- P7.3: interactive serial shell runs in Ring 3, supports bounded line editing and commands `help/hostname/version/echo/clear/config/exit`; CI #447 passed serial-socket interaction.
- P7.4: bounded allocation-free `exhi-config` parser and embedded hostname/prompt/version settings; invalid/duplicate known entries safely fall back to defaults. CI #447 passed parser and QEMU shell checks. Persistent VFS config loading remains open.
- The remaining P7.1 blocker is production scheduler integration: the current `InitServices::yield_current` returns `Ok(None)`, so init resumes itself; the general `exhi-task` scheduler is not yet the production process scheduler. The P5.6 two-task scheduler path remains an architecture smoke adapter.
- User-mode fault termination cleanup was hardened in [e1d8999](https://github.com/mindwork64/exhiOS/commit/e1d8999792ff10d68dd3f188816f32b05150ced1); diagnostics were fixed to emit CRLF in [bd52245](https://github.com/mindwork64/exhiOS/commit/bd522457884c3d8536c1cd2b487fd8bc07179954); rustfmt followed in [3943378](https://github.com/mindwork64/exhiOS/commit/39433785d5a116fa1e6908bf1f91b806bd931c48).
- Current HEAD [3943378](https://github.com/mindwork64/exhiOS/commit/39433785d5a116fa1e6908bf1f91b806bd931c48) passed both self-hosted CI jobs in [run #451](https://github.com/mindwork64/exhiOS/actions/runs/42580983148): full build/host tests and the headless QEMU runtime suite including user-fault cleanup. Intermediate runs #449/#450 failed; #451 is the successful current-HEAD result.
- Documentation checkpoint: all core wiki pages are being synchronized to this code state. Wiki updates are part of every milestone, not optional cleanup.


## 2026-11-09 — P7.1 kernel-stack tracking across user-task switches

- The common user-entry validator held global active kernel-stack bounds written only by the initial `prepare_entry`. The P5.6 scheduler already changed CR3, TSS.RSP0 and the syscall entry stack/FX pointers, but the exception-frame validator could still be checking the first task's old stack bounds after a user-task switch.
- Added `user::activate_kernel_stack(address_space, bottom, top)`: requires IF=0 and an already active address space; validates canonical/aligned supervisor stack bounds and the complete mapping; sets and reads back TSS.RSP0; updates and verifies the active exception-frame bounds.
- `prepare_entry` now uses this shared helper. The P5.6 user-task selector calls it after activating the chosen user's CR3, so the fault-frame validator follows the selected task. `UserEntryFrame` fields are crate-visible for the upcoming architecture-owned context construction.
- Commit: [b80c4c2](https://github.com/mindwork64/exhiOS/commit/b80c4c2802434ccb150927f8ee1cda2d3193ef40).
- CI [#453](https://github.com/mindwork64/exhiOS/actions/runs/42592065020) passed the complete build/host-test job and all 21 headless QEMU runtime scenarios, including first-process loader, init syscall path, interactive shell, user-fault cleanup, P5.1–P5.6 and the final two-user-task lifecycle/reaper test.
- This closes the active exception-stack tracking gap in the existing smoke scheduler; it does **not** close P7.1. Production `process-runtime` still has one init process and `Yield` returns `Ok(None)`. General scheduler integration and a dedicated production multi-process QEMU test remain next.

## 2026-11-09 — Production P7.1 scheduler/reaper integration and warning-free feature matrix

- Production `process-runtime` now instantiates `exhi-task::Scheduler`, registers the init process as a user task and registers a blocked `kernel-reaper` task. The reaper owns a dedicated guarded supervisor stack and its own FXSAVE image.
- `Yield` delegates to the scheduler and returns the selected task context when the scheduler chooses a peer. With only one user process registered, the current result is `Continue`; this is expected and is not evidence of multi-process dispatch.
- `Exit` marks init Finished, wakes the blocked reaper, dispatches it from idle and switches away before destroying the address space. The reaper restores kernel CR3, destroys the LoadedProcess, releases owned user frames/page tables and removes the Finished task record.
- CPL3 fault termination follows the same deferred-reclamation path. IDT entry does not automatically execute `SWAPGS`; the validated user-fault handler explicitly switches to the trusted kernel GS base before updating the GS-relative syscall CPU-local context, then finishes init and schedules the reaper. This fixed the QEMU fault path.
- CI [#484](https://github.com/mindwork64/exhiOS/actions/runs/42597997791), HEAD [8f0b4bf](https://github.com/mindwork64/exhiOS/commit/8f0b4bf5f854b06b2f4d9452b408baf5a0a719d9): build job passed all 50 steps with zero Rust warning lines; QEMU job passed all 27 job steps, including 21 runtime scenarios. The production user-fault scenario observed #UD vector 6, the reaper start marker, full init address-space/frame/task-record reclamation and expected debug-exit status 33. Normal init Exit and interactive shell reaping also passed.
- Feature-specific module/API gates were narrowed to the smoke/runtime variants that actually use them. This removes feature-matrix `dead_code` noise without suppressing warnings; the final full build log contains no `warning:` lines.
- P7.1 remains open for production multi-process support. Current globals/service adapters represent one init process; no spawn/registry for a second production user process exists yet. The separate P5.6 two-user-task runtime remains a scheduler architecture test.


## 2026-11-10 — P7 dynamic worker creation and shell coverage

- Added stable `SpawnWorker = 6` and `TaskSpawn = 11`. The shared dispatcher denies spawn before entering the service unless the active process holds the grant.
- Added `libexhi::spawn_worker()` and the init shell command `spawn`. Init alone receives `TaskSpawn`; the worker receives only yield, exit and console-write capabilities.
- Kernel worker creation loads the embedded ELF into a free resource slot, creates its private address space, installs and records its guarded kernel stack, initializes FXSAVE, registers the task and restores the caller's process context before returning.
- Process registry slot selection is aligned with the matching static resource slot. Stack guard installation is tracked per slot to avoid duplicate installation on retry.
- Updated the build script to build both ELF targets and extended the serial shell driver to verify worker execution and final reaper cleanup of both process records.
- Updated the P7 roadmap and architecture pages. This was a code-only update: no GitHub Actions, QEMU, Cargo build/tests or Clippy were run. Local Rust formatting tools were unavailable.


## 2026-11-10 — P7 dynamic worker creation: local lint checkpoint

- User ran `cargo fmt` after syncing the P7 dynamic-worker implementation.
- User built the two embedded userland images successfully with `cargo build -p exhi-init --bin exhi-init --bin exhi-worker --target x86_64-unknown-none`.
- Clippy completed successfully for `cargo clippy --workspace --lib --bins --target x86_64-unknown-none`, the production `process-runtime` feature, `process-shell-smoke-test`, and `process-multi-smoke-test`.
- The feature-specific multi-process Clippy pass initially reported one `dead_code` warning for `run_init`, because that feature enters through `run_process_pair`. The entry point was cfg-gated out of the process-pair build; the user reran Clippy and confirmed that warning disappeared.
- Clippy then completed successfully for `process-fault-smoke-test`, `process-run-smoke-test` and `process-load-smoke-test` as well. The six P7 kernel feature configurations and the common workspace library/binary targets now all pass Clippy. Remaining warnings are pre-existing suggestions in allocator, ELF and baseline architecture modules, not the new P7 process-runtime path.
- No unit tests, GitHub Actions or QEMU runs were performed. P7 runtime acceptance remains pending.


## 2026-11-10 — P7 worker fault recovery coverage added (runtime pending)
- User verified the dedicated fault-mode userland build, userland Clippy, and kernel `process-worker-fault-smoke-test` Clippy. All completed successfully; kernel warnings remain limited to legacy modules. The runtime interaction driver was not run.

- Extended the normal shell QEMU driver to issue a second `spawn` after the first worker exits; it requires a clean "could not be started" result while the Finished worker still occupies its slot, then verifies normal init exit/reaper cleanup.


- Added the `process-worker-fault-smoke-test` kernel feature and `worker-fault-smoke-test` userland build feature. In this mode the embedded worker emits its startup marker and executes `UD2` from CPL3.
- The user-fault handoff records the fault vector only when the current scheduler task is named `worker`; the shared scheduler fault path must select the still-Ready init task instead of waking the reaper prematurely.
- Added a reaper acceptance condition requiring vector 6, both process records reclaimed, and an empty registry. A serial driver waits for the fault, issues `hostname` to prove init resumed, then sends `exit` and waits for final cleanup.
- Added `EXHIOS_PROCESS_WORKER_FAULT_TEST=1` support to the build and QEMU runner scripts. Code-only addition; Clippy is pending and no QEMU or CI job was run.

## 2026-11-10 — Worker fault recovery: GS transition fix and lint pass

- Static review found a context-sensitive GS-base bug in the worker-fault handoff. The user exception path enters trusted kernel GS with `SWAPGS`; when the scheduler selects a still-Ready init process, the return path must restore the user GS layout before `IRETQ`, otherwise the resumed process's next `SYSCALL` would swap the wrong bases.
- Updated the alternate-process handoff to execute `SWAPGS` before `user::enter_user`. If that transfer unexpectedly returns or rejects the entry frame, the code swaps back to trusted kernel GS before the fatal error path. The reaper handoff remains in kernel GS.
- User synced [9b63adf](https://github.com/mindwork64/exhiOS/commit/9b63adf625512e2429110a1c89f0ed5d06060dcc), ran `cargo fmt`, and successfully ran:
  `cargo clippy -p exhi-kernel --bin exhi-kernel --features process-worker-fault-smoke-test --target x86_64-unknown-none`.
- Clippy emitted 20 existing warnings in legacy modules (`manual_is_multiple_of`, `new_without_default`, and `collapsible_if` suggestions); there were no new warnings in the process-runtime or worker-fault code.
- No Cargo tests/build, QEMU runtime, or GitHub Actions/CI were run for this checkpoint. The earlier userland build and Clippy checks remain as previously recorded. P7 acceptance still requires runtime evidence for normal spawn/rejection/reaping and worker-fault recovery.

## 2026-11-10 — Explicit TaskSpawn denial check in the worker

- Added an adversarial capability check to the embedded worker: it calls `libexhi::spawn_worker()` without the `TaskSpawn` grant and requires the exact `SyscallError::AccessDenied` result. Any success or different error prints an explicit failure marker and exits.
- The check runs in both normal worker mode (before the cooperative yield loop) and worker-fault mode (before CPL3 `UD2`). It therefore validates the grant boundary independently of the worker's termination mode.
- Updated the normal shell and worker-fault serial drivers to require the success marker. A generic `Io` result due to the two occupied process slots does not satisfy the assertion.
- Code-only update. User-side Clippy for `exhi-init --bins` in the normal configuration and with `worker-fault-smoke-test` is pending. No build, tests, QEMU, or CI were run.
