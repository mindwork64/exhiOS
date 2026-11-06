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
- Основной CI использует self-hosted runner labels `[self-hosted, linux, x64]`; runner уже имеет Rust/binutils/QEMU/OVMF/disk tools, поэтому job не устанавливает пакеты через apt. Fork pull requests не исполняют код на self-hosted runner.
- Обновление локального репозитория: `git fetch origin` и `git reset --hard origin/main`. Не предлагать `git pull --ff-only`.
- QEMU запускать из корня: `./scripts/run-qemu.sh`.
- Пользователь присылает serial output для runtime проверки. Не объявлять этап закрытым без него.

## Последняя подтверждённая точка
- P0/P1, P2.1, P2.2, P2.3, P2.4, P2.5, P2.5.1, P2.6 и P3.1–P3.5 закрыты по применимым локальным + CI runtime/host evidence.
- P2.5.1: use-after-destroy, idempotent destroy, active-destroy rejection, W^X, invalid/duplicate mappings, double-unmap, page-table frame validation and retry-safe teardown.
- P2.5.1 CI QEMU/OVMF: [run 42238977281](https://github.com/mindwork64/exhiOS/actions/runs/42238977281); local Linux Mint/QEMU log also passed with debug-exit 33.
- P2.5 is Ring 0 address-space management only; actual CPL3 execution, syscalls and hardware-enforced user/supervisor fault tests are not implemented.

## Текущий кодовый checkpoint
- Current P3.4 implementation commits: [993b651](https://github.com/mindwork64/exhiOS/commit/993b651b7dd4cffa9aeffb6331e325dd1d8b190a) and [eafdbd1](https://github.com/mindwork64/exhiOS/commit/eafdbd1305ec7f3ae5b1f29f26d85bf317662310).
- P2.6 implementation: 64-bit TSS in GDT (selector 0x18), TR loaded with LTR, 16 KiB aligned IST1 stack, IDT vector 8 uses IST1; exception assembly passes normalized frame pointer to the Rust #DF handler.
- BootInfo ABI was bumped to magic `EXHIOS02`, version 2, and now carries kernel stack base/size.
- UEFI loader allocates a contiguous 17-page LOADER_DATA region: first page is a reserved guard page; remaining 16 pages form the 64 KiB kernel stack.
- Kernel reserves the guard physical page before later allocator use, then removes its identity mapping through VMM after the UEFI map is parsed. It verifies the guard is NotMapped and the stack remains supervisor mapped.
- QEMU debug-exit test sets RSP to the first byte of the mapped kernel stack and reads from the guard page. Page-fault frame delivery hits the same guard page and escalates to #DF; CPU switches to IST1.
- Headless CI/OVMF runtime: [run 42256850017](https://github.com/mindwork64/exhiOS/actions/runs/42256850017) passed. Local Linux Mint/QEMU log on 2026-11-06 also passed: guard 0x1E0D3000, stack 0x1E0D4000..0x1E0E4000, guard unmapped, stack supervisor-only, #DF frame pointer 0x122FC8 inside IST1 0x11F000..0x123000, debug-exit 33. P2.6 is closed.
- P3.1 adds a reusable polling 16550 SerialPort driver in `arch/x86_64/serial.rs`. COM1 output is migrated from main.rs; RX adds non-blocking try_read_byte and polling read_byte_blocking. Boot prints the P3.1 readiness marker.
- RX-inclusive driver build + headless QEMU regression [run 42258141119](https://github.com/mindwork64/exhiOS/actions/runs/42258141119) passed before the interactive test harness was added. The CI boot trace proves TX/boot regression, not receipt of input bytes.
- Commits [a58220e](https://github.com/mindwork64/exhiOS/commit/a58220e5fda9afb069b71f08529ebc68f95a4d36) and [6014dec](https://github.com/mindwork64/exhiOS/commit/6014dec505682a4d91ff7165d222f274a15561da) add the opt-in interactive RX byte-echo mode. Run `EXHIOS_SERIAL_RX_TEST=1 ./scripts/run-qemu.sh`; type ASCII, test Enter/Backspace, then press Ctrl+D to print the received-byte count. This mode has not yet been locally exercised.
- P2.6 and P3.1 are closed with local + CI evidence. P3.1 local RX log: `hello exhiOS` echoed, Enter produced a new prompt, Ctrl+D printed byte count 0x0D.
- P3.2 initial GOP framebuffer console is implemented in `exhi-console` and kernel startup. It supports GOP formats 0/1, visible width + separate scanline stride, ASCII text, wrap and scroll. Host tests, build variants and headless QEMU runtime passed in CI [run 42272107662](https://github.com/mindwork64/exhiOS/actions/runs/42272107662). User's local QEMU serial log (2026-11-06) confirms GOP framebuffer 0x80000000, 1280x800, stride 1280, pixel format 1, successful console initialization, kernel handoff and halt loop. User-provided screenshot description confirms the expected visible strings for kernel banner, framebuffer ready, 1280x800, stride 1280, BGRX, COM1 ready and kernel handoff success. Fine visual details (font sharpness, spacing, artifacts) were not independently assessed. P3.2 initial functional acceptance is complete.
- Follow-up commits [c3d2b8c](https://github.com/mindwork64/exhiOS/commit/c3d2b8ccaa34c6b399b7f12905ae11658455916b) and [558b394](https://github.com/mindwork64/exhiOS/commit/558b394f8b5d8dc66d3cd565e21302bbffde049a) add an allocation-free `KernelConsole` writer. After GOP setup it mirrors byte and formatted output to COM1 + framebuffer; on GOP failure it remains serial-only. Early startup diagnostics remain COM1-only. `write_fmt` uses `core::fmt` without heap allocation. CI build + headless QEMU runtime passed in [run 42275788072](https://github.com/mindwork64/exhiOS/actions/runs/42275788072); local QEMU serial output confirms unified-writer readiness and final handoff/halt messages.

## Последняя подтверждённая точка — P3.5 закрыт

- User local QEMU log (2026-11-06): registry summary `registered=3 initialized=1 failed=0`; COM1/Serial/Initialized, PIT/Clock/Registered, i8042 PS/2/Input/Registered; runtime pass marker and debug-exit status 33.
- Self-hosted CI workflow migration commit [f161f2a](https://github.com/mindwork64/exhiOS/commit/f161f2a967b4fe0194c05ea968ebab87ed814202). Run [42301446350](https://github.com/mindwork64/exhiOS/actions/runs/42301446350) passed build, host tests, standard boot, PIT and registry QEMU runtime.
- P3.5 is fully closed.

## Следующие действия
1. P4.1: validate the host-testable `exhi-task` no_std model and round-robin policy on self-hosted CI.
2. P4.2: implement per-task kernel stacks, x86-64 context switch (callee-saved registers + RSP, SysV alignment), first-task bootstrap and a two-task cooperative QEMU runtime test.
3. P4.3: integrate tick accounting and later preemption separately; do not call Rust or scheduler from the minimal IRQ0 assembly stub.
4. Ring 3, syscalls, user-pointer validation and hardware-enforced user/supervisor fault tests remain P5. TSS.RSP0 must be updated when per-task kernel stacks arrive; keep dedicated double-fault IST1 and boot-stack guard separate.

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
