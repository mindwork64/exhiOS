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
- HEAD main: [8f0b4bf](https://github.com/mindwork64/exhiOS/commit/8f0b4bf5f854b06b2f4d9452b408baf5a0a719d9).
- Последний полный CI [#484](https://github.com/mindwork64/exhiOS/actions/runs/42597997791): build job — success (50 шагов, 0 Rust warnings); QEMU job — success (27 завершённых job steps, в том числе все 21 runtime scenario).
- P0–P6 закрыты по применимым host/build/QEMU evidence.
- P5.1–P5.7: Ring 3 entry/isolation, common SYSCALL, pointer validation/safe-copy, отдельный двухзадачный scheduler lifecycle и default-deny capabilities приняты runtime suite. Scope BSP/single-CPU.
- P6: bounded allocator-free VFS/RAMFS, file/directory operations and kernel smoke test accepted. Userland FS syscalls/persistent disk FS are absent.
- P7.1: production runtime now registers init and a blocked kernel-reaper task in `exhi-task::Scheduler`. Init `Exit` and validated CPL3 #UD fault both switch away to the reaper; IDT fault path explicitly executes `SWAPGS`; reaper restores kernel CR3, releases process mappings/frames and reaps the Finished task record on its dedicated guarded stack. CI #484 verifies both paths.
- P7.1 remains open only for the next layer: the production manager still registers one user process; there is no process-spawn/registry and no production multi-user dispatch yet. `Yield` returns scheduler `Continue` while no other user task is Ready.
- P7.2: `libexhi` wrappers for console_write/read, yield_now and exit are implemented and tested (CI #435).
- P7.3: interactive Ring 3 serial shell and QEMU serial input tests passed (CI #447).
- P7.4: bounded `key=value` parser and embedded hostname/prompt/version config are implemented. Persistent VFS config loading remains open.

## Ближайшая задача: production multi-process support for P7.1
1. Replace init-specific singleton ownership with a process registry/manager holding each process's LoadedProcess/address space, saved user frame, guarded kernel stack, FXSAVE state, and capabilities.
2. Generalize `select_process_context` and syscall services to resolve the currently selected user task. Remove the hard-coded `INIT_TASK_ID`/ `INIT_PROCESS` assumptions from ConsoleRead/ConsoleWrite and process termination paths.
3. Add a second production user process with independent CR3, user/kernel stacks, FX state and saved context; prove that `Yield` switches between both Ready processes.
4. Generalize Exit and CPL3 fault handling so any user process becomes Finished and the kernel reaper reclaims it only after switch-away. Preserve the explicit IDT-originated `SWAPGS` handling and BSP-only assumptions.
5. Add a QEMU production-runtime test covering two user processes, capability denial/no side effects, yield/exit/fault, CR3/TSS/GS/FX switching, reaper reclamation and Finished task-record removal.
6. Keep the existing P5/P6 and P7.1 init/shell/fault scenarios as regressions. Continue after successful CI/QEMU validation without waiting for a separate approval.

## Платформенные ограничения, которые нельзя скрывать
- BSP/single-CPU only: AP-local GS setup, SMP process scheduling and TLB shootdown not implemented.
- User TLS is not implemented. NMI/exception handling during SWAPGS transition window is not claimed.
- RAMFS is in-memory; userland filesystem syscalls and persistent storage are future work.
- Production init currently registers one user process plus a kernel reaper. `Yield` goes through the scheduler but resumes init when no other user task is Ready; multi-process creation/dispatch is the remaining P7.1 work.

## Правила общения/работы
- Общение на русском.
- Двигаться последовательными законченными порциями: код → tests/build → CI → QEMU runtime → wiki.
- Использовать фактические репозиторий/CI данные; не повторять вопросы, ответы на которые уже есть в Wiki.
- При отчёте пользователя о локальном QEMU прогоне сразу продолжать следующую запланированную задачу, если нет реального блокера или риска данных.
- Wiki обновлять в том же цикле, а не откладывать «на потом».
