# ExhiOS — текущее состояние

Снимок: **2026-11-10**
Репозиторий: https://github.com/mindwork64/exhiOS
Рабочая ветка: `main` (code-only update; Actions suppressed via `[skip ci]`).

P0–P6 сохранены. P7 code path включает двухслотовый process manager, capability-gated `SpawnWorker`, независимые address space/kernel stack/FX state, deferred reaper cleanup, `FileRead` из kernel RAMFS и shell-команды `config`/`spawn`. Serial QEMU driver расширен проверкой запуска worker и очистки обоих процессов. Эта версия не собиралась и не запускалась в QEMU/CI; `cargo`, `rustfmt` и `clippy-driver` в локальном контейнере отсутствуют.

## Закрыто и проверено в QEMU

### P0 — UEFI boot chain
Loader читает и валидирует ELF64, загружает PT_LOAD по фиксированным физическим адресам, обнуляет BSS, получает GOP и ACPI RSDP, подготавливает BootInfo v2 и выделяет единый блок из 17 страниц: одну guard page и 64 KiB kernel stack. Затем получает финальную memory map, выполняет ExitBootServices и передаёт управление ядру по System V x86-64 ABI.

### P1 — Kernel foundation
- GDT загружена; CS=0x08, DS=ES=SS=0x10 подтверждены.
- IDT загружена; SIDT подтвердил base и limit 0x0FFF.
- INT3 реально вызвал vector 3 (Breakpoint); Rust handler напечатал error code, RIP, CS, RFLAGS и остановил CPU.
- Legacy PIC переназначен на 0x20–0x2F; обе маски прочитаны как 0xFF; IF остаётся выключенным.
- QEMU isa-debug-exit завершил гостя с ожидаемым status 33.

### P2.1 — Physical page allocator
Проверен на настоящей UEFI memory map:
- 127 descriptors (`0x7F`), stride 48 bytes (`0x30`), version 1.
- 118 837 conventional pages (`0x1D035`), около 464,2 MiB.
- 159 страниц ниже 1 MiB зарезервированы; выше 4 GiB доступных страниц не было.
- Верхняя граница управляемой области: `0x1FE77000`.
- Allocation → free → reuse вернул тот же frame: `0x14C000`.
- QEMU завершился с status 33.

P2.1 закрыт по runtime evidence. В дорожной карте основного репозитория это отмечено.

## P2.2 — Bootstrap page tables: завершено и проверено в QEMU

Код: [6ae6826](https://github.com/mindwork64/exhiOS/commit/6ae6826ac5e9ad9969efa907d438571444942dec). Headless QEMU CI: [run 42205457670](https://github.com/mindwork64/exhiOS/actions/runs/42205457670). Локальный QEMU пользователя: проверка 2026-11-06, serial output получен в чате.

Реализация:
- paging module экспортирован и вызывается при kernel startup после P2.1;
- PML4 → PDPT → четыре Page Directory;
- identity map 0–4 GiB через 2 MiB supervisor RW pages;
- шесть page-table pages находятся в выровненном kernel .bss, что не требует предполагать доступность произвольных физических кадров под старыми UEFI mappings;
- до и после CR3 switch проверяются code, stack, BootInfo, UEFI map, framebuffer, ACPI RSDP, GDT, IDT и storage таблиц;
- отдельная translate_current проверка сопоставляет kernel entry с физическим адресом.

QEMU/OVMF runtime evidence из CI:
- PML4 root = active CR3 = 0x14E000;
- 4 GiB отображено, huge page size 2 MiB;
- проверено 9 address ranges;
- kernel entry translation 0x100000 → 0x100000;
- success serial line выведена, debug-exit status 33.

Локальная проверка пользователя (2026-11-06):
- PML4 root = active CR3 = 0x10C000;
- 4 GiB отображено, huge page size 2 MiB;
- проверено 9 address ranges;
- stack 0x1E0E3A00;
- kernel entry translation 0x100000 → 0x100000;
- success serial line выведена, debug-exit status 33.
Различие адреса корневой таблицы относительно CI ожидаемо: UEFI memory map и размещение loader allocations отличаются между запусками.

P2.2 закрыт по CI и локальному runtime smoke test.

Ограничения bootstrap map — временная карта RWX/supervisor для первых 4 GiB. W^X, user/supervisor isolation, 4 KiB mappings, per-device cache attributes и process address spaces ещё не реализованы.


## P2.3 — Virtual Memory Manager: CI + local QEMU verified

Код: [08f179a](https://github.com/mindwork64/exhiOS/commit/08f179af25190092597dead6ad918082fa79b79b). Headless QEMU/OVMF CI: [run 42213581974](https://github.com/mindwork64/exhiOS/actions/runs/42213581974).

Реализовано:
- 4 KiB `map_page`, `unmap_page`, `query_page`; split 2 MiB PDE в 512 PTE.
- Page flags: RW/RO, user/supervisor, executable/NX, PWT, PCD, global; NXE только после CPUID.
- `invlpg` на изменениях; безопасное coalesce обратно в 2 MiB при проверке всех 512 PTE; освобождение child table frame после инвалидации 512 virtual pages.

CI QEMU подтвердил NX enabled, split, RW supervisor NX mapping, запись/чтение, unmap/NotMapped, identity restore, page-table reclaim и debug-exit 33.

Локальный Linux Mint/QEMU прогон пользователя (2026-11-06) подтвердил:
- P2.2 root/CR3: `0x110000`; 4 GiB bootstrap identity map, 9 critical ranges.
- NX support: enabled.
- 2 MiB page split into 4 KiB entries.
- RW supervisor NX map, read/write, unmap и TLB invalidation.
- Original identity map restored; page-table frame safely reclaimed.
- `P2.3: virtual memory manager 4 KiB mapping smoke test passed.`
- QEMU debug-exit status 33.

P2.3 закрыт. Ограничения: VMM пока single-core и требует IF=0 для изменений; address spaces с отдельными PML4 и физический доступ выше 4 GiB ещё не готовы.

## P2.4 — Kernel Heap: завершено и подтверждено локально + в CI

Код: [f03e92b](https://github.com/mindwork64/exhiOS/commit/f03e92b7239cd75b95ff1dffc1a4dce053a74f30). Headless QEMU/OVMF CI: [run 42215755990](https://github.com/mindwork64/exhiOS/actions/runs/42215755990).

Реализация:
- Новый exhi-heap (no_std): first-fit free list, alignment, splitting, deallocation, coalescing и reuse; host tests 4/4.
- Kernel GlobalAlloc получает кадры через exhi-memory, отображает RW+NX через VMM в virtual range от 0x1_0000_0000 (4 GiB), расширяется по требованию до 64 MiB.
- Начальная heap region — 4 страницы. Рост сериализован spinlock при локально выключенных interrupts; частичное расширение откатывается.
- Startup smoke test проверяет align64, 48 KiB align4096 allocation, запись/чтение через границы 4 KiB pages, free/reuse/coalescing.

Локальный Linux Mint/QEMU прогон пользователя (2026-11-06):
- Heap base: 0x100000000.
- Initial pages: 4 (0x4000 bytes).
- After 48 KiB allocation: 18 pages (0x12000 bytes).
- Free bytes after test: 0x12000, т.е. все блоки вернулись в free list.
- Строка P2.4 kernel heap allocator ready и QEMU debug-exit status 33.

Headless CI/OVMF run [42215755990](https://github.com/mindwork64/exhiOS/actions/runs/42215755990) показал те же heap показатели и debug-exit 33. P2.4 закрыт по host tests, локальному и CI runtime.

## P2.5 — Kernel Address Spaces: первая реализация

Код:
- [33b4962](https://github.com/mindwork64/exhiOS/commit/33b49627e8b691a6168d5c0b9860029ff0af0b72) — отдельный address_space module, per-space PML4/PDPT, map/unmap/translate/destroy и startup QEMU smoke test.
- [a856090](https://github.com/mindwork64/exhiOS/commit/a856090c0d6e25215e5b56d269894b00696bd01d) — user frame mapping помечен unsafe с ownership/scrub contract; smoke test дополнительно проверяет supervisor-only kernel/heap translations.

Политика отображений:
- Каждое пространство получает собственный PML4 и PDPT.
- PDPT entries 0..4 копируются из kernel root и разделяются: identity map физического диапазона 0..4 GiB и heap VA начиная с 4 GiB.
- PDPT entries 5..7 зарезервированы.
- Private user VA window начинается с 8 GiB (0x200000000) и ограничен 512 GiB; private mappings лежат в PDPT entries 8..511.
- PML4 U/S становится USER при первом user mapping, но общие kernel/heap mappings остаются supervisor-only из-за U/S=0 на нижних уровнях.
- map требует физический кадр во владении caller; вызывающий код обязан держать frame выделенным и очистить его перед передачей недоверенному user-коду. unmap возвращает frame caller; destroy освобождает private page-table frames, но не data frames.

Headless CI QEMU/OVMF [run 42217123278](https://github.com/mindwork64/exhiOS/actions/runs/42217123278) passed:
- Kernel root: 0x15B000; address-space A PML4: 0x177000; B PML4: 0x179000.
- Same user VA 0x200000000 mapped to A frame 0x17B000 and B frame 0x17C000.
- A retained byte 0xA1, B retained byte 0xB2 after CR3 switches.
- Kernel entry and heap translations survived process-root activation.
- Private page tables were reclaimed, data frames returned, debug-exit status 33.

Последний code commit [a856090](https://github.com/mindwork64/exhiOS/commit/a856090c0d6e25215e5b56d269894b00696bd01d) adds explicit assertions that shared kernel code, stack and heap remain supervisor-only, plus the unsafe caller ownership/scrub contract for user frames. Headless QEMU/OVMF CI [run 42217369804](https://github.com/mindwork64/exhiOS/actions/runs/42217369804) passed.

Локальный Linux Mint/QEMU прогон пользователя (2026-11-06) после этого checkpoint также прошёл:
- Kernel address-space root: 0x11B000.
- Address-space A PML4: 0x177000; B PML4: 0x179000.
- Shared kernel translation: 0x104000; shared heap translation: 0x163000.
- User VA 0x200000000 mapped to A physical frame 0x17B000 and B physical frame 0x17C000.
- Separate roots, shared kernel maps, private user mappings and CR3 switching verified.
- Private page tables reclaimed; data frames safely returned.
- Kernel handoff successful; QEMU debug-exit status 33.

P2.5 initial address-space layer закрыт по CI и локальному runtime evidence. Smoke test выполняется на Ring 0: CPL3 execution и аппаратные user/supervisor fault tests ещё не реализованы. P2.5.1 завершён и проверен локально + в CI. Проверки включают use-after-destroy, идемпотентный destroy, запрет destroy активного CR3, W^X, invalid/duplicate map, double-unmap, page-table frame validation и retry-safe teardown. Headless CI/OVMF [run 42238977281](https://github.com/mindwork64/exhiOS/actions/runs/42238977281) прошёл сборку и QEMU runtime. Пользовательский Linux Mint/QEMU лог (2026-11-06) завершился P2.5 smoke success и debug-exit 33; поскольку негативные проверки встроены перед этой success line, каждая из них вернула ожидаемый отказ.

Ограничения: test пока выполняется в Ring 0, он подтверждает раздельные mappings/CR3 и permission bits в page tables, но ещё не запускает код в Ring 3 и не проверяет аппаратные user-mode faults. Это подготовка address-space layer, не завершённая process isolation.

## P2.6 — TSS, double-fault IST and boot-stack guard page

Code checkpoint: [b46c870](https://github.com/mindwork64/exhiOS/commit/b46c8702e3725281e9a42654524649d1d04b4fe8). Headless CI/OVMF runtime: [run 42256850017](https://github.com/mindwork64/exhiOS/actions/runs/42256850017) (build and QEMU runtime both passed).

Implementation:
- 64-bit TSS descriptor is added to GDT and loaded with LTR (selector 0x18).
- A dedicated 16 KiB, page-aligned IST1 stack is reserved in kernel BSS.
- IDT vector 8 selects IST1; IdtStatus verifies its IST index.
- The common exception stub passes the normalized frame pointer as the sixth SysV argument; the #DF handler checks that it lies inside the IST1 range.
- BootInfo v2 carries the boot stack base and size. The loader allocates a contiguous 17-page block: one dedicated guard page plus the 16 pages used for the 64 KiB kernel stack.
- The physical allocator reserves the guard frame before subsequent allocations. After bootstrap paging and VMM setup, the guard identity mapping is removed; the complete stack remains supervisor mapped.
- The QEMU debug-exit variant sets RSP to the bottom of the mapped stack and faults on the guard page. The #PF exception-frame push then faults into the same guard page; the processor raises #DF, switches to IST1, and the handler verifies its stack pointer.

Headless CI/OVMF serial evidence ([run 42256850017](https://github.com/mindwork64/exhiOS/actions/runs/42256850017)):
- Guard page 0x1E6FF000; kernel stack 0x1E700000..0x1E710000 (64 KiB).
- Guard translation is NotMapped; first/last stack pages remain mapped and supervisor-only.
- Exception stack pointer 0x16AFC8 lies inside IST1; QEMU debug-exit status 33.

Local Linux Mint/QEMU serial evidence (2026-11-06, supplied log):
- Guard page 0x1E0D3000; kernel stack 0x1E0D4000..0x1E0E4000 (64 KiB).
- Kernel explicitly reports that the guard page is unmapped and the boot stack remains mapped and supervisor-only.
- Controlled guard fault raises vector 8 (Double Fault); exception frame pointer 0x122FC8 lies inside IST1 range 0x11F000..0x123000.
- P2.6 double-fault IST runtime smoke test passed; QEMU debug-exit status 33.

P2.6 is closed with both local and CI runtime evidence.

## P3.1 — Polling serial driver

Driver implementation: [d1ea614](https://github.com/mindwork64/exhiOS/commit/d1ea6145e4fa2c10a04e03725d7677b679cbede0). Interactive RX diagnostic mode: [a58220e](https://github.com/mindwork64/exhiOS/commit/a58220e5fda9afb069b71f08529ebc68f95a4d36), with control-flow cleanup in [6014dec](https://github.com/mindwork64/exhiOS/commit/6014dec505682a4d91ff7165d222f274a15561da).

- Reusable 16550-compatible SerialPort with configurable I/O base.
- COM1 initialization: 38400 baud, 8N1, FIFO, transmitter polling with a bounded timeout.
- TX: write_byte, write and fixed-width hexadecimal output. Local boot log (2026-11-06) confirms COM1 output.
- RX: non-blocking try_read_byte and polling read_byte_blocking.
- Opt-in diagnostic mode: `EXHIOS_SERIAL_RX_TEST=1 ./scripts/run-qemu.sh`; it echoes printable bytes, handles Enter/Backspace and exits on Ctrl+D after reporting the received-byte count.
- CI build + headless QEMU regression passed for the interactive RX kernel feature: [run 42270059656](https://github.com/mindwork64/exhiOS/actions/runs/42270059656).
- Local typed input (2026-11-06): `hello exhiOS` echoed correctly, a fresh `RX>` prompt appeared after Enter, and Ctrl+D ended the loop with byte count `0x0D` (13 including Enter). Backspace was not explicitly exercised in the supplied log.
- P3.1 is closed with local input and CI build/runtime evidence.

## Wiki и автоматическая синхронизация

Исходники страниц находятся в `wiki/` основного репозитория. Workflow `.github/workflows/publish-wiki.yml` публикует их в отдельный GitHub Wiki repository после изменений в `wiki/**`. Первый publish прошёл успешно; Wiki commit: `4181995` на ветке `master`. Существующий контент Home сохраняется вне управляемого блока; посторонние Wiki-страницы не удаляются.

## Команды

Обновить локальную копию:
```bash
git fetch origin
git reset --hard origin/main
```

Обычный запуск: `./scripts/run-qemu.sh`  
INT3: `EXHIOS_EXCEPTION_TEST=1 ./scripts/run-qemu.sh`  
QEMU debug-exit: `EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh`




## P3.2 unified KernelConsole writer — implementation checkpoint

Commits: [c3d2b8c](https://github.com/mindwork64/exhiOS/commit/c3d2b8ccaa34c6b399b7f12905ae11658455916b) adds `exhi-kernel/src/console.rs`; [558b394](https://github.com/mindwork64/exhiOS/commit/558b394f8b5d8dc66d3cd565e21302bbffde049a) integrates it into kernel handoff.

- `KernelConsole` sends byte slices to COM1 and, when initialized, to GOP framebuffer.
- `write_fmt` uses `core::fmt` and does not allocate from the kernel heap.
- If framebuffer initialization fails, the writer remains useful through COM1 only.
- Early diagnostics remain serial-only before GOP initialization; the post-handoff, unified-writer-ready, QEMU debug-exit and final halt messages use both outputs.
- CI build and headless QEMU runtime passed in [run 42275788072](https://github.com/mindwork64/exhiOS/actions/runs/42275788072). Local Linux Mint/QEMU log (2026-11-06) confirms the writer emitted the unified readiness, kernel handoff and halt-loop lines on COM1.

## P3.2 — GOP framebuffer console (initial implementation)

Initial framebuffer implementation checkpoint: [eee4895](https://github.com/mindwork64/exhiOS/commit/eee489589c0b79416ac4753e181dfd1656b8deed).

- New `exhi-console` no_std crate with a compact 5x7 ASCII font rendered in 12x16 cells.
- RGB-reserved GOP format 0 and BGR-reserved format 1 are packed separately; BitMask and BltOnly modes return an explicit unsupported-format error.
- The renderer honors `pixels_per_scan_line` independently of visible width, validates backing-buffer length, and uses volatile pixel writes. It supports clear, text, decimal numbers, CR/LF/tab, line wrap and scroll.
- Kernel-side initialization checks dimensions, alignment, buffer byte count and that the framebuffer range lies within the current 4 GiB identity map. On success the first screen shows the kernel banner, resolution, scanline stride and pixel format.
- Host tests, all target build variants and headless QEMU/OVMF runtime passed in [run 42272107662](https://github.com/mindwork64/exhiOS/actions/runs/42272107662). Serial output confirms framebuffer initialization; the P2.6 IST1/debug-exit regression also passed (status 33).
- Local Linux Mint/QEMU serial evidence (2026-11-06): GOP framebuffer 0x80000000, 1280x800, stride 1280, pixel format 1 (BGR-reserved); console initialization marker printed and kernel reached handoff + halt loop.
- The user supplied a screenshot description confirming all expected on-screen strings: kernel banner, console ready, 1280x800 resolution, stride 1280, BGRX format, COM1 ready and successful kernel handoff. Fine visual details (glyph sharpness, spacing and artifacts) were not assessed from the text description.
- Known first-version limits: ASCII-only glyphs, GOP formats 0/1 only, no framebuffer cache policy setup (PAT/MTRR) yet.

## P3.3 — PIT timer and monotonic tick source

Code checkpoint: [d2ed6b2](https://github.com/mindwork64/exhiOS/commit/d2ed6b2d40260340396af1db8284b607eba1c4ad). CI build + headless QEMU/OVMF: [run 42275788072](https://github.com/mindwork64/exhiOS/actions/runs/42275788072).

- New module: `exhi-kernel/src/arch/x86_64/timer.rs`.
- PIT channel 0 is configured as mode 2, requested 100 Hz, input clock 1,193,182 Hz, divisor 11,932 (actual nominal rate approximately 99.9985 Hz).
- IDT vector 0x20 uses a dedicated interrupt gate; IDT initialization reads it back and includes it in the global validity check.
- PIC IRQ0 can be unmasked/masked without changing other lines. The timer starts with IF clear, then its caller explicitly enables interrupts. Other PIC IRQs remain masked.
- The minimal assembly IRQ0 stub saves RAX, atomically increments the 64-bit `EXHIOS_PIT_TICKS` counter, sends EOI to the master 8259 and returns with IRETQ. It does not call Rust or touch the heap.
- API: `timer::ticks()` and wrap-safe `timer::elapsed_ticks(start)`. The counter is monotonic modulo 2^64.
- Opt-in test: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_TIMER_TEST=1 ./scripts/run-qemu.sh`. It waits for at least five delivered ticks, disables IF, masks IRQ0 again and verifies PIC state.
- CI build passed all host tests and all kernel variants; headless QEMU first passed the existing P2.6 guard/#DF regression, then timer QEMU printed `P3.3: monotonic timer advanced by 5 ticks; IRQ0 masked again` and exited status 33.
- Local Linux Mint/QEMU (2026-11-06): user serial log confirms PIT at 100 Hz/divisor 11,932, five elapsed ticks, IRQ0 remasked, timer runtime success marker and QEMU debug-exit status 33.
- Current scope is the legacy PIT + PIC path. Local APIC/IOAPIC, HPET/TSC calibration and scheduler integration are future work.

## P3.4 — PS/2 keyboard and input events

## P3.5 — Static driver registry

New no_std crate: `exhi-driver`.
- `DriverDescriptor` is immutable metadata: stable `DriverId`, broad `DriverClass`, and static name.
- `DriverRegistry<const CAPACITY>` stores a fixed-size array of optional entries. Registration, lookup, iteration, state counting and transitions require no heap allocation.
- Duplicate IDs, full capacity, unknown IDs and invalid lifecycle transitions return explicit errors. States are `Registered`, `Initialized`, and `Failed`; a failed state is terminal for this boot.
- COM1, PIT channel 0 and the i8042 PS/2 keyboard expose descriptors from their own driver modules. The boot path registers all three after PIC initialization; COM1 is marked initialized after early UART setup. Timer/keyboard states transition to Initialized only after their hardware setup succeeds, or Failed on an initialization error.
- The registry is an inventory/lifecycle layer only. It does not call hardware initialization callbacks; boot order remains explicit so IDT/PIC/device dependencies are visible.
- Boot diagnostics print registry totals and each driver's ID/class/state. Dedicated runtime mode: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_DRIVER_REGISTRY_TEST=1 ./scripts/run-qemu.sh`.
- CI registry host tests: 8/8 passed; all kernel build variants and shell syntax checks passed. Headless standard boot, PIT timer and driver-registry runtime smoke tests passed on the self-hosted runner in [run 42301446350](https://github.com/mindwork64/exhiOS/actions/runs/42301446350).
- Local Linux Mint/QEMU (2026-11-06): registered=3, initialized=1, failed=0; COM1 is Initialized, PIT and i8042 PS/2 keyboard are Registered. Registry runtime pass marker printed and QEMU exited with status 33.


New no_std crate: `exhi-input`; hardware driver: `exhi-kernel/src/arch/x86_64/keyboard.rs`.
- The i8042 controller is configured with IRQ1 enabled, IRQ12 disabled, keyboard clock enabled, auxiliary clock disabled and translated Set 1 output. The keyboard receives the F4 enable-scanning command and its ACK is checked before PIC IRQ1 is unmasked.
- IDT vector 0x21 points to a dedicated IRQ1 stub. PIC exposes IRQ1-only mask/unmask operations preserving all other lines.
- IRQ1 assembly reads port 0x60, publishes one byte into a fixed SPSC queue, increments an overflow counter if full, sends master PIC EOI and returns with IRETQ. The stub makes no Rust calls and allocates nothing.
- Foreground decoding maps Set 1 make/break bytes to USB HID keyboard usage IDs. KeyEvent contains physical key identity, down/up state and modifier snapshot; left/right Shift, Control, Alt and Meta are tracked independently, and Caps/Num/Scroll Lock toggle on make.
- Host unit tests: 6/6 passed. CI also built the keyboard-enabled kernel variant in [run 42278708100](https://github.com/mindwork64/exhiOS/actions/runs/42278708100).
- Interactive mode: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_KEYBOARD_TEST=1 ./scripts/run-qemu.sh`. Click the QEMU display, type keys, then hold Ctrl and press D. The kernel prints event transitions and dropped-byte count, masks IRQ1 and requests QEMU debug-exit.
- Local Linux Mint/QEMU runtime (2026-11-06): i8042 config 0x65, IRQ1 gate valid, keyboard events received for letters, Enter, Backspace and arrows; Ctrl+D was decoded with ctrl=true, 28 transitions total, zero dropped scan codes, IRQ1 masked again and QEMU status 33. No separate Shift event appears in this log; Shift decoding has host-test coverage. This initial driver supports legacy PS/2 only; USB HID enumeration and keyboard-layout/character translation are future work.


## P4.1 — Task model and scheduler core (host tests + self-hosted CI passed)

New no_std crate: `exhi-task`.
- `TaskId` is monotonically assigned within a scheduler. A reaped task slot may be reused, but an identifier is never reused.
- Each fixed-size task record stores a static name, lifecycle state and opaque `TaskContext` containing the architecture-prepared saved stack pointer.
- `Scheduler<const CAPACITY>` is a fixed-capacity, allocation-free round-robin policy core.
- States: Ready, Running, Blocked and Finished. APIs cover spawn, start, cooperative yield, block/wake, finish and reap.
- Invalid zero stack pointers, capacity overflow, unknown tasks and illegal state transitions are rejected.
- Self-hosted CI passed all 11 task-model unit tests, all kernel build variants and headless standard/PIT/registry QEMU regressions in [run 42302307480](https://github.com/mindwork64/exhiOS/actions/runs/42302307480).
- User-local Linux Mint `cargo fmt` + `cargo test` passed all 57 host tests on 2026-11-06.
- This slice does not allocate kernel stacks or switch CPU registers. x86-64 initial stack frames/context switching and two-task QEMU proof are P4.2.
- Tasks remain Ring 0 and share the kernel address space. No user-mode isolation or timer preemption is claimed.


## P4.2 — x86-64 cooperative context switching (local + self-hosted CI accepted)

Implementation:
- Feature-gated architecture module: `exhi-kernel/src/arch/x86_64/task.rs`; runtime mode: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_CONTEXT_SWITCH_TEST=1 ./scripts/run-qemu.sh`.
- Two static task stacks, each 20 KiB total: one 4 KiB unmapped guard page + 16 KiB usable stack. Guard pages are removed from the identity map through VMM before first use.
- Synthetic initial frame is 16-byte aligned and matches the assembly restore order. R12 carries the task entry pointer into the assembly bootstrap.
- Context switch saves/restores RSP and the SysV callee-saved set RBP/RBX/R12–R15. Caller-saved registers remain caller-clobbered according to the ABI.
- Returning task entry functions transition to Finished and never resume. The final task restores the suspended boot stack.
- TSS.RSP0 is updated before each switch and cleared/restored after returning to boot; double-fault IST1 remains independent.
- Self-hosted QEMU [run 42304100157](https://github.com/mindwork64/exhiOS/actions/runs/42304100157) and user-local Linux Mint/QEMU log (2026-11-06) both confirm A/B alternation A→B→A→B→A→B, preserved task-local values at distinct stack addresses, both tasks Finished, boot context resumed, TSS.RSP0 restored and QEMU status 33.
- P4.2 is closed. It remains cooperative, Ring 0 and single-CPU; timer preemption, Ring 3 and per-process CR3 isolation are not included.

## P4.3 — PIT-backed cooperative task runtime accounting (local + CI accepted)

- Added `runtime_ticks: u64` to each `exhi-task::Task` and `Scheduler::account_current_ticks()`; accumulation saturates rather than wrapping. Three new scheduler unit tests cover independent task totals, saturation and invalid state; all 14 task tests pass.
- The feature-gated x86-64 runtime test uses the real 100 Hz PIT. Two guarded Ring 0 tasks wait for PIT ticks, alternate cooperatively and account their own elapsed intervals at yield/finish boundaries.
- The IRQ0 stub remains assembly-only: locked tick increment + master PIC EOI. It never calls Rust or accesses the scheduler.
- Tasks enable interrupts with `sti; hlt` while waiting. IF is cleared across accounting and each call-based context switch, then enabled by the resumed task; this prevents IRQ0 from interrupting an incomplete stack restore.
- Runtime command: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_TASK_ACCOUNTING_TEST=1 ./scripts/run-qemu.sh`.
- Corrected self-hosted CI [run 42306726315](https://github.com/mindwork64/exhiOS/actions/runs/42306726315) passed all host tests (including 14/14 task tests), kernel build variants, standard/PIT/registry/P4.2 regressions and the actual P4.3 QEMU runtime test. Serial output: two tasks each completed three PIT intervals; elapsed=0x12 ticks, task A=0x9, task B=0x9, Finished=2, IRQ0 masked=yes, TSS.RSP0 restored=yes, P4.3 pass marker and debug-exit status 33.
- An earlier CI run exited through the generic P2.6 guard/#DF probe before reaching P4.3. The P2.6 test cfg was corrected to exclude this dedicated accounting mode; run 42306726315 is the valid P4.3 runtime evidence.
- User-local Linux Mint/QEMU serial (2026-11-07): 18 elapsed PIT ticks; runtime A=9/B=9; Finished=2; IRQ0 masked; TSS.RSP0 restored; pass marker; debug-exit 33. P4.3 is closed locally and in CI. This is cooperative accounting, not timer preemption.

## P4.4 — Timer-driven preemption (local + CI accepted)

- Dedicated test mode: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_TIMER_PREEMPTION_TEST=1 ./scripts/run-qemu.sh`.
- Long-mode hardware interrupt frames are 20 qwords in this path: 15 saved GPRs followed by RIP, CS, RFLAGS, RSP and SS. IRETQ consumes all five return slots even for a same-CPL Ring 0 return. The initial synthetic context therefore includes explicit task RSP and kernel SS=0x10.
- An earlier 18-qword frame missed RSP/SS and faulted immediately in IRETQ (#PF at CR2 0x13C000). Commit [b0365e3](https://github.com/mindwork64/exhiOS/commit/b0365e30acbeb40143913299cb25a2d072e3546a) fixes the frame and makes fatal P4.4 exceptions exit QEMU as test failures rather than hanging in HLT.
- Dedicated IRQ0 preemption stub saves general registers and each task's FXSAVE64 state, increments PIT ticks and sends PIC EOI before calling the Rust scheduler/accounting handler. The selected task's frame and FX state are restored, followed by IRETQ. TSS.RSP0 follows the selected task; IST1 remains independent.
- Two Ring 0 tasks busy-loop without cooperative yields. Self-hosted CI [run 42334341610](https://github.com/mindwork64/exhiOS/actions/runs/42334341610) and regression workflow [run 42335136555](https://github.com/mindwork64/exhiOS/actions/runs/42335136555) passed all build and QEMU jobs: 18 PIT ticks, 18 preemption switches, A/B runtime=9/9, XMM state preserved for both, distinct task stacks, active TSS.RSP0, IRQ0 masked, P4.4 pass marker and debug-exit 33.
- User-local Linux Mint/QEMU (2026-11-07): elapsed=0x12, switches=0x12, runtime A/B=0x9/0x9, XMM preserved, stacks distinct, TSS.RSP0 active, IRQ0 masked, pass marker and debug-exit 33. P4.4 is closed. Scope remains Ring 0, single CPU and PIT/legacy PIC; Ring 3 and syscalls remain in P5.

## P4.5 — Preemptive task termination (local + self-hosted CI accepted)

- New feature alias: `preemptive-task-exit-smoke-test`; runtime command: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_PREEMPTIVE_TASK_EXIT_TEST=1 ./scripts/run-qemu.sh`.
- The dedicated assembly bootstrap calls `exhi_preemptive_task_finished` when a preemptive task entry returns. The Rust path disables interrupts, accounts the final PIT runtime interval, calls `Scheduler::finish_current()` and either dispatches the next task's saved interrupt frame through IRETQ or shuts down the last-task test.
- On dispatch it validates the next task frame and stack bounds, updates TSS.RSP0, updates the active FXSAVE area and restores the next task with the common interrupt-context entry. When no Ready tasks remain, it clears TSS.RSP0 and active FX-state pointer, masks IRQ0 and requests controlled QEMU debug-exit.
- Task stacks are static and are not yet reaped or freed. Idle-context dispatch and blocked-task wake-up are implemented and runtime-tested in P4.6.
- Self-hosted CI [run 42335136555](https://github.com/mindwork64/exhiOS/actions/runs/42335136555) passed build and all QEMU regressions. Serial: elapsed=0x18, switches=0x10, runtime A=0x8/B=0xA, both tasks Finished, XMM preserved, stacks distinct, current=none, Finished=2, TSS.RSP0 cleared, IRQ0 masked, P4.5 pass marker and debug-exit 33.
- Local Linux Mint/QEMU (2026-11-07): user serial log confirms elapsed=0x18, switches=0x10, runtime A/B=0x8/0x10; both tasks Finished; XMM state preserved; stacks distinct; current=none/Finished=2; TSS.RSP0 cleared; IRQ0 masked; P4.5 pass marker and debug-exit 33. P4.5 is closed locally and in CI.

## P4.6 — Idle context and blocked-task wake-up

**Завершено по self-hosted CI/build + QEMU runtime.** Пользователь явно разрешил считать успешный QEMU на его self-hosted runner эквивалентом локальной Linux Mint/QEMU системы.

- Scheduler имеет явное idle-состояние, отличное от обычной Task; последняя runnable-задача может блокироваться, не оставляя фиктивную Running task.
- Для idle создан отдельный guarded stack и собственный 20-qword IRETQ frame. Cooperative P4.2 saved-RSP contexts не смешиваются с полными interrupt frames.
- `INT 0x30` сохраняет контекст блокируемой задачи; когда Ready peers нет, CPU переходит в idle loop `sti; hlt`.
- PIT IRQ0 обслуживает idle frame, проверяет wrap-safe deadlines, переводит проснувшиеся задачи в Ready и восстанавливает выбранный interrupt frame, FXSAVE state и TSS.RSP0.
- Runtime test проверяет две задачи с разнесёнными таймерами пробуждения, входы в idle, возврат к сохранённым stack-local и XMM данным, завершение обеих задач, очищенный TSS.RSP0, маску IRQ0 и controlled debug-exit.
- Scheduler host tests включают idle enter/exit, таймерный deadline, переполнение `u64` tick counter и ошибочные переходы.
- Self-hosted CI [run 42337327289](https://github.com/mindwork64/exhiOS/actions/runs/42337327289): build job и полный QEMU runtime job завершились success; runtime suite включал стандартную загрузку, PIT, registry и regressions P4.2–P4.5 вместе с отдельным P4.6 тестом.
- По прямой инструкции пользователя этот успешный self-hosted QEMU runtime принят как локально-эквивалентное подтверждение. Отдельный ручной запуск не требуется.
- Границы: Ring 0, single CPU, PIT/legacy PIC; task stacks пока статические. Освобождение стеков — отдельный этап.

## Текущее состояние на 2026-11-09

### P5 — User-mode isolation: закрыто

P5.1–P5.7 реализованы. Общие x86-64 Ring 3 entry, SYSCALL entry, user-pointer validation и safe-copy примитивы находятся в always-built architecture modules; capability checks выполняются общим `exhi-syscall` dispatcher. Два изолированных Ring 3 task contexts проверены через полный lifecycle: yield, exit, idle, reaper, освобождение address spaces/frames и удаление Finished records.

[CI #405](https://github.com/mindwork64/exhiOS/actions/runs/42434078573) прошёл все 20 QEMU runtime scenarios и build/host-test job. Текущая платформа BSP/single-CPU; SMP/AP-local GS/user TLS не заявлены.

### P6 — VFS/RAMFS: закрыто

`exhi-vfs` предоставляет allocator-free FileSystem API и bounded RAMFS с generation-tagged handles, nested paths, CRUD, directory pagination, sparse zero-fill, capacity checks и защитой stale handles. В kernel smoke test покрыты read/write, oversized-write no-mutation, non-empty directory и повторное использование slot.

[CI #412](https://github.com/mindwork64/exhiOS/actions/runs/42546613316): 8/8 VFS host tests, build и headless QEMU runtime passed. Userland filesystem syscalls и persistent filesystem пока отсутствуют.

### P7 — Userland: состояние

**P7.1 — init: частично готово.** ET_EXEC init загружается в private address space с RX code, RW+NX stack и guard page и исполняется в Ring 3. Production runtime уже использует `exhi-task::Scheduler`: регистрирует init как user task и заблокированный kernel-reaper task; `Yield` проходит через scheduler, `Exit` переводит init в Finished и передаёт управление reaper. Проверенный CPL3 fault path делает явный `SWAPGS`, переводит init в Finished и также передаёт управление reaper. Reaper работает на отдельном guarded kernel stack, восстанавливает kernel CR3, освобождает address space/frames и удаляет Finished task record после switch-away. Всё это прошло QEMU в CI #484. Ограничение: production runtime пока регистрирует только один user process, поэтому `Yield` продолжает init, когда нет другого Ready user task; spawn и multi-process dispatch ещё не реализованы. P5.6 остаётся отдельным двухзадачным scheduler smoke test.

**P7.2 — libexhi: готово.** `no_std` wrappers: `console_write`, `console_read`, `yield_now`, `exit`; стабильные syscall numbers и decoding kernel errors. CI #435 подтвердил использование в Ring 3.

**P7.3 — serial shell: готово.** Интерактивная консоль Ring 3 с line editing, CR/LF/CRLF, Backspace/DEL, Ctrl+C/D, ограничением 127 байт и командами `help`, `hostname`, `version`, `echo`, `clear`, `config`, `exit`. QEMU serial-socket interaction accepted в CI #447.

**P7.4 — конфигурация: готова для embedded config.** `exhi-config` — bounded allocation-free UTF-8 key/value parser; init применяет hostname/prompt/version, invalid config безопасно переключается на defaults. CI #447 passed. Persistent VFS config loading — future work.

### Последняя проверенная точка CI

- HEAD: [8f0b4bf](https://github.com/mindwork64/exhiOS/commit/8f0b4bf5f854b06b2f4d9452b408baf5a0a719d9).
- CI [#484](https://github.com/mindwork64/exhiOS/actions/runs/42597997791): build — success (50 job steps, 0 Rust warning lines); QEMU runtime — success (27 completed job steps, including all 21 runtime scenarios).
- P7.1 runtime markers in #484: init syscall/Exit reaper path passed; invalid-opcode vector 6 was captured; kernel reaper started on its dedicated stack; init address space, user frames and Finished task record were reclaimed; QEMU exited with status 33.
- Feature-specific build gates now keep loader, Ring 3, syscall and shell-only code out of unrelated smoke configurations. The complete CI build matrix is warning-free.

### Ближайшая задача

P7.1: довести production runtime от одного init до нескольких user processes. Следующий этап — process registry/creation path, отдельные LoadedProcess/address-space/kernel-stack/FX/context state для каждого процесса, общая dispatch/termination логика без привязки к `INIT_PROCESS`, и реальное переключение двух production user tasks через `Yield`. Отдельный QEMU сценарий должен проверить два user processes, capability-gated services, CR3/TSS/GS/FX switching, exit/fault и reaper reclamation.
