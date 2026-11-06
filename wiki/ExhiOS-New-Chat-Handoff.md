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
- P0/P1, P2.1, P2.2, P2.3, P2.4 и initial P2.5 address-space layer завершены по локальному + CI runtime evidence.
- P2.3 local QEMU: NX enabled; 2 MiB to 4 KiB split; RW supervisor NX read/write; unmap/TLB invalidation; identity restore and reclaim; debug-exit 33.
- P2.4 local + CI QEMU: heap at 4 GiB; 4 initial pages; grows to 18 pages; cross-page read/write, free/reuse/coalescing; 0x12000 free bytes; debug-exit 33.

## Текущий checkpoint
- P2.5 base checkpoint: [a856090](https://github.com/mindwork64/exhiOS/commit/a856090c0d6e25215e5b56d269894b00696bd01d) (local + CI QEMU verified).
- Latest code checkpoint: [e19db15](https://github.com/mindwork64/exhiOS/commit/e19db15bf891805c2e22cb3251ebade7872bf515) (P2.5.1 hardening; CI and local runtime pending).
- P2.5 initial address-space manager committed. New PML4/PDPT per space; shared supervisor-only kernel PDPT entries 0..4; reserved entries 5..7; private user VA 8..512 GiB (PDPT entries 8..511).
- Unsafe user-frame mapping contract: caller owns and scrubs the frame before exposing it to untrusted code.
- Initial headless QEMU CI [run 42217123278](https://github.com/mindwork64/exhiOS/actions/runs/42217123278) passed: same VA 0x200000000 mapped to different physical frames in two roots; A1/B2 contents survived CR3 switches; shared kernel/heap maps survived; private page tables reclaimed; debug-exit 33.
- Latest hardening commit a856090 adds explicit supervisor-only assertions for shared code/stack/heap and the user frame safety contract. Headless QEMU/OVMF CI [run 42217369804](https://github.com/mindwork64/exhiOS/actions/runs/42217369804) and local Linux Mint/QEMU both passed.
- Local serial values: kernel root 0x11B000; A PML4 0x177000; B PML4 0x179000; shared kernel translation 0x104000; shared heap translation 0x163000; VA 0x200000000 → A frame 0x17B000 / B frame 0x17C000; private page tables reclaimed; debug-exit 33.
- P2.5 smoke test runs at Ring 0. No actual Ring 3 process execution, syscalls or hardware user-fault test yet. TSS/IST and stack guard page also remain future hardening.
- P2.5.1 latest code adds use-after-destroy guards, active-destroy rejection, W^X denial, invalid/duplicate map and double-unmap tests, page-table frame validation and retry-safe teardown. This code has not yet received CI/local runtime confirmation.

## Следующие действия
1. Получить CI и локальный runtime evidence для P2.5.1 checkpoint e19db15; исправить compile/runtime ошибки до дальнейшего расширения подсистемы.
2. После закрытия P2.5.1 перейти к P2.6: TSS и отдельный IST stack для double fault; перевести #DF gate на IST и проверить контролируемую диагностику.
3. Только после подтверждённого IST path добавить guard page под kernel stack. Не допускать тройного fault при тестах.
4. Ring 3, системные вызовы и аппаратные user/supervisor fault tests остаются отдельным последующим user-mode этапом.

Локальная проверка P2.5 завершена 2026-11-06. Для следующих кодовых изменений обновлять main через:
~~~bash
git fetch origin
git reset --hard origin/main
~~~
После этого запускать `cargo fmt`, `cargo test` и `EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh` по необходимости.

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
