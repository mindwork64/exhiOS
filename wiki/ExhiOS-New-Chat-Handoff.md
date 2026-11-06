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

## Последний подтверждённый checkpoint
- Main commit: 207df7ffc86d97cdc15f32b8231ee2d1176f2139.
- P2.2 bootstrap page tables are integrated and runtime-verified.
- CI run: https://github.com/mindwork64/exhiOS/actions/runs/42205457670
- Headless QEMU + OVMF booted the UEFI loader and kernel.
- PML4 root = active CR3 = 0x14E000.
- Six page-table pages; 4 GiB identity map using 2 MiB pages.
- Nine critical ranges verified before and after CR3 switch.
- Kernel entry 0x100000 translated to physical 0x100000.
- Serial success line emitted; QEMU debug-exit status 33.
- Build job is clean (no Rust compiler warnings); runtime job passed.

P2.2 is complete. The user's local Linux Mint/QEMU run is an additional environment confirmation, not a blocker for the CI-verified milestone.

## Следующая задача — P2.3 Virtual Memory Manager
Implement in small complete increments:
1. Add dynamic page-table allocation after kernel identity map is active; allocator frames below 4 GiB are now identity-accessible.
2. Support 4 KiB map/unmap and translation while preserving the existing 2 MiB bootstrap mappings.
3. Split a 2 MiB huge PDE into a 4 KiB page table when a sub-page mapping is requested.
4. Add map flags (present/writable/user, and NX only after safe EFER.NXE capability handling).
5. Add INVLPG/TLB invalidation and safe cleanup/release of empty page-table levels.
6. Add a QEMU smoke test that maps a spare physical frame at a non-identity virtual address, checks translation, unmaps it, and verifies absence; release the frame only after unmap.
7. Update Roadmap and Wiki; require CI compile + QEMU runtime evidence.

Local environment confirmation command:
```bash
git fetch origin
git reset --hard origin/main
EXHIOS_QEMU_EXIT_TEST=1 ./scripts/run-qemu.sh
```

## Правила работы
- Русский язык общения.
- Конкретные порции готового кода, коммиты в main, CI, затем runtime-тест.
- Не задавать повторно вопросы, ответы на которые есть в Wiki.
- Не считать compile success runtime success.
- По ходу работы обновлять Wiki и ROADMAP.
