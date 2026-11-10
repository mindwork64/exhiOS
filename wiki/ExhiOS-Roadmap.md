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
- [x] Отдельные address spaces с собственными PML4 roots и kernel mapping policy — реализованы в P2.5.
- Физические адреса выше текущего 4 GiB allocator window намеренно отложены; это documented limitation alpha 0.1.

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
- [x] Дополнительные lifecycle/error-path и permission-negative tests — закрыты в P2.5.1.
- [x] Ring 3 execution и hardware-enforced user/supervisor fault tests — закрыты в P5.

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
- [x] Additional deliberate stack exhaustion/guard-page variant for process stacks: all 16 usable 4 KiB pages (64 KiB) of init, worker and reaper stacks are verified as identity-mapped supervisor RW pages, and each lower guard is verified unmapped. The worker-stack guard fault escalates from #PF to #DF on IST1.
- [x] Локальный Linux Mint/QEMU прогон пользователя (2026-11-10): `P2.6: init, worker and reaper stacks have 64 KiB supervisor mappings and unmapped guards`; controlled worker guard fault produced vector 8 (Double Fault), the exception frame was on IST1, `P2.6: double-fault IST runtime smoke test passed`, QEMU debug-exit status 33.

Runtime command:

```bash
EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_PROCESS_STACK_GUARD_TEST=1 EXHIOS_HEADLESS=1 ./scripts/run-qemu.sh
```

The process-stack guard scenario is accepted from the supplied local QEMU runtime log. User-side Clippy completed successfully but reported one new `unreachable_code` warning caused by the intentionally diverging smoke-test trigger; the normal Ring 3 handoff tail is now cfg-excluded in this test configuration ([fix](https://github.com/mindwork64/exhiOS/commit/23f34f878b4fa63a4e9600633f0c477f4e54a66c)). User reran `cargo fmt` and `cargo clippy -p exhi-kernel --bin exhi-kernel --features process-stack-guard-smoke-test --target x86_64-unknown-none` on 2026-11-10. The command finished successfully with no new `unreachable_code` warning; only existing Clippy suggestions remain in `exhi-heap`, `exhi-elf`, `exhi-memory` and baseline kernel modules. The process-stack guard implementation is lint-verified. Follow-ups: cfg fix [23f34f8](https://github.com/mindwork64/exhiOS/commit/23f34f878b4fa63a4e9600633f0c477f4e54a66c), formatting [76c06c5](https://github.com/mindwork64/exhiOS/commit/76c06c589224ae315e659ae035584c48a8730594), and diagnostic-literal preservation [0bfd656](https://github.com/mindwork64/exhiOS/commit/0bfd6565a224b6b3a0986209b7bebd286a680e74).

Feature implementation: [521e5fe](https://github.com/mindwork64/exhiOS/commit/521e5fe875217f8c18e2167598a07751577659aa).

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
**Завершено; host tests, self-hosted CI и локальный Linux Mint/QEMU подтверждены 2026-11-07.**
- [x] Per-task `runtime_ticks` в `exhi-task`, saturating accumulation только у Running task.
- [x] IRQ0 stub остаётся assembly-only: atomic tick increment + EOI; Rust и scheduler из ISR не вызываются.
- [x] Отдельный QEMU test с двумя задачами, реальными PIT intervals и cooperative accounting на yield/finish boundaries.
- [x] Host tests: 14/14 `exhi-task`; kernel feature build и все QEMU regression modes прошли в self-hosted CI [run 42306726315](https://github.com/mindwork64/exhiOS/actions/runs/42306726315).
- [x] CI и локальный Linux Mint/QEMU serial: elapsed PIT ticks=0x12, task A runtime=0x9, task B runtime=0x9, Finished=2, IRQ0 masked=yes, TSS.RSP0 restored=yes, pass marker, debug-exit 33.

### P4.4 — Timer-driven preemption
**Завершено; self-hosted CI и локальный Linux Mint/QEMU подтверждены 2026-11-07.**
- [x] Полный long-mode interrupt frame: 15 сохранённых GPR + RIP/CS/RFLAGS/RSP/SS (20 qwords, 160 bytes). Начальный synthetic IRETQ frame явно задаёт RSP=task stack top и SS=0x10.
- [x] Отдельный IRQ0 preemption stub сохраняет GPR и per-task FX state через FXSAVE64/FXRSTOR64; generic P3.3 IRQ0 stub не изменён.
- [x] Rust preemption handler учитывает runtime, выполняет round-robin scheduler selection, переключает FX state и TSS.RSP0; выбранный complete frame восстанавливается через IRETQ.
- [x] Две задачи busy-loop без cooperative yield. Test проверяет timer preemption, runtime, независимые stack locals, XMM state, scheduler state, TSS.RSP0 и маску IRQ0.
- [x] Фатальные исключения в preemption test завершают QEMU failure debug-exit, а не оставляют runner в HLT до таймаута.
- [x] Self-hosted CI [run 42334341610](https://github.com/mindwork64/exhiOS/actions/runs/42334341610) и regression workflow [run 42335136555](https://github.com/mindwork64/exhiOS/actions/runs/42335136555) прошли. Serial: elapsed=0x12 ticks, switches=0x12, runtime A=0x9/B=0x9, XMM preserved для обеих задач, stacks distinct=yes, TSS.RSP0 active=yes, IRQ0 masked=yes, pass marker, debug-exit 33.
- [x] Локальный Linux Mint/QEMU (2026-11-07): elapsed=0x12 ticks, switches=0x12, runtime A=0x9/B=0x9, XMM preserved, stacks distinct=yes, TSS.RSP0 active=yes, IRQ0 masked=yes, pass marker и debug-exit 33.

### P4.5 — Preemptive task termination
**Завершено; local Linux Mint/QEMU и self-hosted CI приняты 2026-11-07.**
- [x] Отдельный feature/test mode `preemptive-task-exit-smoke-test` / `EXHIOS_PREEMPTIVE_TASK_EXIT_TEST=1`; P4.4 runtime test оставлен отдельной регрессией.
- [x] Returning task entry вызывает terminal trampoline на собственном стеке. Он запрещает interrupts, начисляет финальный runtime, переводит current task в Finished через `Scheduler::finish_current()`, выбирает Ready peer и восстанавливает его interrupt frame через IRETQ.
- [x] Последняя завершившаяся задача оставляет scheduler без current task; test очищает TSS.RSP0 и active FX-state pointer, маскирует IRQ0 и выполняет controlled debug-exit. Память стеков пока статическая и не освобождается.
- [x] Проверены две задачи без cooperative yield: A завершилась первой; B продолжила работу и затем тоже вернулась. Проверяются оба Finished state, отсутствие current/Ready/Blocked задач, runtime, frame bounds, XMM, отдельные stacks, очищенный TSS.RSP0 и IRQ0 mask.
- [x] Self-hosted CI [run 42335136555](https://github.com/mindwork64/exhiOS/actions/runs/42335136555): build и все QEMU regression modes прошли. P4.5 serial: elapsed=0x18 ticks, switches=0x10, runtime A=0x8/B=0xA, A finished=yes, B finished=yes, XMM preserved, stacks distinct=yes, current=none, Finished=2, TSS.RSP0 cleared=yes, IRQ0 masked=yes, pass marker, debug-exit 33.
- [x] Локальный Linux Mint/QEMU (2026-11-07): elapsed=0x18, switches=0x10, runtime A/B=0x8/0x10, A/B Finished, XMM preserved, stacks distinct, current=none/Finished=2, TSS.RSP0 cleared, IRQ0 masked, pass marker и debug-exit 33.

### P4.6 — Idle context and blocked-task wake-up
**Завершено; self-hosted CI build + QEMU runtime приняты 2026-11-07.** Пользователь отдельно разрешил считать этот self-hosted runner эквивалентом локальной Linux Mint/QEMU системы.
- [x] Явное idle-состояние scheduler при отсутствии Ready tasks и наличии Blocked tasks; idle не является фиктивной пользовательской Task.
- [x] Отдельный guarded idle stack + 20-qword long-mode interrupt frame. Cooperative P4.2 RSP contexts отделены от IRETQ contexts.
- [x] Dedicated DPL0 scheduler interrupt `INT 0x30` блокирует Running task и сохраняет полный interrupt frame.
- [x] Idle loop `sti; hlt`; PIT IRQ0 сохраняет idle frame, выполняет wrap-safe deadline wake-up и dispatches Ready task с её interrupt frame, FXSAVE и обновлённым TSS.RSP0.
- [x] Завершение задачи при наличии только Blocked peers переключает в idle. Финальная задача очищает TSS.RSP0 и active FX-state pointer.
- [x] Host tests для idle entry/exit, timed deadline, u64 tick wraparound и недопустимых переходов.
- [x] Self-hosted CI [run 42337327289](https://github.com/mindwork64/exhiOS/actions/runs/42337327289): build и полный QEMU runtime suite прошли, включая отдельный `EXHIOS_IDLE_WAKEUP_TEST=1` runtime test и P4.4/P4.5 regressions.
- [x] Runtime проверяет две задачи с разными wake deadlines, idle entries/ticks, обе Finished, сохранность stack-local/XMM state, current=none, Ready=0, Blocked=0, Finished=2, TSS.RSP0 cleared, IRQ0 masked и debug-exit 33.
- [x] По инструкции пользователя успешный self-hosted QEMU runtime считается эквивалентным локальной системе. P4.6 принят без отдельного ручного прогона.
- [x] Scope: Ring 0, single CPU, PIT/legacy PIC, static guarded stacks. Освобождение task stacks — отдельный этап.

## P5 — User-mode isolation

**P5.1–P5.7 завершены и приняты по self-hosted build + полному headless QEMU runtime suite.** Реализация общего production syscall entry и memory primitives находится в always-built x86-64 architecture modules. Подтверждённая конфигурация BSP/single-CPU; SMP/AP-local GS и user TLS не заявляются.

- [x] **P5.1 — Ring 3 entry and CPL3 exception return:** DPL3 selectors, private CR3, TSS.RSP0, validated IRETQ frame and processor-originated exception return.
- [x] **P5.2 — Supervisor isolation:** CPL3 cannot access supervisor-only kernel mappings; hardware #PF reports the expected U/S protection fault.
- [x] **P5.3 — System-call entry/return:** validated SYSCALL MSR setup, common GS-relative BSP entry, trusted kernel-stack switch, normalized frame, FXSAVE/FXRSTOR and IRETQ return.
- [x] **P5.4 — User-pointer validation:** canonical-address, complete mapped-range and per-page read/write/execute permission checks; bounded to 16 MiB.
- [x] **P5.5 — Safe user copies:** whole-range validation before touching memory; invalid cross-boundary destination is rejected without partial writes.
- [x] **P5.6 — User-task lifecycle:** two isolated Ring 3 tasks with private CR3/code/stack contexts; syscall yield/exit, idle dispatch, kernel reaping, address-space/frame reclamation and scheduler-record removal.
- [x] **P5.7 — Capabilities/default deny:** typed process capability set, empty-by-default user tasks, unknown-bit rejection and enforcement inside the shared syscall dispatcher before service invocation.

Runtime evidence: [CI run #405](https://github.com/mindwork64/exhiOS/actions/runs/42434078573) passed build, host tests and all 20 headless QEMU scenarios. It verified common syscall entry/return; separate A/B user address spaces; authorized ConsoleWrite; denied Yield and ConsoleWrite without side effects; two exits; kernel reaping; page/frame reclamation; task-record removal and TSS.RSP0 cleanup. The accepted scope is BSP-only: AP-local GS, user TLS, NMI/exception SWAPGS-window handling and SMP scheduling remain future work.

## P6 — Storage

**Initial allocator-free VFS/RAMFS implementation is complete and QEMU-verified.**

- [x] Allocator-free `FileSystem` contract, generation-tagged `NodeId`, metadata and paginated directory-entry API.
- [x] Bounded fixed-capacity `RamFs<NODE_COUNT, FILE_CAPACITY>` with fixed node table and per-node file storage.
- [x] Lookup, nested absolute/relative path resolution, create/read/write/remove, directory pagination, sparse zero-fill, capacity/overflow checks, non-empty directory protection and stale-handle rejection.
- [x] Kernel integration smoke test covers `/etc/exhi.conf`, binary read/write round-trip, enumeration, oversized-write no-mutation, directory protection, removal and stale-handle invalidation.

Evidence: all eight `exhi-vfs` unit tests passed; [CI run #412](https://github.com/mindwork64/exhiOS/actions/runs/42546613316) passed build and the full headless QEMU runtime job, including `P6: VFS/RAMFS runtime smoke test passed.` and debug-exit status 33.

Scope boundary: userland file-open/read/write syscalls and persistent on-disk filesystems are not implemented. The current configuration is embedded in `exhi-init`; VFS/RAMFS is currently exercised from kernel code.

## P7 — Userland

### P7.1 — Production process creation and lifecycle

- [x] The two-slot production registry and per-process LoadedProcess/address-space, guarded kernel-stack, FXSAVE and scheduler records are runtime-verified.
- [x] Capability-gated `SpawnWorker`: init alone receives `TaskSpawn`; worker receives only `TaskYield`, `TaskExit` and `ConsoleWrite`. The worker's explicit spawn attempt returned `AccessDenied` in both normal and fault modes.
- [x] `Exit` and validated CPL3 faults mark only the current process Finished; the dedicated reaper restores kernel mappings, releases Finished resources and reaps task records after switch-away.
- [x] Serial QEMU verified normal worker startup/yield/exit, occupied-slot rejection, cleanup of both records, and rollback after a post-activation setup failure followed by successful slot reuse.
- [x] Separate worker-fault QEMU mode forced CPL3 `UD2`, verified init remained responsive to `hostname`, and confirmed reaper cleanup of both process records.

### P7.2 — libexhi

- [x] `no_std` wrappers include `console_write`, `console_read`, `file_read`, `spawn_worker`, `yield_now` and `exit`; ABI, wrapper and syscall-dispatcher unit tests passed in Actions #513.

### P7.3 — serial shell

- [x] Bounded COM1 shell with `spawn` passed the interactive QEMU serial-driver scenario, including command handling, line editing, Ctrl+C, config output and exit.

### P7.4 — configuration

- [x] Init reads `/etc/exhi.conf` through capability-gated `FileRead` from kernel RAMFS, applies `hostname`, `prompt` and `version`, and passes the shell runtime scenario. Bounded parser tests passed.
- [x] RAMFS remains volatile and kernel-populated; persistent storage and FileWrite are outside P7.

**Validation:** GitHub Actions [run #513](https://github.com/mindwork64/exhiOS/actions/runs/42829639418) passed on self-hosted runner `eqstream`: build job success and the complete QEMU runtime job success (30 completed steps). This includes the new worker rollback/slot-reuse test, normal worker capability/yield/exit test, worker-fault recovery with live init, production two-process scheduler/reaper test, and the existing platform smoke tests. P7.1–P7.4 are accepted for the current BSP/single-CPU implementation. Persistent storage and SMP are not implied.
## Current platform boundaries

- Production first-process runtime and the shared syscall CPU-local context are BSP/single-CPU only.
- AP-local GS setup, SMP scheduling/TLB shootdown, user TLS and NMI/exception handling during the SWAPGS transition are not claimed.
- The initial VFS is in-memory RAMFS; persistent storage and userland filesystem syscalls remain future work.

### 2026-11-10 — Fault-recovery GS transition correction

- The CPL3 worker-fault handoff now restores the user GS layout with `SWAPGS` before returning to a still-Ready Ring 3 init process. If the user-entry transfer unexpectedly returns or rejects its frame, the path restores trusted kernel GS before reporting the fatal handoff failure. The reaper path does not switch back to user GS.
- User ran `cargo fmt` and the kernel `process-worker-fault-smoke-test` Clippy configuration successfully on [9b63adf](https://github.com/mindwork64/exhiOS/commit/9b63adf625512e2429110a1c89f0ed5d06060dcc). The only reported kernel warnings are 20 existing suggestions in legacy modules.
- This verifies compilation/linting, not runtime behavior. Worker-fault and normal spawn/reaper QEMU scenarios remain unexecuted; P7 stays open.

### P7.1 — Worker capability-denial regression coverage (2026-11-10)

- The worker now attempts `SpawnWorker` and requires the exact `AccessDenied` response before continuing. This is checked in both the normal yield/exit path and the fault-injection path before `UD2`.
- Both QEMU serial drivers now require the explicit `TaskSpawn denied as expected` marker. This proves the capability gate rejects the syscall before the process-slot allocator can return a generic resource error.
- Code is committed; userland Clippy and runtime scenarios remain pending. This does not change P7 acceptance state.

### 2026-11-10 — Worker capability-denial lint passed

- User ran `cargo fmt` and Clippy for `exhi-init --bins` in both the default configuration and with `worker-fault-smoke-test`. Both completed successfully with no warnings.
- The exact `TaskSpawn → AccessDenied` assertion is now lint-clean in both worker paths. Runtime confirmation by the serial QEMU drivers remains pending; P7 is still open.


## R0 — ExhiOS 0.1.0-alpha.1 release preparation

**State: the Release image has been built and booted successfully in QEMU/OVMF; final source/redistribution gates remain open.**

- [x] Define the target version in the root `VERSION` file.
- [x] Select Release userland artifacts when `debug_assertions` are disabled; retain Debug include paths for development builds.
- [x] Add `scripts/package-release.sh`: build init/worker first, embed those Release ELFs into the Release kernel, build the UEFI application, verify ELF types, create the GPT/FAT32 EFI System Partition, write the fallback bootloader and kernel, read them back for byte-for-byte comparison, and generate SHA-256 checksums/archive.
- [x] Add `scripts/run-release-qemu.sh` for the packaged image with OVMF and fresh NVRAM variables.
- [x] Update README, BUILD, USERLAND, Release Notes, CHANGELOG, version strings and smoke-test expectations.
- [x] User ran `scripts/package-release.sh` successfully on Linux Mint; GPT verification, FAT directory listing and byte-for-byte ESP readback checks passed. The image, archive and checksum files were created.
- [x] User booted the actual Release image through QEMU/OVMF. UEFI handoff, Release kernel, memory/VMM/heap/address-space startup, framebuffer/COM1 and Ring 3 init all reached the interactive shell.
- [x] User manually verified `help`, `hostname`, `version`, `config`, `echo`, worker startup, exact worker `TaskSpawn → AccessDenied`, yield sequence and clean worker exit.
- [x] User reproduced worker slot recycling: after one worker exited, the next `spawn` emitted `reclaimed finished process before worker spawn`, started another worker and completed its yield/exit sequence. Runtime evidence was supplied on 2026-11-11 against the worker-reclamation change.
- [x] `scripts/run-release-qemu.sh` accepted the image checksum before launch. The archive checksum has not yet been independently checked with `sha256sum --check`.
- [ ] Run Clippy for the current kernel `process-runtime` path after the worker-reclamation change; userland Clippy had passed earlier. Run `cargo fmt` on the final synced tree.
- [ ] Add the locally generated `Cargo.lock` to source control; it is not currently present on `main`.
- [ ] Independently verify the release archive checksum.
- [ ] Choose and add a project license/redistribution terms; no LICENSE/COPYING file exists.
- [ ] Rebuild and repeat the Release QEMU smoke test on the final synced commit after the authorization-order hardening; only then consider a release tag. Do not publish until the license and remaining gates are resolved.

The current Release runtime evidence is QEMU/OVMF-only. It does not qualify physical hardware, SMP, persistent storage or broad device support.
