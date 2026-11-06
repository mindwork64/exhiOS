# ExhiOS — архитектура

## Назначение и стек
Независимая x86-64 ОС на Rust Stable, freestanding/no_std kernel и собственный native UEFI loader. Это не Linux distribution; GRUB и Linux kernel не используются. Первичный runtime target — QEMU + OVMF.

## Boot chain
1. UEFI запускает `EFI/BOOT/BOOTX64.EFI`.
2. `exhi-boot` читает `\exhi-kernel.elf`.
3. `exhi-elf` валидирует ELF64 и PT_LOAD.
4. Loader размещает сегменты по точным физическим адресам, обнуляет zero-tail.
5. Получает GOP framebuffer и ACPI RSDP.
6. Резервирует BootInfo v2 и единый 17-page LOADER_DATA block: один guard page и 64 KiB kernel stack.
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
| `exhi-driver` | no_std fixed-capacity driver inventory and lifecycle registry |

## BootInfo
BootInfo v2 содержит magic/version/struct_size, физический адрес и размер memory map, descriptor size/version, framebuffer metadata, ACPI RSDP address и физические base/size boot kernel stack. Magic: EXHIOS02, version: 2. Loader выделяет contiguous block из 17 страниц: первая — guard reservation, следующие 16 — 64 KiB stack. Начальная загрузка сохраняет UEFI identity mapping.

## CPU foundation
**GDT:** null, code selector 0x08, data/stack selector 0x10, 64-bit TSS descriptor selector 0x18. TSS загружается через LTR; IST1 указывает на отдельный 16 KiB aligned stack in kernel BSS.

**IDT:** 256 entries, stubs CPU exception vectors 0–31, общий нормализованный stack frame, diagnostics vector/error code/RIP/CS/RFLAGS, CR2 для page fault. Vector 8 (#DF) использует IST1. Общий exception stub передаёт frame pointer как шестой SysV argument; #DF handler проверяет, что frame лежит внутри IST1. INT3 и реальный #DF/IST1 проверены в QEMU.

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
COM1 base `0x3F8` обслуживается модулем `arch/x86_64/serial.rs`: reusable 16550-compatible `SerialPort`, 38400 baud, 8N1, FIFO, bounded polling TX, non-blocking `try_read_byte` и polling `read_byte_blocking`. Основной boot/exception logging использует `serial::COM1`; raw COM1 I/O удалён из `main.rs`. QEMU подключает UART как `-serial stdio`. Локально проверены TX и интерактивный RX: `hello exhiOS` echo, Enter prompt и Ctrl+D с byte counter 0x0D. Diagnostic mode остаётся доступен через `EXHIOS_SERIAL_RX_TEST=1 ./scripts/run-qemu.sh`; это не command shell.

GOP framebuffer приходит через BootInfo. Новый `exhi-console` no_std crate создаёт текстовую консоль без heap allocation: 5x7 ASCII glyphs в 12x16 cells, foreground/background colors, clear, text, decimal values, CR/LF/tab, wrap и scrolling. Поддерживаются UEFI pixel formats 0 (RGB-reserved) и 1 (BGR-reserved); BitMask и BltOnly явно отклоняются. Kernel проверяет physical alignment, dimensions, stride, buffer byte size и что весь используемый framebuffer диапазон помещается в текущий 0..4 GiB identity map. Renderer пишет volatile pixels и индексирует строки по `pixels_per_scan_line`, не по width. Первый экран выводит kernel banner, resolution, stride и pixel format. Host tests, kernel build variants and headless QEMU/OVMF runtime passed in [run 42272107662](https://github.com/mindwork64/exhiOS/actions/runs/42272107662); boot serial confirms framebuffer initialization and the P2.6 IST1 regression. Local QEMU serial confirms 1280x800, stride 1280, GOP format 1, initialization, handoff and halt; the user-provided screenshot description confirms the expected banner and all framebuffer status strings. Fine visual details were not independently inspected.

## P2.3 — Initial Virtual Memory Manager

New module: \`exhi-kernel/src/arch/x86_64/vmm.rs\`.
- 4 KiB \`map_page\`, \`unmap_page\`, \`query_page\` over the bootstrap 0..4 GiB identity window.
- Huge 2 MiB PDE can be split into 512 equivalent 4 KiB leaves.
- PageFlags represent writable, user, executable, write-through, cache-disable and global. NX is used only after CPUID feature detection and EFER.NXE activation.
- Changed leaves are invalidated with INVLPG. Mutating operations require interrupts disabled.
- Coalescing only occurs after checking all 512 leaves for presence, physical contiguity, alignment and identical permissions/cache attributes. A/D bits are merged. All 512 virtual pages are invalidated before releasing the no-longer-referenced page-table frame.
- VMM separates virtual and physical bounds: lower canonical virtual addresses below 512 GiB are accepted; mapped physical frames remain below 4 GiB. Missing PDPT children are created on demand.
- Local Linux Mint/QEMU run on 2026-11-06 confirmed NX, split, RW supervisor NX read/write, unmap/TLB invalidation, identity restore, table reclaim and debug-exit 33; CI runtime passed in run 42213581974.
- Independent address spaces are now in initial implementation: separate PML4/PDPT, shared supervisor-only kernel entries 0..4, private user PDPT entries 8..511. CI QEMU verified two roots mapping the same VA to different physical frames; local verification pending.
- Physical memory beyond 4 GiB, SMP TLB shootdown and actual Ring 3 execution/permission fault tests remain future work. TSS/IST and the boot-stack guard page are locally and CI runtime verified under P2.6.

## P2.4 — Kernel Heap (initial implementation)
- `exhi-heap` provides no_std first-fit free-list allocation, alignment, splitting, deallocation, coalescing and reuse; four host unit tests pass.
- Kernel `GlobalAlloc` uses a virtual arena starting at 4 GiB, initially four pages, max 64 MiB; physical frames are mapped RW+NX from the allocator's managed range.
- Heap grows on demand through VMM. It serializes allocator operations with an atomic spinlock and disables local interrupts while held. Current SMP assumption: one CPU.
- Startup smoke test requests a 48 KiB aligned block to force growth, writes/reads across 4 KiB page boundaries, frees it and checks reuse/coalescing.
- Host tests 4/4, local Linux Mint/QEMU and headless CI QEMU [run 42215755990](https://github.com/mindwork64/exhiOS/actions/runs/42215755990) passed. Both logs show 4 initial pages, 18 after growth, 0x12000 free bytes and debug-exit 33.


## P2.5 — Address-space manager (initial implementation)

Module: exhi-kernel/src/arch/x86_64/address_space.rs.
- initialize captures the active kernel CR3 after heap initialization.
- Each address space owns a PML4 and a private PDPT. It copies kernel PDPT entries 0..4: bootstrap identity mappings 0..4 GiB and heap mapping starting at 4 GiB. PDPT entries 5..7 remain reserved.
- User virtual range is 8..512 GiB, PDPT entries 8..511. Private PD/PT tables are allocated on demand.
- The root PML4 USER bit is enabled when a user mapping is added. Shared kernel/heap entries retain U/S=0 at lower page-table levels and stay supervisor-only.
- API: create, unsafe map_user_page (caller owns and scrubs frame), translate, unmap_user_page, activate, restore_kernel and destroy. Empty private page tables are reclaimed; data frames remain caller-owned.
- Headless QEMU/OVMF CI [run 42217123278](https://github.com/mindwork64/exhiOS/actions/runs/42217123278) verified two roots: same VA 0x200000000 maps to distinct frames, byte A1/B2 survives CR3 switches, kernel and heap translations persist, page tables/data frames are reclaimed, debug-exit 33.
- Safety hardening includes supervisor-only checks for kernel code, stack and heap, unsafe caller ownership/scrub contract, use-after-destroy guards, active-destroy rejection, W^X denial, invalid/duplicate map and double-unmap rejection, page-table frame validation and retry-safe teardown. P2.5.1 passed local QEMU and CI [run 42238977281](https://github.com/mindwork64/exhiOS/actions/runs/42238977281).
- Ring 3 execution, syscalls, user-pointer validation and hardware-enforced user/supervisor fault tests are not implemented yet. TSS/IST and boot-stack guard are implemented under P2.6; CI proof is [run 42256850017](https://github.com/mindwork64/exhiOS/actions/runs/42256850017), and local QEMU verification passed on 2026-11-06.



## Unified kernel console writer

Module: `exhi-kernel/src/console.rs`; implementation commits [c3d2b8c](https://github.com/mindwork64/exhiOS/commit/c3d2b8ccaa34c6b399b7f12905ae11658455916b) and [558b394](https://github.com/mindwork64/exhiOS/commit/558b394f8b5d8dc66d3cd565e21302bbffde049a).
- `KernelConsole` mirrors byte slices to COM1 and the optional GOP framebuffer.
- `write_fmt` uses `core::fmt` without heap allocation.
- It is instantiated only after GOP setup; prior boot diagnostics remain serial-only. If GOP setup fails, output remains available over COM1.
- QEMU debug-exit and final halt messages use the shared writer. It is single-core and polling-only, with no locking/IRQ support yet.
- CI build and headless QEMU runtime passed in [run 42275788072](https://github.com/mindwork64/exhiOS/actions/runs/42275788072). Local Linux Mint/QEMU serial output also confirms the unified writer readiness, kernel handoff and halt messages.

## P3.1 — Polling serial driver

Module: `exhi-kernel/src/arch/x86_64/serial.rs`.
- Reusable `SerialPort` accepts a UART base I/O port; `COM1` is configured at 0x3F8.
- Initialize as 38400 baud, 8N1, FIFO enabled; transmit bytes with bounded polling.
- TX API: `write_byte`, `write`, `write_hex`.
- RX API: non-blocking `try_read_byte` and polling `read_byte_blocking`.
- Driver is single-core and polling; no UART IRQs or interrupt-safe output locking yet. Early kernel diagnostics use COM1; after GOP setup, `KernelConsole` mirrors post-handoff output to COM1 and framebuffer.
- Headless CI build and QEMU regression passed [run 42270059656](https://github.com/mindwork64/exhiOS/actions/runs/42270059656). Local interactive byte reception is verified: `hello exhiOS` echoed, Enter prompted again and Ctrl+D ended the loop with 13 received bytes.

## P3.3 — PIT timer and monotonic ticks

Module: `exhi-kernel/src/arch/x86_64/timer.rs`; code checkpoint [d2ed6b2](https://github.com/mindwork64/exhiOS/commit/d2ed6b2d40260340396af1db8284b607eba1c4ad).
- PIT channel 0 runs in mode 2 at a requested 100 Hz using input clock 1,193,182 Hz and divisor 11,932 (approximately 99.9985 actual ticks/second).
- The IDT installs and validates a dedicated IRQ0 interrupt gate at vector 0x20.
- PIC APIs unmask or mask only master IRQ0 while preserving the state of all other lines. The timer setup leaves IF policy to its caller.
- The IRQ stub is pure assembly: saves RAX, uses a locked 64-bit increment of `EXHIOS_PIT_TICKS`, sends master PIC EOI and executes IRETQ. It makes no Rust calls, heap allocations or SIMD state changes.
- `timer::ticks()` reads the monotonic 64-bit counter; `timer::elapsed_ticks(start)` uses wrapping subtraction.
- The opt-in smoke test enables IF only after PIT/IDT/PIC setup, receives five real IRQ0 ticks, disables IF, masks IRQ0 again and verifies all other PIC lines remain masked.
- CI build + headless QEMU/OVMF passed in [run 42275788072](https://github.com/mindwork64/exhiOS/actions/runs/42275788072); local Linux Mint/QEMU log (2026-11-06) also reports five ticks, IRQ0 remasked and debug-exit status 33.
- This is the initial legacy PIT/PIC implementation; APIC/IOAPIC, HPET/TSC calibration, SMP delivery and scheduler integration are deferred.

## P3.4 — PS/2 keyboard and input events
## P3.5 — Static driver registry

The registry lives in the host-testable no_std `exhi-driver` crate. Hardware modules publish immutable `DriverDescriptor` constants; startup registers them in a bounded `DriverRegistry`.
- Stable IDs distinguish COM1, PIT channel 0 and the i8042 PS/2 keyboard. Classes are descriptive (Serial, Clock, Input); this is not yet a device-bus model.
- Registry entries move through `Registered`, `Initialized` and `Failed`. Duplicate IDs, full capacity, unknown IDs and illegal transitions are explicit errors. Failed is terminal for the current boot.
- The registry is fixed-capacity and allocation-free. It offers lookup, iteration and state counts.
- The registry records lifecycle but does not invoke init callbacks. The kernel keeps hardware initialization in the explicit boot path to preserve ordering constraints and make interrupt dependencies auditable.
- Normal boot reports three registered drivers: COM1 initialized, PIT and PS/2 registered. Timer and keyboard smoke modes mark their own driver initialized after successful setup.
- Eight host tests passed; CI shell syntax, all kernel build variants and headless standard/PIT/driver-registry QEMU runtime tests passed in [run 42299723267](https://github.com/mindwork64/exhiOS/actions/runs/42299723267). Local Linux Mint/QEMU evidence remains pending.
- Dedicated test: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_DRIVER_REGISTRY_TEST=1 ./scripts/run-qemu.sh`.


The hardware driver is `exhi-kernel/src/arch/x86_64/keyboard.rs`; pure queue/event decoding lives in the host-testable no_std `exhi-input` crate.
- The i8042 command byte enables IRQ1, disables IRQ12, enables the keyboard clock, disables the auxiliary clock and enables translated Set 1 output. The keyboard's F4 ACK is validated before IRQ1 is unmasked.
- PIC IRQ1 mask/unmask operations preserve the state of IRQ0 and every other line. IDT vector 0x21 is read-back validated.
- The IRQ1 stub is assembly-only: reads port 0x60, inserts one byte into the SPSC queue, counts overflow, sends master PIC EOI and returns. It deliberately makes no Rust call and does not allocate.
- Foreground code maps Set 1 make/break bytes to USB HID usage IDs, emits key-down/key-up events and tracks left/right modifier state plus Caps/Num/Scroll Lock. The queue has 63 usable entries and reports dropped scan codes.
- Six input-crate host tests passed; CI also built the kernel with the keyboard smoke-test feature in [run 42278708100](https://github.com/mindwork64/exhiOS/actions/runs/42278708100).
- Interactive QEMU smoke mode: `EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_KEYBOARD_TEST=1 ./scripts/run-qemu.sh`; Ctrl+D ends the test after event output. This is separate from the PIT timer test.
- Local interactive keyboard QEMU validation remains pending. This initial driver supports legacy PS/2 only; USB HID enumeration, keyboard layouts/Unicode translation, typematic policy and input focus routing remain future work.


## P2.6 — TSS, double-fault IST and boot-stack guard page

Code: [b46c870](https://github.com/mindwork64/exhiOS/commit/b46c8702e3725281e9a42654524649d1d04b4fe8). Headless CI/OVMF QEMU: [run 42256850017](https://github.com/mindwork64/exhiOS/actions/runs/42256850017).

- GDT has a 64-bit TSS descriptor at selector 0x18, loaded with LTR. IST1 points to a dedicated 16 KiB page-aligned kernel BSS stack.
- IDT vector 8 uses IST index 1. The common exception stub passes the normalized exception frame pointer to Rust; the #DF handler verifies it falls inside IST1.
- BootInfo v2 carries boot stack base/size. UEFI allocates a contiguous 17-page LOADER_DATA block: first page is reserved as the guard, next 16 pages form the 64 KiB kernel stack.
- Kernel reserves the guard physical frame in the allocator before subsequent allocations and removes its identity mapping through VMM after bootstrap paging and VMM initialization. It verifies the guard page is NotMapped and first/last stack pages remain supervisor mapped.
- QEMU debug-exit test sets RSP to the first byte of the mapped stack and accesses the guard page. #PF delivery attempts to push its exception frame into the guard page, causing delivery failure and a processor-generated #DF. The CPU switches to IST1; the handler checks its frame pointer and debug-exits with status 33 only on success.
- CI evidence: loader reports guard 0x1E6FF000 and stack 0x1E700000..0x1E710000; #DF frame pointer 0x16AFC8 lies within IST1; guard unmapping and stack mapping checks pass; debug-exit 33.

Local Linux Mint/QEMU verification by the user is pending. Do not consider P2.6 fully closed until local runtime evidence is reviewed.

