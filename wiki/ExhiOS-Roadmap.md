# ExhiOS — дорожная карта

## Приёмка
Этап считается завершённым только после требуемого runtime evidence. CI/build success недостаточно.

## P0 — UEFI boot chain
**Завершено.** UEFI entry, ELF64 validation, PT_LOAD load, zero-fill, GOP/ACPI, final memory map, ExitBootServices и kernel handoff проверены в QEMU.

## P1 — Kernel foundation
**Завершено.**
- P1.1 GDT; CS/DS/ES/SS подтверждены.
- P1.2 IDT и exceptions 0–31; INT3 дошёл до handler.
- P1.3 PIC remap/masks и QEMU debug-exit status 33.

## P2 — Memory management

### P2.1 Physical page allocator
**Проверено в QEMU.** Descriptor version 1, stride 48; Conventional Memory only; первые 1 MiB зарезервированы; limit 4 GiB; managed/allocated bitmaps; allocate/free/reuse. 7 host tests прошли. QEMU показал 118 837 managed pages и повторно выдал frame 0x14C000.

### P2.2 Bootstrap page tables
**Завершено; runtime проверено в headless QEMU/OVMF CI.**
- [x] Ревью и безопасная схема bootstrap storage.
- [x] Экспортировать модуль и вызвать после P2.1 при IF=0.
- [x] Создать PML4 → PDPT → 4 PD в выровненном kernel .bss.
- [x] Identity map 0–4 GiB через 2 MiB pages.
- [x] Переключить CR3; проверить критические ranges до и после.
- [x] QEMU serial smoke test: root=CR3=0x14E000, 9 проверенных ranges, kernel entry translation 0x100000, debug-exit status 33.
- Evidence: [GitHub Actions run 42205457670](https://github.com/mindwork64/exhiOS/actions/runs/42205457670).

### P2.3 Virtual memory manager
**Завершено и подтверждено CI + локальным Linux Mint/QEMU 2026-11-06.**
- [x] 4 KiB `map_page`, `unmap_page`, `query_page`.
- [x] Разбиение 2 MiB huge page на 512 4 KiB entries.
- [x] Права writable/read-only, user/supervisor, executable/NX; включение NXE только после CPUID.
- [x] TLB invalidation через `invlpg`.
- [x] Безопасное coalesce 512 совместимых contiguous PTE в 2 MiB PDE.
- [x] Освобождение page-table frame после замены PDE и инвалидирования 512 адресов.
- [x] QEMU проверил RW supervisor NX mapping, read/write, unmap/NotMapped, восстановление identity map и reclaim; debug-exit 33.
- [x] Локальный прогон: NX enabled, 2 MiB→4 KiB split, запись/чтение, unmap/TLB invalidation, identity restore/reclaim; debug-exit 33.
- [ ] Отдельные address spaces с собственными PML4 roots и kernel mapping policy.
- [ ] Физические адреса за пределами текущего 4 GiB allocator window.

### P2.4 Kernel heap
**Завершено; host tests, headless CI QEMU и локальный Linux Mint/QEMU подтверждены 2026-11-06.**
- [x] Новый no_std crate exhi-heap: first-fit free list, alignment, splitting/coalescing, free/reuse.
- [x] Kernel GlobalAlloc поверх page-backed виртуальной области с динамическим ростом.
- [x] Heap VA начинается с 4 GiB; физические frames остаются ниже 4 GiB; heap mappings RW+NX.
- [x] Host tests: heap 4/4; ABI 2/2, ELF 12/12, physical memory 7/7.
- [x] Startup smoke test: align64, 48 KiB align4096 allocation, cross-page write/read, free/reuse/coalescing.
- [x] Local + CI QEMU: initial 4 pages, вырос до 18 pages (0x12000 bytes), все 0x12000 bytes free after test; debug-exit 33.
- Evidence: [headless QEMU CI run 42215755990](https://github.com/mindwork64/exhiOS/actions/runs/42215755990).

### P2.5 Kernel address spaces
**Первая реализация завершена; headless CI/OVMF и локальный Linux Mint/QEMU подтверждены 2026-11-06.**
- [x] Менеджер адресных пространств и отдельные PML4/PDPT для каждого пространства.
- [x] Общая supervisor-only область ядра: PDPT entries 0..4 (identity map 0..4 GiB + heap); entries 5..7 зарезервированы.
- [x] Private user VA window: 8..512 GiB (PDPT entries 8..511); caller-owned physical pages, map/unmap/query, reclaim empty private tables.
- [x] CR3 switch/restore и destroy inactive address spaces.
- [x] QEMU smoke test: два PML4, одинаковый user VA 0x200000000 ведёт на разные physical frames; общие kernel/heap translations сохраняются; private page tables reclaimed, data frames safely returned.
- [x] Headless CI QEMU: [run 42217123278](https://github.com/mindwork64/exhiOS/actions/runs/42217123278); safety-hardening commit a856090: [run 42217369804](https://github.com/mindwork64/exhiOS/actions/runs/42217369804).
- [x] Локальный Linux Mint/QEMU пользователя (2026-11-06): kernel root 0x11B000; A PML4 0x177000; B PML4 0x179000; shared kernel translation 0x104000; shared heap translation 0x163000; user VA 0x200000000 → A frame 0x17B000 / B frame 0x17C000; reclaim прошёл; debug-exit status 33.
- [ ] Дополнительные lifecycle/error-path и permission-negative tests перед подготовкой Ring 3.
- [ ] Ring 3 execution и hardware-enforced user/supervisor fault tests — отдельная часть user-mode milestone (P5).

Границы текущего результата: smoke test проверяет раздельные mappings, CR3 switching и supervisor-only policy на Ring 0. Он ещё не доказывает выполнение кода в CPL3 или аппаратное блокирование доступа пользователя к supervisor pages.

### P2.5.1 Address-space lifecycle and mapping hardening
**Завершено; headless CI/OVMF и локальный Linux Mint/QEMU подтверждены 2026-11-06.**
- [x] Guard для вызовов после уничтожения AddressSpace: методы не должны разыменовывать освобождённый root; повторный destroy идемпотентен.
- [x] Запрет destroy активного CR3.
- [x] Отказ от writable+executable user mappings (W^X).
- [x] Негативные smoke tests: unaligned/out-of-window virtual address, unaligned physical address, W+X, duplicate mapping, double unmap, use-after-destroy.
- [x] Валидация диапазона и выравнивания физических кадров вложенных page tables перед обходом.
- [x] Двухпроходный teardown: сначала validate всего private дерева, затем освобождение с unlink-after-success и возможностью безопасной повторной попытки.
- [x] CI build + headless QEMU/OVMF runtime: [run 42238977281](https://github.com/mindwork64/exhiOS/actions/runs/42238977281) — build и QEMU runtime jobs прошли.
- [x] Локальный Linux Mint/QEMU пользователя (2026-11-06): все P2.5.1 negative cases завершились без ошибки внутри address-space smoke test; normal address-space test passed; debug-exit status 33.

Кодовый checkpoint P2.5.1: [e19db15](https://github.com/mindwork64/exhiOS/commit/e19db15bf891805c2e22cb3251ebade7872bf515).

### P2.6 Exception and stack hardening
**Завершено; headless CI/OVMF и локальный Linux Mint/QEMU подтверждены 2026-11-06.**
- [x] Kernel 64-bit TSS descriptor in GDT; LTR installs TR; dedicated aligned 16 KiB IST1 stack.
- [x] IDT vector 8 (#DF) gate selects IST1; status readback checks IST index.
- [x] Exception assembly passes the normalized exception-frame pointer to the handler; #DF handler verifies that it lies inside IST1.
- [x] UEFI loader allocates one contiguous 17-page block: one reserved guard page followed by the 16-page (64 KiB) boot kernel stack. BootInfo v2 carries the exact stack bounds.
- [x] Physical allocator reserves the guard frame before later allocations; VMM removes its supervisor identity mapping and confirms the stack pages remain mapped.
- [x] QEMU debug-exit variant switches RSP to the bottom of the mapped stack and faults on the guard page. The CPU's #PF frame push faults into the same guard page, raising #DF on IST1; handler verifies its frame pointer is inside IST1.
- [x] Headless CI QEMU/OVMF run [42256850017](https://github.com/mindwork64/exhiOS/actions/runs/42256850017): loader reports guard 0x1E6FF000, stack 0x1E700000..0x1E710000; guard is unmapped, stack remains supervisor-only, #DF reaches IST1, debug-exit status 33.
- [x] Локальный Linux Mint/QEMU (2026-11-06): guard page 0x1E0D3000 не отображена; stack 0x1E0D4000..0x1E0E4000 (64 KiB) остался supervisor-only; вызванный guard fault привёл к vector 8 (#DF), exception frame 0x122FC8 находится внутри IST1 0x11F000..0x123000; debug-exit status 33.
- [ ] Additional deliberate stack exhaustion/guard-page variants beyond the controlled #DF probe.

Latest code checkpoint: [b46c870](https://github.com/mindwork64/exhiOS/commit/b46c8702e3725281e9a42654524649d1d04b4fe8).

## P3 — Basic devices

### P3.1 — Polling serial driver
**COM1 TX is confirmed locally and in headless QEMU CI; RX API and an interactive diagnostic mode are implemented; actual local RX input validation is pending.**
- [x] Reusable 16550-compatible `SerialPort` type with configurable base port.
- [x] COM1 initialization (38400 baud, 8N1, FIFO), bounded polling TX and fixed-width hexadecimal output.
- [x] Non-blocking `try_read_byte` and polling `read_byte_blocking`.
- [x] Kernel boot diagnostics use `serial::COM1`; direct COM1 port I/O was removed from `main.rs`.
- [x] Local Linux Mint/QEMU boot log (2026-11-06) shows the P3.1 readiness marker and successful COM1 diagnostic output.
- [x] Headless QEMU/OVMF regression: [run 42257896620](https://github.com/mindwork64/exhiOS/actions/runs/42257896620), including the P2.6 guard-page/#DF/IST1 check.
- [x] RX-inclusive driver build + headless QEMU/OVMF regression: [run 42258141119](https://github.com/mindwork64/exhiOS/actions/runs/42258141119) passed; CI does not inject serial input.
- [x] Added opt-in interactive byte-echo test mode: `EXHIOS_SERIAL_RX_TEST=1 ./scripts/run-qemu.sh`; it echoes printable input, handles Enter/Backspace and reports the received byte count on Ctrl+D. The feature build passed in CI run [42268731034](https://github.com/mindwork64/exhiOS/actions/runs/42268731034); local RX runtime remains pending.
- [ ] Local Linux Mint/QEMU test with actual typed input.

### P3.2 — Framebuffer console
- [ ] Pixel-format-aware framebuffer console with GOP stride support.

### P3.3 — Timer
- [ ] Initial timer source and monotonic tick interface.

### P3.4 — Keyboard
- [ ] Keyboard input driver and key event abstraction.

### P3.5 — Driver registration
- [ ] Initial driver registration model.

## P4 — Tasks and scheduling
- [ ] Task representation/stacks, context switching, cooperative scheduler, timer preemption.

## P5 — User-mode isolation
- [ ] Ring 3, per-process address spaces, syscalls, user-pointer validation, capabilities.

## P6 — Storage
- [ ] VFS, RAMFS, file/directory operations.

## P7 — Userland
- [ ] init, libexhi, serial shell, basic utilities.
