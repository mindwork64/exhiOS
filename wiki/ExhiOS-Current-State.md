# ExhiOS — текущее состояние

Снимок: **2026-11-06**  
Репозиторий: https://github.com/mindwork64/exhiOS  
Ветка: `main`  
Кодовый checkpoint: 207df7ffc86d97cdc15f32b8231ee2d1176f2139 (Paging + headless QEMU runtime CI).

## Закрыто и проверено в QEMU

### P0 — UEFI boot chain
Loader читает и валидирует ELF64, загружает PT_LOAD по фиксированным физическим адресам, обнуляет BSS, получает GOP и ACPI RSDP, подготавливает BootInfo и 64 KiB stack, получает финальную memory map, выполняет ExitBootServices и передаёт управление ядру по System V x86-64 ABI.

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
