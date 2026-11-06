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
| `exhi-heap` | no_std first-fit heap allocator, host-tested |

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
- VMM separates virtual and physical bounds: lower canonical virtual addresses below 512 GiB are accepted; mapped physical frames remain below 4 GiB. Missing PDPT children are created on demand.
- Local Linux Mint/QEMU run on 2026-11-06 confirmed NX, split, RW supervisor NX read/write, unmap/TLB invalidation, identity restore, table reclaim and debug-exit 33; CI runtime passed in run 42213581974.
- Independent address spaces with separate PML4 roots, physical memory beyond 4 GiB, SMP shootdown and Ring 3 permission tests remain future work.

## P2.4 — Kernel Heap (initial implementation)
- `exhi-heap` provides no_std first-fit free-list allocation, alignment, splitting, deallocation, coalescing and reuse; four host unit tests pass.
- Kernel `GlobalAlloc` uses a virtual arena starting at 4 GiB, initially four pages, max 64 MiB; physical frames are mapped RW+NX from the allocator's managed range.
- Heap grows on demand through VMM. It serializes allocator operations with an atomic spinlock and disables local interrupts while held. Current SMP assumption: one CPU.
- Startup smoke test requests a 48 KiB aligned block to force growth, writes/reads across 4 KiB page boundaries, frees it and checks reuse/coalescing.
- CI build and host tests passed in run 42215253674; QEMU runtime job had not yet executed the heap smoke test at the time of this snapshot.
