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
- Ring 3 execution, supervisor isolation, syscalls, safe user copies and capabilities are implemented under P5; current runtime remains BSP/single-CPU.


## Exception and kernel-stack hardening (accepted P2.6)

- GDT contains a 64-bit TSS (selector 0x18), loaded by LTR. IST1 points to a dedicated 16 KiB aligned kernel BSS stack.
- IDT vector 8 (#DF) uses IST1. Exception stubs normalize CPU-pushed error-code frames; the handler validates the frame lies inside IST1.
- UEFI reserves a contiguous 17-page region: one guard page followed by the 64 KiB boot kernel stack. Kernel reserves and unmaps the guard before use.
- QEMU deliberately faults on the guard so delivery escalates to #DF; IST1 receives the frame and the test exits with status 33. P2.6 is implemented and runtime-verified, not planned.

## User-mode and process architecture (P5–P7)

- User mappings are private to an address space; the shared kernel identity/heap mappings remain supervisor-only.
- User entry and syscall handling require IF=0 while changing CR3, TSS.RSP0 and trusted entry state. The first implementation is BSP/single-CPU.
- Common SYSCALL entry uses a 64-byte-aligned GS-relative boot-CPU context. SYSCALL is not enabled until GS state, trusted kernel stack, FXSAVE pointer and dispatcher are installed and verified.
- Syscall authorization is default-deny. The common dispatcher owns syscall-to-capability mapping and checks grants before service callbacks. Missing/kernel tasks resolve to empty user-service grants.
- User pointers are not dereferenced directly: validate canonical bounds, all touched pages and effective permissions first; copy helpers validate the complete range before copying.
- Address-space teardown must switch to the kernel root first, unmap owned user pages, return data frames, then release private page-table structures. A running/current task must never be reaped.
- After selecting a user task and activating its CR3, the architecture layer must update TSS.RSP0 and the trusted active kernel-stack bounds used by CPL3 exception-frame validation as one serialized IF=0 operation. The P5.6 context selector now uses `user::activate_kernel_stack`; verified in CI #453.
- Production process scheduler integration is not yet complete: current init runtime is single-process and Yield resumes the same init. Do not confuse the P5.6 scheduler smoke adapter with a production process manager.
- AP-local GS state, user TLS, NMI/exception handling across the SWAPGS window, SMP scheduling and TLB shootdown are outside the accepted BSP-only scope.
- VFS/RAMFS is currently in-memory and kernel-integrated. Userland filesystem syscalls and persistent storage are future work.
