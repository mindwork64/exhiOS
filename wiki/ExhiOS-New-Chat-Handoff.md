# ExhiOS — передача контекста в новый чат

## Текущий статус

- Репозиторий: https://github.com/mindwork64/exhiOS
- Основная ветка: `main`.
- Текущий опубликованный релиз: [ExhiOS 0.1.0-alpha.1](https://github.com/mindwork64/exhiOS/releases/tag/0.1.0-alpha.1), GitHub pre-release от 2026-11-10.
- Release tag `0.1.0-alpha.1` — аннотированный tag, указывает на commit [e223030](https://github.com/mindwork64/exhiOS/commit/e223030b22cbb39510817ee494f1537677301e02). Документационные follow-up коммиты в `main` не двигают и не переписывают этот тег.
- Лицензия исходного кода: Apache-2.0 (`LICENSE`). Third-party dependencies сохраняют собственные лицензии.
- Последний release image был собран в Release profile, проверен через QEMU/OVMF и достиг интерактивной Ring 3 COM1 shell. GPT/FAT32 partition verification и byte-for-byte ESP readback прошли.
- Проверены shell-команды `help`, `hostname`, `version`, `config`, `echo`, `spawn`. Worker подтвердил `TaskSpawn → AccessDenied`, прошёл yield и завершился. После первого worker четыре последовательных replacement worker были успешно созданы после deferred reclamation и чисто завершились.
- Clippy для `exhi-init --bins` и kernel `process-runtime` завершился без ошибок с существующими style suggestions. `cargo fmt --all -- --check` прошёл. Archive и image SHA-256 manifests прошли `sha256sum --check`.
- Ограничения alpha: BSP/single-CPU; физический allocator ниже 4 GiB; RAMFS volatile; нет persistent filesystem, desktop, networking, broad device support или physical-hardware qualification. Shell пока не имеет cursor-key navigation и command history.

## Что читать

1. [[ExhiOS Current State]] — последнее runtime evidence и Release 0.1.0-alpha.1.
2. [[ExhiOS Roadmap]] — принятые этапы и будущая работа.
3. [[ExhiOS Architecture]] — текущие подсистемы и границы.
4. [[ExhiOS Decisions]] — обязательные архитектурные решения.
5. [[ExhiOS Development Log]] — история изменений и runtime checkpoints.

## Репозиторий и рабочий процесс

- Wiki source of truth — Markdown в `wiki/` основного репозитория.
- Локальное обновление пользователя: `git fetch origin`, затем `git reset --hard origin/main`, затем `cargo fmt`. Не предлагать `git pull`.
- Пользователь запускает локальные build/test/Clippy/QEMU проверки. Не запускать GitHub Actions/CI, сборки, тесты, Clippy или QEMU со стороны ассистента; выполнять code writing, GitHub static review и документацию. Не утверждать, что проверка пройдена без пользовательского лога.
- Не запускать и не перезаписывать release assets или уже опубликованный tag ради документационных изменений.
- Не менять Release tag при обычных документационных коммитах.
- Commit messages для docs-only изменений заканчивать на `[skip ci]`; GitHub Actions не запускать.
- При изменениях архитектуры синхронизировать Current State, Roadmap, Architecture/Decisions и Development Log. Handoff обновлять при изменении milestone или рабочего процесса.

## Текущая архитектура и scope

- Rust Stable; native x86-64 UEFI loader, freestanding kernel, COM1 serial shell.
- Physical allocator, four-level paging, 4 KiB VMM, kernel heap, private user address spaces, GDT/IDT/TSS, double-fault IST1 and guarded kernel stacks.
- Common SYSCALL entry/return, safe user copies, default-deny capabilities.
- Scheduler/idle/reaper; production `process-runtime` has a fixed two-slot registry for init + one worker. Init alone can spawn a worker. Worker cleanup is deferred until after switch-away; next authorized spawn reclaims a Finished worker before reusing the slot. One active worker slot, not arbitrary multi-process service or SMP.
- `libexhi` wrappers, interactive COM1 Ring 3 shell, config parsing and volatile kernel RAMFS.
- The final Release QEMU evidence is in Current State / Roadmap / Development Log.

## Следующий milestone: P8 — Native ExhiOS Shell

P7.1–P7.4 и релиз 0.1.0-alpha.1 приняты. Следующая разработка начинается не с kernel shell и не с Bash-клона, а с улучшения уже работающего собственного Ring 3 shell в `exhi-init`.

### Текущий shell
- Реализован в `exhi-init/src/main.rs`; COM1 input/output через `libexhi::console_read` и `console_write`.
- Команды: `help`, `hostname`, `version`, `echo`, `clear`, `config`, `spawn`, `exit`.
- Редактор принимает printable ASCII до 127 байт, CR/LF/CRLF, Backspace/DEL, Ctrl+C и Ctrl+D на пустой строке.
- ANSI cursor-key sequences не разбираются; Up/Down/Left/Right и history отсутствуют. Up-arrow может превращаться в `[A` в command buffer.
- `libexhi` уже предоставляет ConsoleRead/ConsoleWrite. Kernel syscall ABI для line editing менять не требуется на старте.

### P8.1 — первая задача нового чата
Сделать bounded interactive line editor и command history:
1. Сначала изучить `exhi-init/src/main.rs`, `libexhi/src/lib.rs`, `exhi-abi/src/syscall.rs`, `documentation/USERLAND.md` и связанные host-test patterns.
2. Спроектировать небольшой host-testable `no_std` core (предлагаемое имя crate — `exhi-shell`); оставить serial I/O в `exhi-init`.
3. Добавить инкрементальный ANSI/VT100 CSI parser, устойчивый к тому, что escape sequence разбита между разными `ConsoleRead` chunks.
4. Добавить Left/Right/Home/End/Delete, Backspace, Up/Down и bounded in-memory history (первоначально 8 команд). Сохранить текущие Ctrl+C, Ctrl+D на пустой строке и CR/LF/CRLF semantics.
5. Использовать фиксированные буферы и не выделять память динамически. Начальный input остаётся printable ASCII, максимум 127 байт.
6. Покрыть editor/parser host tests, затем подготовить QEMU interactive acceptance steps для пользователя. Не заявлять runtime success до получения его лога.

**Не смешивать P8.1 с parser rewrite или запуском произвольных приложений.** P8.2 отдельно займётся quoted arguments и bounded command dispatch. Generic ELF launch требует отдельного процесса/loader API и userland filesystem syscalls; сейчас `spawn` запускает только встроенный worker. Pipelines, redirection, variable expansion, scripts и Bash/POSIX compatibility пока вне scope.

### Другие будущие направления
- Userland VFS/directory/file syscalls и persistent storage.
- Общий capability-gated process API и запуск ELF-программ из файловой системы.
- APIC/IOAPIC, AP-local GS, SMP scheduling/TLB shootdown, user TLS, broader device support и hardware qualification.

## Правила работы

- Общение на русском.
- Изменения вносить законченными логическими порциями; одна функциональность на этап.
- Приоритет — корректность, проверяемые факты и отсутствие выдуманных runtime results.
- Не повторять вопросы, на которые уже есть ответы в этом handoff или актуальной Wiki.
