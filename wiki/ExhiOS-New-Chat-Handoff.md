# ExhiOS — передача контекста в новый чат

Прочитать:
1. [[ExhiOS Current State]]
2. [[ExhiOS Architecture]]
3. [[ExhiOS Roadmap]]
4. [[ExhiOS Decisions]]
5. [[ExhiOS Development Log]]

## Репозиторий и workflow
- https://github.com/mindwork64/exhiOS
- Основная ветка `main`.
- Wiki source pages находятся в `wiki/` основного репозитория; GitHub Actions публикует их в Wiki.
- Пользователь работает в Linux Mint + QEMU/OVMF.
- Обновление локального репозитория: `git fetch origin` и `git reset --hard origin/main`. Не предлагать `git pull --ff-only`.
- QEMU запускать из корня: `./scripts/run-qemu.sh`.
- Пользователь присылает serial output для runtime проверки. Не объявлять этап закрытым без него.

## Последняя подтверждённая точка
P0 и P1 закрыты в QEMU. P2.1 проверен с настоящей UEFI memory map: 127 descriptors, stride 48, 118 837 managed pages, frame 0x14C000 повторно использован, debug-exit status 33.

## Последний кодовый checkpoint
- Commit: 6ae6826ac5e9ad9969efa907d438571444942dec.
- Paging module exported and called after physical allocator initialization.
- Bootstrap tables in aligned kernel BSS (six pages), 4-level map 0–4 GiB via 2 MiB pages.
- CR3 switch and translation checks for code, stack, BootInfo, UEFI map, framebuffer, ACPI RSDP, GDT, IDT and table pages before/after switch.
- CI run 42204902818 succeeded for all jobs and all kernel variants.
- First CI log showed one redundant assignment and two unused allocator-helper warnings; cleanup follow-up is recorded in the next main commit.
- Do not mark P2.2 complete yet: QEMU runtime is still pending.

## Следующее обязательное действие
After updating local source, run:
```bash
git fetch origin
git reset --hard origin/main
EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh
```
Inspect serial output for:
- Page-table root (PML4) and Active CR3 with matching physical roots.
- Verified address ranges greater than zero.
- Kernel entry translation matching the kernel entry address.
- P2.2: kernel page tables active; CR3 and identity mappings verified.
- QEMU exits with status 33.

If QEMU halts or resets before that line, capture the full serial log and CR2 if the page-fault handler reports it. Do not claim runtime success on CI build alone.

Bootstrap pages are statically stored in the loaded kernel BSS on purpose; the firmware mapping is not assumed to identity-map arbitrary allocator frames. Dynamic VMM tables can use physical allocator frames after this initial map is active.

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
