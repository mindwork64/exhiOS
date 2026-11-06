# ExhiOS — архитектурные решения

Здесь фиксируются устойчивые решения, а не временные гипотезы.

## Проект
- Самостоятельная x86-64 ОС, Rust Stable, freestanding/no_std kernel.
- Собственный native UEFI loader; Linux kernel и GRUB не являются основой.
- Kernel target `x86_64-unknown-none`; loader target `x86_64-unknown-uefi`.
- QEMU + OVMF — первый runtime target.

## Boot contract
- Kernel ELF — fixed-address ET_EXEC.
- PT_LOAD physical и virtual addresses совпадают для текущего identity-loaded ядра.
- BootInfo — явный repr(C) ABI; address fields физические.
- До ExitBootServices loader закрывает protocol handles и освобождает временные heap allocations.
- Handoff — System V x86-64 ABI, отдельный 64 KiB stack.
- IF остаётся clear до готовности IDT и IRQ handlers.

## CPU/interrupts
- Kernel-owned GDT и IDT.
- CPU exceptions vectors 0–31 обслуживаются отдельными stubs.
- Legacy PIC смещён на 0x20–0x2F, все IRQ линии masked.
- APIC/IOAPIC не активировать до ACPI MADT parsing и handlers.
- Не включать interrupts до готовности gates, handlers и EOI.

## Physical memory
- Использовать firmware-reported UEFI descriptor stride, не sizeof.
- Пока поддерживается descriptor version 1.
- Только Conventional Memory (type 7) считается free.
- Loader/BootServices/Runtime/ACPI/MMIO и прочие типы зарезервированы.
- Первые 1 MiB reserved; первая allocator bitmap ограничена 4 GiB.
- Page size 4096 bytes.
- Ранний allocator используется на single core при IF=0; locking/SMP — будущий этап.

## Paging
- UEFI page tables сохраняются до подготовки kernel-owned tables.
- Первый переходный map обязан сохранять доступ к kernel, stack, BootInfo, memory map, table frames и используемым framebuffer/ACPI адресам.
- Bootstrap identity mapping 0–4 GiB через 2 MiB pages — временное решение, не конечная защита.
- W^X, user/supervisor isolation и process address spaces пока не реализованы.
- `paging.rs` в HEAD 572cce506… — черновик, ещё не экспортирован и не запущен.

## Правила разработки
- Не закрывать этап только по CI.
- Runtime логи QEMU фиксировать в Current State и Development Log.
- Не освобождать firmware/loader ranges на основании догадок.
- Не разыменовывать произвольный физический адрес до подтверждённого mapping.
