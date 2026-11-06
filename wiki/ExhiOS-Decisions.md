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


## Kernel heap layout (P2.4)
- Keep the first 4 GiB virtual range as the bootstrap physical identity map.
- Kernel heap begins at virtual address 4 GiB and uses the lower canonical virtual window (VMM limit 512 GiB); this prevents heap allocations from replacing existing identity mappings.
- Heap maps physical frames below the current 4 GiB physical allocator limit as RW+NX.
- Initial heap region is four 4 KiB pages; growth is on demand, up to 64 MiB. The current implementation does not shrink mapped heap pages.
- Use a small no_std first-fit free-list allocator with in-band free blocks and allocation headers; split free ranges and coalesce adjacent free blocks.
- Expose it as the kernel GlobalAlloc only after explicit initialization during kernel startup. Operations are serialized by an atomic spinlock and local interrupts are disabled while the lock is held. Current kernel is single-core; SMP TLB shootdown/allocator lock design remains future work.


## Kernel address-space layout (P2.5)
- The kernel currently occupies PML4[0] and keeps its low 4 GiB identity map plus heap virtual range beginning at 4 GiB.
- New address spaces get private PML4 and PDPT pages. PDPT entries 0..4 are shared from the kernel root; entries 5..7 are reserved.
- User mappings begin at VA 8 GiB (PDPT[8]) and are limited to the current VMM window below 512 GiB (PDPT[8..511]).
- The PML4[0] USER bit may be set for user pages, but shared kernel/heap mappings remain supervisor-only because their lower-level entries retain U/S=0.
- map_user_page is unsafe: caller owns frame, keeps it allocated until unmap and scrubs before exposing it to untrusted code. unmap returns data-frame ownership to caller; destroy only reclaims private page-table structures.
- CR3 switches require IF=0. Current design is single-core and assumes no PCID/global mappings; SMP TLB shootdown is future work.
- Actual Ring 3 execution and hardware privilege fault verification remain separate user-mode work.


## Exception and kernel-stack hardening (planned P2.6)
- Existing exception vectors 0..31 have assembly stubs and a fatal Rust handler; INT3 was runtime-tested and the handler reports vector/error code/RIP/CS/RFLAGS and CR2 for #PF.
- Current handlers run on the interrupted kernel stack; no TSS or IST is installed yet. A stack-overflow fault can therefore escalate to double fault/triple fault before diagnostics.
- Install a TSS with a dedicated IST stack for #DF before enabling a guard page below the boot kernel stack.
- Verify the #DF path in QEMU before treating the guard page as an active protection mechanism.
