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
- P0/P1, P2.1, P2.2, P2.3, P2.4, P2.5 и P2.5.1 прошли локальную + CI runtime проверку.
- P2.5.1: use-after-destroy, idempotent destroy, active-destroy rejection, W^X, invalid/duplicate mappings, double-unmap, page-table frame validation and retry-safe teardown.
- P2.5.1 CI QEMU/OVMF: [run 42238977281](https://github.com/mindwork64/exhiOS/actions/runs/42238977281); local Linux Mint/QEMU log also passed with debug-exit 33.
- P2.5 is Ring 0 address-space management only; actual CPL3 execution, syscalls and hardware-enforced user/supervisor fault tests are not implemented.

## Текущий кодовый checkpoint
- Latest code commit: [b46c870](https://github.com/mindwork64/exhiOS/commit/b46c8702e3725281e9a42654524649d1d04b4fe8).
- P2.6 implementation: 64-bit TSS in GDT (selector 0x18), TR loaded with LTR, 16 KiB aligned IST1 stack, IDT vector 8 uses IST1; exception assembly passes normalized frame pointer to the Rust #DF handler.
- BootInfo ABI was bumped to magic `EXHIOS02`, version 2, and now carries kernel stack base/size.
- UEFI loader allocates a contiguous 17-page LOADER_DATA region: first page is a reserved guard page; remaining 16 pages form the 64 KiB kernel stack.
- Kernel reserves the guard physical page before later allocator use, then removes its identity mapping through VMM after the UEFI map is parsed. It verifies the guard is NotMapped and the stack remains supervisor mapped.
- QEMU debug-exit test sets RSP to the first byte of the mapped kernel stack and accesses the guard page. Page-fault frame delivery hits the same guard page and escalates to #DF; CPU must switch to IST1.
- Headless CI/OVMF runtime: [run 42256795134](https://github.com/mindwork64/exhiOS/actions/runs/42256795134) passed. Serial showed guard 0x1E6FF000, stack 0x1E700000..0x1E710000, exception stack pointer 0x16AFC8 inside IST1, P2.6 success marker and debug-exit 33.
- Local Linux Mint/QEMU test for latest P2.6 guard-page checkpoint is still pending. Do not mark P2.6 fully closed until that log is reviewed.

## Следующие действия
1. Пользовательская локальная проверка checkpoint b46c870:
   ```bash
   git fetch origin
   git reset --hard origin/main
   cargo fmt
   cargo test
   EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh
   ```
   Expected markers: kernel stack guard page reported unmapped; stack remains supervisor-only; controlled guard-page fault raises #DF; exception stack pointer lies inside IST1; debug-exit status 33.
2. Once local P2.6 is confirmed, start P3 device foundation: serial abstraction, framebuffer console, timer, keyboard and driver registry.
3. Keep Ring 3, syscalls and hardware user/supervisor fault tests as a later user-mode milestone. Guard page is now implemented and CI-tested; don't remove or bypass it.

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
