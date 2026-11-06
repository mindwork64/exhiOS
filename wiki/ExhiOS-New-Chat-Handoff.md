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

## Следующая задача
HEAD `572cce506098b2465497a34cc0b00b7fa5b1aefd` содержит черновой `exhi-kernel/src/arch/x86_64/paging.rs`.

Начать с ревью: page-table flags/masks, address calculations, CR3, identity assumptions, rollback и unsafe access. Затем экспортировать модуль, вызвать после P2.1, собрать CI, проверить в QEMU:
- page tables allocated from frame allocator;
- root written to CR3;
- code, stack и page-table frames остаются доступными;
- translation probes возвращают identity physical addresses.

Текущая заготовка использует 4-level paging, 2 MiB huge pages и карту 0–4 GiB. Она не является законченным VMM.

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
