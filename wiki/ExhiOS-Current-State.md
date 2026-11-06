# ExhiOS — текущее состояние

Снимок: **2026-11-06**  
Репозиторий: https://github.com/mindwork64/exhiOS  
Ветка: `main`  
Кодовый checkpoint P2.2: 6ae6826ac5e9ad9969efa907d438571444942dec (Paging подключён к старту ядра). Последующие изменения этой записи фиксируются отдельным Wiki/docs-коммитом.

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

## P2.2 — Bootstrap page tables: код подключён, QEMU pending

В коммите [6ae6826](https://github.com/mindwork64/exhiOS/commit/6ae6826ac5e9ad9969efa907d438571444942dec):
- paging.rs экспортирован из arch/x86_64/mod.rs и вызывается во время kernel startup после P2.1.
- Собирается четырёхуровневая структура PML4 → PDPT → четыре Page Directory.
- Первые 4 GiB identity-mapped страницами 2 MiB.
- Шесть страниц таблиц размещены в выровненном kernel .bss, а не взяты из physical allocator. Это намеренно: до первого CR3 switch нельзя считать, что произвольный физический кадр доступен под firmware mappings.
- До switch проверяются CPU state, IF=0, адреса kernel code, stack, BootInfo, UEFI memory map, framebuffer, ACPI RSDP, GDT, IDT и storage таблиц.
- После CR3 switch повторно проверяются те же диапазоны; отдельный translate_current() проверяет kernel entry.
- Kernel serial output сообщает root/CR3, размер карты, размер huge page, число проверенных диапазонов и translation entrypoint.

CI run 42204902818 прошёл успешно: ABI, ELF parser, physical allocator tests, UEFI build, default kernel build, exception smoke-test build, QEMU debug-exit build и ET_EXEC check. После первого прохода выявлены и удаляются три warning в cleanup-коммите.

QEMU runtime пока не подтверждён. Следующая проверка пользователем после обновления локальной копии:
```bash
EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh
```
Ожидаем serial строки "P2.2: kernel page tables active; CR3 and identity mappings verified." и нормальный QEMU debug-exit status 33. До этого P2.2 остаётся незакрытым.

Bootstrap map — временная карта RWX/supervisor для первых 4 GiB. W^X, user/supervisor isolation, 4 KiB mappings, per-device cache attributes и process address spaces не реализованы.

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
