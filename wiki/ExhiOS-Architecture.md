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

## P2.2 — Bootstrap paging (runtime verified)

Файл exhi-kernel/src/arch/x86_64/paging.rs подключён к kernel startup после P2.1. Строит PML4 → PDPT → четыре PD, identity map 0–4 GiB через 2 MiB supervisor RW pages, переключает CR3.

Шесть страниц таблиц находятся в выровненном kernel .bss. Физические кадры allocator не разыменовываются до первого kernel-owned map. Перед и после CR3 switch проверяются kernel code, stack, BootInfo, UEFI memory map, framebuffer, ACPI RSDP, GDTR, IDTR и таблицы. translate_current проверяет kernel entry translation.

CI build и headless QEMU/OVMF runtime прошли: root/CR3 0x14E000, map 4 GiB, 9 проверенных диапазонов, kernel entry 0x100000 → 0x100000, QEMU debug-exit status 33. Подробности: GitHub Actions run 42205457670.

Ограничения bootstrap map: RWX, supervisor-only, 2 MiB granularity; нет W^X, user mappings, per-device cache attributes, process address spaces и общей VMM.

## Устройства
COM1 base `0x3F8` используется для serial output. GOP framebuffer приходит через BootInfo, но текстовая framebuffer console ещё не реализована. QEMU: q35, OVMF, 512 MiB RAM, ESP на USB storage, serial stdio.

## P2.3 — Initial Virtual Memory Manager

New module: \`exhi-kernel/src/arch/x86_64/vmm.rs\`.
- 4 KiB \`map_page\`, \`unmap_page\`, \`query_page\` over the bootstrap 0..4 GiB identity window.
- Huge 2 MiB PDE can be split into 512 equivalent 4 KiB leaves.
- PageFlags represent writable, user, executable, write-through, cache-disable and global. NX is used only after CPUID feature detection and EFER.NXE activation.
- Changed leaves are invalidated with INVLPG. Mutating operations require interrupts disabled.
- Coalescing only occurs after checking all 512 leaves for presence, physical contiguity, alignment and identical permissions/cache attributes. A/D bits are merged. All 512 virtual pages are invalidated before releasing the no-longer-referenced page-table frame.
- No independent address spaces, mappings above 4 GiB, SMP locks or Ring 3 permission tests yet. Those belong to later iterations.
- QEMU runtime smoke test passed in CI run 42213581974; user's local verification remains pending.
