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

Host tests, all kernel build variants and headless QEMU/OVMF runtime passed in [run 42272107662](https://github.com/mindwork64/exhiOS/actions/runs/42272107662). The serial trace printed `P3.2: framebuffer console initialized; GOP format and stride validated`, then confirmed P2.6 Double Fault/IST1 and debug-exit status 33. Local visual confirmation remains pending.
