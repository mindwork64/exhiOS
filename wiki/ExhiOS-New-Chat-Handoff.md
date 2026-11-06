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
P0/P1 и P2.1 закрыты. P2.2 локально подтверждён пользователем: PML4=CR3=0x10C000, 4 GiB identity map, 9 диапазонов, kernel entry 0x100000, debug-exit 33.

## Текущий checkpoint
- Main commit: 08f179af25190092597dead6ad918082fa79b79b.
- P2.2 bootstrap paging завершён и локально проверен.
- P2.3 VMM реализован и прошёл headless QEMU/OVMF CI: https://github.com/mindwork64/exhiOS/actions/runs/42213581974
- Реализованы 4 KiB map/unmap/query, split 2 MiB→4 KiB, NXE/permission flags, INVLPG и safe coalescing/reclaim.
- CI serial подтвердил NX enabled, RW supervisor NX, запись/чтение, unmap/NotMapped, восстановление identity map, освобождение page-table frame и debug-exit 33.
- Локальный прогон P2.3 на Linux Mint пользователя ещё не получен; не объявлять P2.3 полностью закрытым.

## Следующее действие после локального отчёта
Сразу после того как пользователь присылает serial output локального QEMU:
1. Сверить P2.3 output и показатели с CI, найти и исправить возможные расхождения.
2. Зафиксировать локальное runtime evidence в Wiki/Development Log.
3. Немедленно перейти к следующей части без запроса отдельного одобрения. Следующий плановый этап — P2.4 Kernel Heap (heap allocator поверх физического allocator + VMM), сохраняя дальнейшее разбиение на небольшие законченные инкременты.

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
