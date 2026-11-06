# ExhiOS — текущее состояние

Снимок: **2026-11-06**  
Репозиторий: https://github.com/mindwork64/exhiOS  
Ветка: `main`  
Последний известный HEAD: `572cce506098b2465497a34cc0b00b7fa5b1aefd`

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

## Текущая задача — P2.2 page tables

В HEAD добавлен черновик `exhi-kernel/src/arch/x86_64/paging.rs`:
- PML4 → PDPT → четыре Page Directory;
- identity map 0–4 GiB страницами 2 MiB;
- шесть физических страниц для таблиц;
- CR3 switch;
- проверки трансляции kernel code, stack и PML4.

**Важно:** файл пока не экспортирован из `arch/x86_64/mod.rs`, не вызывается при старте и не имеет CI/QEMU подтверждения. Считать его готовой подсистемой нельзя. Следующий шаг — ревью, интеграция, сборка, затем QEMU.

Первая карта широкая и временная: пока нет W^X, user/supervisor isolation, 4 KiB mappings, отдельных process address spaces и управления cache attributes.

## Команды

Обновить локальную копию:
```bash
git fetch origin
git reset --hard origin/main
```

Обычный запуск: `./scripts/run-qemu.sh`  
INT3: `EXHIOS_EXCEPTION_TEST=1 ./scripts/run-qemu.sh`  
QEMU debug-exit: `EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh`
