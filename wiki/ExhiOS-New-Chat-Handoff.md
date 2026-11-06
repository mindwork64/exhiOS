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
- P0/P1, P2.1, P2.2 и P2.3 завершены.
- P2.3 local QEMU (2026-11-06): CR3=0x110000, 4 GiB bootstrap map, 9 ranges; NX enabled; 2 MiB→4 KiB split; RW supervisor NX write/read; unmap/TLB invalidation; identity restore and page-table reclaim; debug-exit 33.

## Текущий checkpoint
- Code checkpoint: [f03e92b](https://github.com/mindwork64/exhiOS/commit/f03e92b7239cd75b95ff1dffc1a4dce053a74f30).
- P2.4 first implementation committed. `exhi-heap` no_std free-list crate (4/4 tests); kernel `GlobalAlloc`, virtual heap base 4 GiB, initial 4 pages, growth to 64 MiB.
- VMM virtual window expanded to lower canonical 512 GiB; physical frame targets remain below 4 GiB. Heap mappings RW+NX.
- Heap startup smoke test integrated: aligned small allocation, 48 KiB aligned growth test, cross-page writes/reads, free/reuse/coalescing.
- ABI tests 2/2, ELF 12/12, physical memory 7/7, heap 4/4, UEFI + all kernel build variants succeeded in CI run [42215253674](https://github.com/mindwork64/exhiOS/actions/runs/42215253674).
- QEMU job was still installing packages at last check. P2.4 is not runtime-closed until headless CI and local serial output print the heap success markers and debug-exit 33.

## Следующее действие после локального отчёта
После того как пользователь присылает serial output локального QEMU:
1. Сверить P2.4 startup markers, heap base/page count, growth/read-write/free/reuse and debug-exit 33.
2. Зафиксировать runtime evidence в Wiki.
3. Немедленно начать следующий плановый этап **P2.5 Kernel Address Spaces** без запроса одобрения: separate PML4 roots, kernel mapping policy, create/destroy address spaces and safe private page-table reclamation. Сохранять цикл code → CI → user QEMU → immediately next work.

Команда локальной проверки:
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
