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
- P0/P1, P2.1, P2.2, P2.3, P2.4, P2.5, P2.5.1 и P2.6 прошли локальную + CI runtime проверку.
- P2.5.1: use-after-destroy, idempotent destroy, active-destroy rejection, W^X, invalid/duplicate mappings, double-unmap, page-table frame validation and retry-safe teardown.
- P2.5.1 CI QEMU/OVMF: [run 42238977281](https://github.com/mindwork64/exhiOS/actions/runs/42238977281); local Linux Mint/QEMU log also passed with debug-exit 33.
- P2.5 is Ring 0 address-space management only; actual CPL3 execution, syscalls and hardware-enforced user/supervisor fault tests are not implemented.

## Текущий кодовый checkpoint
- Latest code commit: [6014dec](https://github.com/mindwork64/exhiOS/commit/6014dec505682a4d91ff7165d222f274a15561da).
- P2.6 implementation: 64-bit TSS in GDT (selector 0x18), TR loaded with LTR, 16 KiB aligned IST1 stack, IDT vector 8 uses IST1; exception assembly passes normalized frame pointer to the Rust #DF handler.
- BootInfo ABI was bumped to magic `EXHIOS02`, version 2, and now carries kernel stack base/size.
- UEFI loader allocates a contiguous 17-page LOADER_DATA region: first page is a reserved guard page; remaining 16 pages form the 64 KiB kernel stack.
- Kernel reserves the guard physical page before later allocator use, then removes its identity mapping through VMM after the UEFI map is parsed. It verifies the guard is NotMapped and the stack remains supervisor mapped.
- QEMU debug-exit test sets RSP to the first byte of the mapped kernel stack and reads from the guard page. Page-fault frame delivery hits the same guard page and escalates to #DF; CPU switches to IST1.
- Headless CI/OVMF runtime: [run 42256850017](https://github.com/mindwork64/exhiOS/actions/runs/42256850017) passed. Local Linux Mint/QEMU log on 2026-11-06 also passed: guard 0x1E0D3000, stack 0x1E0D4000..0x1E0E4000, guard unmapped, stack supervisor-only, #DF frame pointer 0x122FC8 inside IST1 0x11F000..0x123000, debug-exit 33. P2.6 is closed.
- P3.1 adds a reusable polling 16550 SerialPort driver in `arch/x86_64/serial.rs`. COM1 output is migrated from main.rs; RX adds non-blocking try_read_byte and polling read_byte_blocking. Boot prints the P3.1 readiness marker.
- RX-inclusive driver build + headless QEMU regression [run 42258141119](https://github.com/mindwork64/exhiOS/actions/runs/42258141119) passed before the interactive test harness was added. The CI boot trace proves TX/boot regression, not receipt of input bytes.
- Commits [a58220e](https://github.com/mindwork64/exhiOS/commit/a58220e5fda9afb069b71f08529ebc68f95a4d36) and [6014dec](https://github.com/mindwork64/exhiOS/commit/6014dec505682a4d91ff7165d222f274a15561da) add the opt-in interactive RX byte-echo mode. Run `EXHIOS_SERIAL_RX_TEST=1 ./scripts/run-qemu.sh`; type ASCII, test Enter/Backspace, then press Ctrl+D to print the received-byte count. This mode has not yet been locally exercised.
- P2.6 is closed; only the P3.1 typed-byte RX test remains pending.

## Следующие действия
1. Проверить результат текущего CI для commit 6014dec: сборка обычного ядра и нового `serial-rx-smoke-test` варианта, затем headless QEMU regression.
2. Пользовательская проверка latest main:
   ```bash
   git fetch origin
   git reset --hard origin/main
   cargo fmt
   cargo test
   EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh
   EXHIOS_SERIAL_RX_TEST=1 ./scripts/run-qemu.sh
   ```
   Первый QEMU запуск повторяет проверку P2.6 и завершается status 33. Второй должен показать `P3.1 RX test ready`; введённые символы должны возвращаться echo, Enter — переводить на новый prompt, Backspace — стирать символ, Ctrl+D — печатать число принятых байтов.
3. После локального подтверждения RX переходить к следующему отдельному этапу: framebuffer console, затем timer и keyboard. Не смешивать это с P3.1 до приёмки RX.
4. Ring 3, syscalls and hardware user/supervisor fault tests remain a later user-mode milestone. Guard page is locally and CI verified; do not remove or bypass it.

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
