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
- P0/P1, P2.1, P2.2, P2.3 и P2.4 завершены.
- P2.3 local QEMU: NX enabled; 2 MiB to 4 KiB split; RW supervisor NX read/write; unmap/TLB invalidation; identity restore and reclaim; debug-exit 33.
- P2.4 local + CI QEMU: heap at 4 GiB; 4 initial pages; grows to 18 pages; cross-page read/write, free/reuse/coalescing; 0x12000 free bytes; debug-exit 33.

## Текущий checkpoint
- Code checkpoint: [a856090](https://github.com/mindwork64/exhiOS/commit/a856090c0d6e25215e5b56d269894b00696bd01d).
- P2.5 initial address-space manager committed. New PML4/PDPT per space; shared supervisor-only kernel PDPT entries 0..4; reserved entries 5..7; private user VA 8..512 GiB (PDPT entries 8..511).
- Unsafe user-frame mapping contract: caller owns and scrubs the frame before exposing it to untrusted code.
- Initial headless QEMU CI [run 42217123278](https://github.com/mindwork64/exhiOS/actions/runs/42217123278) passed: same VA 0x200000000 mapped to different physical frames in two roots; A1/B2 contents survived CR3 switches; shared kernel/heap maps survived; private page tables reclaimed; debug-exit 33.
- Latest hardening commit a856090 adds explicit supervisor-only assertions for shared code/stack/heap and the user frame safety contract. CI rerun [run 42217369804](https://github.com/mindwork64/exhiOS/actions/runs/42217369804) is in progress; local user QEMU check is pending.
- P2.5 smoke test runs at Ring 0. No actual Ring 3 process execution, syscalls or hardware user-fault test yet. TSS/IST and stack guard page also remain future hardening.

## Следующее действие после локального отчёта
После того как пользователь пришлёт serial output на checkpoint a856090:
1. Сверить P2.5 root addresses, same-VA/different-frame results, shared kernel/heap translations, private-table reclaim and debug-exit 33.
2. Записать локальный runtime evidence.
3. Продолжить P2.5: проверить permission transitions and address-space lifecycle/error paths, затем подготовить Ring 3 handoff without enabling interrupts prematurely.

Команда локальной проверки:
~~~bash
git fetch origin
git reset --hard origin/main
EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh
~~~

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
