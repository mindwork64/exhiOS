# ExhiOS — архитектура

## Назначение и стек
Независимая x86-64 ОС на Rust Stable, freestanding/no_std kernel и собственный native UEFI loader. Это не Linux distribution; GRUB и Linux kernel не используются. Первичный runtime target — QEMU + OVMF.

## Boot chain
1. UEFI запускает `EFI/BOOT/BOOTX64.EFI`.
2. `exhi-boot` читает `\exhi-kernel.elf`.
3. `exhi-elf` валидирует ELF64 и PT_LOAD.
4. Loader размещает сегменты по точным физическим адресам, обнуляет zero-tail.
5. Получает GOP framebuffer и ACPI RSDP.
6. Резервирует BootInfo и 64 KiB stack.
7. Получает финальную UEFI memory map и выполняет ExitBootServices.
8. Assembly trampoline отключает interrupts, переключает стек и вызывает `_start(BootInfo*)` по System V x86-64 ABI.
9. Kernel устанавливает GDT, IDT и перенастраивает PIC.
10. Kernel инициализирует физический frame allocator.

## Cargo workspace
| Crate | Назначение |
|---|---|
| `exhi-abi` | repr(C) BootInfo ABI |
| `exhi-elf` | no_std ELF64 parser |
| `exhi-boot` | UEFI loader и handoff |
| `exhi-kernel` | Freestanding kernel |
| `exhi-memory` | no_std физический frame allocator |

## BootInfo
Содержит magic/version/struct_size, физический адрес и размер memory map, descriptor size/version, framebuffer metadata и ACPI RSDP address. Начальная загрузка сохраняет UEFI identity mapping.

## CPU foundation
**GDT:** null, code selector 0x08, data/stack selector 0x10; LGDT + far return; проверено в QEMU.

**IDT:** 256 entries, stubs CPU exception vectors 0–31, общий нормализованный stack frame, diagnostics vector/error code/RIP/CS/RFLAGS, CR2 для page fault; INT3 проверен в QEMU.

**PIC:** legacy 8259 remapped на 0x20–0x2F; обе маски 0xFF. IF остаётся clear до готовности IRQ handlers. APIC/IOAPIC отложены до ACPI MADT и handlers.

## Physical memory
- UEFI descriptor version 1; используется реальный firmware stride (в QEMU 48 байт).
- Только EfiConventionalMemory, type 7, считается свободной.
- Loader/firmware/runtime/ACPI/MMIO и остальные типы зарезервированы.
- Первые 1 MiB зарезервированы; первая bitmap управляет адресами ниже 4 GiB.
- Page size 4096 bytes; bitmap managed + allocated.
- Физический frame пока выдаётся как адрес; arbitrary dereference откладывается до page tables.

## P2.2 — Bootstrap paging

Файл exhi-kernel/src/arch/x86_64/paging.rs подключён к kernel startup после инициализации P2.1. Строит четырёхуровневые таблицы: PML4 → PDPT → четыре PD, identity map 0–4 GiB через 2 MiB supervisor RW pages, переключает CR3.

Шесть страниц таблиц размещены как выровненная статическая область kernel .bss. Не используем физические кадры allocator до первого switch: firmware page tables не обязаны отображать произвольные conventional frames по identity address. После включения kernel map такие кадры станут доступны для динамических таблиц будущего VMM.

До и после CR3 switch проверяются kernel code, stack, BootInfo, UEFI memory map, framebuffer, ACPI RSDP, GDTR, IDTR и сами page-table pages. Отдельная проверка translate_current проверяет kernel entry identity translation.

CI run 42204902818 собрал default kernel и оба smoke-test варианта. QEMU runtime ещё не подтверждён.

Ограничения bootstrap map: RWX, supervisor-only, 2 MiB granularity; нет W^X, user mappings, per-device cache attributes, process address spaces и общей VMM.

## Устройства
COM1 base `0x3F8` используется для serial output. GOP framebuffer приходит через BootInfo, но текстовая framebuffer console ещё не реализована. QEMU: q35, OVMF, 512 MiB RAM, ESP на USB storage, serial stdio.
