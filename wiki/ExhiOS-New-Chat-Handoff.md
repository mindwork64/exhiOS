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

## Следующие действия
1. P3.3 PIT timer and P3.4 PS/2 keyboard are closed with local + CI runtime evidence. P3.4 local QEMU log (2026-11-06) reports i8042 config 0x65, IRQ1 gate valid, key-down/key-up events, Ctrl+D recognition, 28 transitions, zero dropped scan codes, IRQ1 remasked and QEMU status 33. Shift was not independently visible in this log; decoder Shift tests passed on host.
2. P3.5 static driver registry is implemented in [fc72e1c](https://github.com/mindwork64/exhiOS/commit/fc72e1c789a49f9efdaf8698f91b645ec8db3dda); the dedicated runtime smoke mode is in [8fca30b](https://github.com/mindwork64/exhiOS/commit/8fca30b78eac0a2267ec27f7163bf98b4586bb5e). The no_std `exhi-driver` crate stores immutable descriptors, IDs/classes and Registered/Initialized/Failed states in a fixed-capacity array. COM1/PIT/PS2 publish descriptors; startup registers all three and prints summary. Host tests (8/8) and all build variants passed CI [run 42299327751](https://github.com/mindwork64/exhiOS/actions/runs/42299327751); headless runtime remains pending.
3. P3.5 local verification:
   ```bash
   git fetch origin
   git reset --hard origin/main
   cargo fmt
   cargo test
   EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_DRIVER_REGISTRY_TEST=1 ./scripts/run-qemu.sh
   ```
   Expect `P3.5: driver registry ready; registered=3 initialized=1 failed=0`, COM1/PIT/keyboard entries and `P3.5: driver registry runtime smoke test passed.` Normal boot should show the same summary; timer/keyboard smoke modes should transition their respective entries to Initialized.
4. Once P3.5 has local runtime evidence, proceed to P4 task representation and cooperative scheduling. Keep scheduler work separate from user-mode/Ring 3.
5. APIC/IOAPIC, HPET/TSC calibration and scheduler timer preemption remain later work. Ring 3, syscalls and hardware user/supervisor fault tests remain a later milestone. P2.6 guard page is locally and CI verified; do not remove or bypass it.

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
