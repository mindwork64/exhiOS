# ExhiOS — передача контекста в новый чат

## Что читать в первую очередь
1. [[ExhiOS Current State]] — фактический статус, последний CI и ближайшая задача.
2. [[ExhiOS Roadmap]] — приёмка P0–P7.
3. [[ExhiOS Architecture]] — актуальные подсистемы и границы.
4. [[ExhiOS Decisions]] — обязательные архитектурные решения.
5. [[ExhiOS Development Log]] — история изменений и runtime evidence.

## Репозиторий и рабочий процесс
- Repository: https://github.com/mindwork64/exhiOS
- Основная ветка: `main`; проект Rust Stable, UEFI loader + freestanding x86-64 kernel, target QEMU + OVMF.
- Wiki source of truth — Markdown в `wiki/` основного репозитория. Изменения `wiki/**` на main публикуются workflow `.github/workflows/publish-wiki.yml`.
- Wiki критически важна: обновлять Roadmap, Current State, Architecture/Decisions при архитектурных изменениях, Development Log и New Chat Handoff на каждом завершённом этапе. Не оставлять её на старом milestone.
- Локальное обновление кода пользователя: `git fetch origin`, затем `git reset --hard origin/main`, затем `cargo fmt`. Не предлагать `git pull`.
- Сборка: `./scripts/build.sh`. QEMU: `./scripts/run-qemu.sh`.
- Self-hosted GitHub runner labels: `[self-hosted, linux, x64]`; он уже содержит Rust, binutils, QEMU/OVMF и disk tools.
- Пользователь явно разрешил считать QEMU runtime на этом self-hosted runner эквивалентом локального Linux Mint/QEMU. Не требовать повторного ручного прогона для каждого этапа, когда CI реально выполнил нужный QEMU scenario.
- Не считать один только compile/build доказательством runtime корректности.

## Текущая подтверждённая точка — 2026-11-09
- HEAD main: [3943378](https://github.com/mindwork64/exhiOS/commit/39433785d5a116fa1e6908bf1f91b806bd931c48).
- Последний CI [#451](https://github.com/mindwork64/exhiOS/actions/runs/42580983148): build и QEMU runtime jobs — success. Промежуточные #449/#450 падали, но #451 на текущем HEAD прошёл.
- P0–P6 закрыты по применимым host/build/QEMU evidence.
- P5.1–P5.7: Ring 3 entry/isolation, common SYSCALL, pointer validation/safe-copy, двухзадачный user lifecycle и default-deny capabilities приняты полным runtime suite в [CI #405](https://github.com/mindwork64/exhiOS/actions/runs/42434078573). Scope BSP/single-CPU.
- P6: bounded allocator-free VFS/RAMFS, операции file/directory и kernel smoke test приняты в [CI #412](https://github.com/mindwork64/exhiOS/actions/runs/42546613316). Userland FS syscalls/persistent disk FS отсутствуют.
- P7.1: ET_EXEC init loader, private address space, RX/RW+NX mappings, stack guard, Ring 3 entry, syscalls and cleanup after exit/user fault implemented. Latest #451 includes user-fault cleanup test. P7.1 remains open because production runtime is still single-process and `Yield` resumes the same init; production process scheduler/reaper dispatch is not integrated.
- P7.2: `libexhi` wrappers for console_write/read, yield_now and exit are implemented and tested (CI #435).
- P7.3: interactive Ring 3 serial shell and QEMU serial input tests passed (CI #447).
- P7.4: bounded `key=value` parser; embedded hostname/prompt/version config is applied and displayed. Persistent VFS config load is not implemented.

## Ближайшая задача: закрытие P7.1
Интегрировать общий `exhi-task::Scheduler` в production process runtime, а не переиспользовать только smoke adapter:
1. Process/task control block владеет private address space, user/kernel stacks, saved syscall/interrupt frame, FX state и capability grants.
2. Реальная `Yield` сохраняет контекст и переключает к другой Ready process; при dispatch синхронно меняет CR3, trusted GS kernel-stack/FX pointers и TSS.RSP0.
3. `Exit` и user exception переводят process в Finished; отдельный kernel reaper выполняется уже на собственном kernel stack и освобождает user mappings, page tables, frames, FX state и task record только после switch-away.
4. Не освобождать стек, на котором ещё выполняется код или который может быть восстановлен. Сначала доказать безопасный handoff.
5. Добавить отдельный QEMU runtime test именно для production manager: не менее двух Ring 3 процессов, реальное yield-переключение, exit/fault, reaper, capability denial/no side effects, корректный CR3/TSS/FX и полная reclamation.
6. Все P5/P6 и базовые P7 сценарии сохранить в CI как регрессии. После успешного локально-эквивалентного QEMU CI сразу переходить к следующей запланированной задаче без ожидания отдельного одобрения.

## Платформенные ограничения, которые нельзя скрывать
- BSP/single-CPU only: AP-local GS setup, SMP process scheduling and TLB shootdown not implemented.
- User TLS is not implemented. NMI/exception handling during SWAPGS transition window is not claimed.
- RAMFS is in-memory; userland filesystem syscalls and persistent storage are future work.
- Production init currently has one process; `Yield` is a same-process resume until P7.1 scheduler integration is completed.

## Правила общения/работы
- Общение на русском.
- Двигаться последовательными законченными порциями: код → tests/build → CI → QEMU runtime → wiki.
- Использовать фактические репозиторий/CI данные; не повторять вопросы, ответы на которые уже есть в Wiki.
- При отчёте пользователя о локальном QEMU прогоне сразу продолжать следующую запланированную задачу, если нет реального блокера или риска данных.
- Wiki обновлять в том же цикле, а не откладывать «на потом».
