# ExhiOS — дорожная карта

## Приёмка
Этап считается завершённым только после требуемого runtime evidence. CI/build success недостаточно.

## P0 — UEFI boot chain
**Завершено.** UEFI entry, ELF64 validation, PT_LOAD load, zero-fill, GOP/ACPI, final memory map, ExitBootServices и kernel handoff проверены в QEMU.

## P1 — Kernel foundation
**Завершено.**
- P1.1 GDT; CS/DS/ES/SS подтверждены.
- P1.2 IDT и exceptions 0–31; INT3 дошёл до handler.
- P1.3 PIC remap/masks и QEMU debug-exit status 33.

## P2 — Memory management

### P2.1 Physical page allocator
**Проверено в QEMU.** Descriptor version 1, stride 48; Conventional Memory only; первые 1 MiB зарезервированы; limit 4 GiB; managed/allocated bitmaps; allocate/free/reuse. 7 host tests прошли. QEMU показал 118 837 managed pages и повторно выдал frame 0x14C000.

### P2.2 Bootstrap page tables
**Код подключён и успешно собран в CI; QEMU runtime pending.**
- [x] Ревью и безопасная схема bootstrap storage.
- [x] Экспортировать модуль и вызвать после P2.1 при IF=0.
- [x] Создать PML4 → PDPT → 4 PD в выровненном kernel .bss.
- [x] Identity map 0–4 GiB через 2 MiB pages.
- [x] Переключить CR3; проверять критические ranges до и после.
- [ ] QEMU serial smoke test: увидеть CR3/root, успешные translations и debug-exit status 33.
- [ ] Только после runtime evidence закрыть P2.2.

### P2.3 Virtual memory manager
- [ ] 4 KiB map/unmap и translation.
- [ ] Page permissions: present/writable/executable/supervisor.
- [ ] TLB invalidation и безопасное освобождение таблиц.
- [ ] Kernel/process address spaces.

### P2.4 Kernel heap
- [ ] Heap allocator поверх физических страниц и VMM.
- [ ] Allocation/free/reuse tests.

## P3 — Basic devices
- [ ] Serial abstraction, framebuffer console, timer, keyboard, driver registration.

## P4 — Tasks and scheduling
- [ ] Task representation/stacks, context switching, cooperative scheduler, timer preemption.

## P5 — User-mode isolation
- [ ] Ring 3, per-process address spaces, syscalls, user-pointer validation, capabilities.

## P6 — Storage
- [ ] VFS, RAMFS, file/directory operations.

## P7 — Userland
- [ ] init, libexhi, serial shell, basic utilities.
