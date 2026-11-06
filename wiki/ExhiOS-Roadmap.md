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
**Завершено; COM1 TX/RX подтверждены локально и в CI.**
- [x] Reusable 16550-compatible `SerialPort` type with configurable base port.
- [x] COM1 initialization (38400 baud, 8N1, FIFO), bounded polling TX and fixed-width hexadecimal output.
- [x] Non-blocking `try_read_byte` and polling `read_byte_blocking`.
- [x] Kernel boot diagnostics use `serial::COM1`; direct COM1 port I/O was removed from `main.rs`.
- [x] Local Linux Mint/QEMU boot log (2026-11-06) confirms COM1 TX.
- [x] Headless QEMU/OVMF build + runtime regressions passed, including the RX-enabled kernel feature: [run 42270059656](https://github.com/mindwork64/exhiOS/actions/runs/42270059656).
- [x] Interactive RX byte-echo mode: `EXHIOS_SERIAL_RX_TEST=1 ./scripts/run-qemu.sh`; printable ASCII echo, Enter prompt, Ctrl+D termination and byte counter.
- [x] Local Linux Mint/QEMU typed input (2026-11-06): entered `hello exhiOS`, saw the same text echoed and a fresh `RX>` prompt; Ctrl+D ended the loop. Counter was `0x0D` (13 received bytes, including Enter).
- [ ] Backspace visual erase behavior still needs a separate explicit keystroke check; the handler is implemented.


### P3.2 — Framebuffer console
**Первая реализация функционально подтверждена CI и локальным Linux Mint/QEMU; вывод на экран подтверждён описанием скриншота пользователя.**
- [x] Новый `no_std` crate `exhi-console`: 5x7 ASCII bitmap font rendered into 12x16 cells; clear, write, decimal output, CR/LF/tab, wrap and one-row scroll.
- [x] GOP format 0 (RGB-reserved) and 1 (BGR-reserved) packing; other GOP formats are rejected explicitly.
- [x] Every write respects `pixels_per_scan_line`, uses volatile pixel accesses and checks framebuffer size/stride/dimensions.
- [x] Kernel validates that the GOP buffer lies inside the currently mapped 0..4 GiB identity range before creating the console.
- [x] Host tests: RGB/BGR byte order, stride padding, invalid stride/format/buffer, wrap/scroll and decimal output.
- [x] Headless QEMU/OVMF boot regression [run 42272107662](https://github.com/mindwork64/exhiOS/actions/runs/42272107662): framebuffer initialization marker printed; prior address-space and P2.6 IST1/debug-exit tests also passed (status 33).
- [x] Local Linux Mint/QEMU serial evidence (2026-11-06): GOP framebuffer 0x80000000, 1280x800, stride 1280, format 1; initialization marker printed; kernel reached handoff and halt loop.
- [x] Screenshot description supplied by user (2026-11-06) confirms all expected on-screen strings: kernel banner, console ready, 1280x800, stride 1280, BGRX, COM1 ready and kernel handoff success.
- [ ] Detailed inspection of glyph sharpness, spacing and visual artifacts was not possible from the textual image description alone.
- [ ] BitMask/BltOnly pixel formats, Unicode fonts and framebuffer cache policy remain future work.

**Unified writer follow-up (implementation checkpoint [558b394](https://github.com/mindwork64/exhiOS/commit/558b394f8b5d8dc66d3cd565e21302bbffde049a)):**
- [x] Allocation-free `KernelConsole` mirrors byte and formatted output to COM1 and GOP framebuffer after framebuffer initialization; if GOP is unavailable it continues as serial-only output.
- [x] Early boot diagnostics remain on COM1 until GOP setup succeeds; QEMU debug-exit and final halt messages use the unified writer.
- [x] CI build + headless QEMU runtime passed in [run 42275788072](https://github.com/mindwork64/exhiOS/actions/runs/42275788072).
- [x] Local Linux Mint/QEMU (2026-11-06): the unified writer emitted readiness, kernel handoff and halt-loop messages on COM1; the same writer call also targets the initialized framebuffer.

### P3.3 — PIT timer and monotonic ticks
**Первая реализация завершена; CI и локальный Linux Mint/QEMU runtime подтверждены 2026-11-06.**
- [x] PIT channel 0 programmed in mode 2 at requested 100 Hz (input 1,193,182 Hz, divisor 11,932).
- [x] IDT vector 0x20 interrupt gate installed and read-back validated.
- [x] PIC exposes IRQ0 mask/unmask controls; timer startup unmasks IRQ0 only and preserves all other PIC masks.
- [x] Minimal assembly IRQ0 stub preserves RAX, performs a locked 64-bit tick increment, sends master PIC EOI and returns with IRETQ. It does not call Rust or allocate memory.
- [x] Public monotonic `ticks()` and wrap-safe `elapsed_ticks(start)` interfaces use an atomic 64-bit counter.
- [x] Opt-in timer smoke test enables interrupts, waits for at least five real PIT ticks, disables IF, masks IRQ0 again and verifies the mask state.
- [x] CI build of all variants plus headless QEMU/OVMF tests passed: [run 42275788072](https://github.com/mindwork64/exhiOS/actions/runs/42275788072). QEMU printed 5 elapsed ticks and exited with status 33.
- [x] Local Linux Mint/QEMU runtime (2026-11-06): user serial log confirms 100 Hz PIT setup, five elapsed ticks, IRQ0 remasked, timer runtime success marker and QEMU debug-exit status 33.
- [ ] APIC/IOAPIC, HPET/TSC calibration and scheduler integration remain future work.

### P3.4 — Keyboard
**Завершено; CI и локальная интерактивная проверка подтверждены 2026-11-06.**
- [x] i8042 initialization: keyboard port enabled, auxiliary port disabled, IRQ1 enabled, Set 2-to-Set 1 translation selected, keyboard scanning enabled and ACK checked.
- [x] PIC IRQ1 mask/unmask preserves IRQ0 and all other PIC mask bits.
- [x] IDT vector 0x21 gate installed and read-back validated.
- [x] Assembly-only IRQ1 handler reads port 0x60, writes to a fixed SPSC queue, counts overflow and sends master PIC EOI; no Rust call or allocation in ISR.
- [x] no_std exhi-input decoder maps Set 1 make/break to HID-usage KeyEvents and tracks modifiers/lock keys.
- [x] CI host tests: 6/6 passed; keyboard-enabled kernel variant compiled successfully in [run 42278708100](https://github.com/mindwork64/exhiOS/actions/runs/42278708100).
- [x] Local Linux Mint/QEMU (2026-11-06): i8042 config 0x65, IRQ1 ready, 28 key transitions from letters/Enter/Backspace/navigation/Ctrl+D, zero dropped scan codes, IRQ1 remasked and debug-exit status 33. Ctrl state was visible on D; no separate Shift usage event was present in this log (decoder Shift handling is covered by host tests).

### P3.5 — Driver registration
**Завершено; локальный Linux Mint/QEMU и self-hosted CI подтверждены 2026-11-07.**
- [x] New no_std `exhi-driver` crate with static descriptors, stable driver IDs/classes and lifecycle states.
- [x] Fixed-capacity registry; no heap allocation, duplicate-ID and capacity checks, lookup/iteration and lifecycle transition validation.
- [x] COM1, PIT channel 0 and PS/2 keyboard expose static descriptors and are registered during kernel startup.
- [x] Boot diagnostics report registered/initialized/failed counts and each registered driver.
- [x] Serial is marked Initialized after its early init; PIT and PS/2 become Initialized only after their respective hardware init succeeds; failures are tracked as Failed.
- [x] CI: 8/8 registry host tests, all kernel build variants, shell syntax checks and headless QEMU tests (standard boot, PIT timer and driver registry) passed in [run 42299723267](https://github.com/mindwork64/exhiOS/actions/runs/42299723267).
- [x] Local Linux Mint/QEMU (2026-11-06): registered=3, initialized=1, failed=0; COM1 Initialized, PIT and PS/2 Registered; registry runtime smoke test passed; QEMU debug-exit status 33.
- [x] Self-hosted CI build and headless QEMU standard/PIT/registry tests passed in [run 42301446350](https://github.com/mindwork64/exhiOS/actions/runs/42301446350).

## P4 — Tasks and scheduling

### P4.1 — Task model and scheduler policy
**Host tests 11/11 и full self-hosted CI подтверждены 2026-11-07.**
- [x] New no_std `exhi-task` crate: fixed-capacity task table, monotonic TaskId, opaque saved-RSP context and Ready/Running/Blocked/Finished lifecycle.
- [x] Cooperative round-robin policy: spawn/start/yield/block/wake/finish/reap, no heap allocation.
- [x] Self-hosted CI: 11/11 task-model tests, all kernel build variants and headless QEMU standard/PIT/registry regressions passed in [run 42302307480](https://github.com/mindwork64/exhiOS/actions/runs/42302307480).
- [x] Локальный Linux Mint: `cargo fmt` + `cargo test`, все 57 host tests (ABI, console, driver, ELF, heap, input, memory, task) passed 2026-11-06.
- [ ] x86-64 kernel stacks, initial stack frames and actual register/RSP switching are P4.2.

### P4.2 — x86-64 cooperative context switching
**Завершено; self-hosted CI и локальный Linux Mint/QEMU подтверждены.**
- [x] Две статические task stacks: по 4 KiB unmapped guard page + 16 KiB usable stack.
- [x] SysV-совместимый initial frame; assembly сохраняет/восстанавливает RSP и RBP/RBX/R12–R15.
- [x] Bootstrap вызывает task entry; завершившаяся задача получает Finished и не возобновляется; последняя задача возвращает выполнение на boot stack.
- [x] TSS.RSP0 соответствует активному task stack; при возврате в boot context сбрасывается. Double-fault IST1 остаётся независимым.
- [x] Self-hosted QEMU [run 42304100157](https://github.com/mindwork64/exhiOS/actions/runs/42304100157) и локальный пользовательский Linux Mint/QEMU log (2026-11-06) подтвердили A→B→A→B→A→B, сохранность stack-local данных на разных стеках, Finished=2, восстановление boot context/TSS.RSP0 и debug-exit 33.

### P4.3 — PIT-backed cooperative task runtime accounting
**Реализация в main; CI и локальная QEMU-приёмка ожидаются.**
- [x] Добавить per-task `runtime_ticks` в `exhi-task` и насыщающее накопление на безопасной границе планирования.
- [x] Оставить IRQ0 stub минимальным: атомарный tick increment + EOI; никакого вызова Rust или scheduler из ISR.
- [x] Добавить отдельный feature-gated QEMU test: две задачи ждут реальные PIT intervals, чередуются кооперативно и накапливают собственное время.
- [ ] Проверить новые host tests, kernel build и headless QEMU runtime в self-hosted CI.
- [ ] Проверить локально: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_TASK_ACCOUNTING_TEST=1 ./scripts/run-qemu.sh`.

### P4.4 — Timer-driven preemption (planned)
- [ ] Спроектировать полный interrupt frame для безопасного переключения с IRQ0.
- [ ] Сохранять/восстанавливать все необходимые general-purpose registers, interrupt return state и корректный task stack/TSS.RSP0.
- [ ] Определить безопасный порядок scheduler mutation, EOI и interrupt return.
- [ ] Не вызывать напрямую P4.2 call-based context switch из минимального IRQ0 stub.

## P5 — User-mode isolation
- [ ] Ring 3, per-process address spaces, syscalls, user-pointer validation, capabilities.

## P6 — Storage
- [ ] VFS, RAMFS, file/directory operations.

## P7 — Userland
- [ ] init, libexhi, serial shell, basic utilities.
