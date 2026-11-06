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
- Первая карта identity-maps первые 4 GiB; все используемые kernel/firmware handoff ranges должны укладываться в этот диапазон.
- Bootstrap tables живут в выровненном kernel .bss, поскольку произвольные Conventional Memory frames нельзя безопасно разыменовывать до первого kernel-owned mapping.
- CR3 switch выполняется при IF=0; перед и после него проверяются code, stack, BootInfo, memory map, framebuffer, ACPI, GDT, IDT и таблицы.
- Bootstrap identity mapping использует 2 MiB RW supervisor pages и является временной, не конечной защитой.
- W^X, user/supervisor isolation, cache attributes и process address spaces — будущие этапы.
- Физический allocator может снабжать динамические таблицы после включения kernel identity map.

## Правила разработки
- Не закрывать этап только по CI.
- Runtime логи QEMU фиксировать в Current State и Development Log.
- Не освобождать firmware/loader ranges на основании догадок.
- Не разыменовывать произвольный физический адрес до подтверждённого mapping.


## Project documentation policy

- Wiki page sources of truth are Markdown files under `wiki/` in the main repository.
- Changes to `wiki/**` on `main` trigger `.github/workflows/publish-wiki.yml`.
- The publisher uses the repository-scoped workflow token with `contents: write`, and pushes to the paired `.wiki.git` repository.
- The managed block in Wiki `Home.md` is replaced idempotently. Other pages are upserted; unrelated Wiki pages are not deleted.
- Update Current State, Roadmap, Development Log and New Chat Handoff as milestones change.
