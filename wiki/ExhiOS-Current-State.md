# ExhiOS — текущее состояние

Снимок: **2026-11-06**  
Репозиторий: https://github.com/mindwork64/exhiOS  
Ветка: `main`  
Кодовый checkpoint: d1ea6145e4fa2c10a04e03725d7677b679cbede0 (P3.1 COM1 polling TX/RX; CI build + headless QEMU pass, local QEMU pending).

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

CI serial evidence (run 42256795134):
- Guard page 0x1E6FF000; kernel stack 0x1E700000..0x1E710000 (64 KiB).
- Guard translation is NotMapped; first/last stack pages remain mapped and supervisor-only.
- Exception stack pointer 0x16AFC8 lies inside IST1.
- #DF/IST1 runtime success marker printed; QEMU debug-exit status 33.

Local Linux Mint/QEMU validation by the user is still pending.

## P3.1 — Polling serial driver

Code: [d1ea614](https://github.com/mindwork64/exhiOS/commit/d1ea6145e4fa2c10a04e03725d7677b679cbede0).

- Reusable 16550-compatible SerialPort with configurable I/O base.
- COM1 initialization: 38400 baud, 8N1, FIFO, transmitter polling with a bounded timeout.
- write/write_byte and fixed-width hexadecimal output are implemented.
- RX: non-blocking try_read_byte and polling read_byte_blocking are implemented for the later serial shell.
- main.rs uses serial::COM1 for all early boot and exception diagnostics; direct COM1 I/O helpers were removed from main.rs.
- CI build for normal, exception and QEMU debug-exit kernel variants passed. RX-inclusive headless QEMU regression [run 42258141119](https://github.com/mindwork64/exhiOS/actions/runs/42258141119) also passed; boot log includes the P3.1 readiness marker and the P2.6 guard-page/#DF/IST1 success marker.
- Interactive byte reception through the user's local QEMU serial terminal has not yet been tested.

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
