#![no_main]
#![no_std]

mod arch;
mod heap;
mod memory;

use arch::x86_64::{address_space, gdt, idt, paging, pic, stack_guard, vmm};
use core::arch::asm;
use core::hint::spin_loop;
use core::panic::PanicInfo;

use exhi_abi::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};

const COM1: u16 = 0x3F8;
const LINE_STATUS: u16 = COM1 + 5;
#[cfg(feature = "qemu-debug-exit-smoke-test")]
const QEMU_DEBUG_EXIT_PORT: u16 = 0xF4;
#[cfg(feature = "qemu-debug-exit-smoke-test")]
const QEMU_DEBUG_EXIT_VALUE: u32 = 0x10;
const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

#[no_mangle]
pub extern "sysv64" fn _start(boot_info: *const BootInfo) -> ! {
    serial_init();
    serial_write(b"ExhiOS Kernel 0.1\r\n");
    serial_write(b"Kernel entry point reached.\r\n");

    // Firmware enters the kernel with IF clear. Keep it that way until the
    // IDT contains handlers for every IRQ line that may be unmasked.
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }

    serial_write(b"GDT: installing kernel GDT and 64-bit TSS.\r\n");
    let tss_status = gdt::initialize();

    let selectors = gdt::read_segment_selectors();
    serial_write(b"GDT selectors: CS=0x");
    serial_write_hex(u64::from(selectors.cs));
    serial_write(b" DS=0x");
    serial_write_hex(u64::from(selectors.ds));
    serial_write(b" ES=0x");
    serial_write_hex(u64::from(selectors.es));
    serial_write(b" SS=0x");
    serial_write_hex(u64::from(selectors.ss));
    serial_write(b"\r\n");
    serial_write(b"GDT TSS selector: 0x");
    serial_write_hex(u64::from(tss_status.selector));
    serial_write(b"\r\nTask register (TR): 0x");
    serial_write_hex(u64::from(tss_status.task_register));
    serial_write(b"\r\nTSS base: 0x");
    serial_write_hex(tss_status.tss_base);
    serial_write(b"\r\nDouble-fault IST1 stack: 0x");
    serial_write_hex(tss_status.double_fault_ist_bottom);
    serial_write(b"..0x");
    serial_write_hex(tss_status.double_fault_ist_top);
    serial_write(b"\r\n");

    if !selectors.is_valid() || !tss_status.valid {
        serial_write(b"GDT/TSS selector verification failed.\r\n");
        halt_forever();
    }

    serial_write(b"GDT: installed and verified.\r\n");

    serial_write(b"IDT: installing kernel IDT.\r\n");
    let idt_status = idt::initialize();
    serial_write(b"IDT base: 0x");
    serial_write_hex(idt_status.base);
    serial_write(b"\r\nIDT limit: 0x");
    serial_write_hex(u64::from(idt_status.limit));
    serial_write(b"\r\nDouble-fault IST index: 0x");
    serial_write_hex(u64::from(idt_status.double_fault_ist));
    serial_write(b"\r\n");

    if !idt_status.valid {
        serial_write(b"IDT: SIDT verification failed.\r\n");
        halt_forever();
    }

    serial_write(b"IDT: installed and verified (CPU exceptions 0-31).\r\n");

    #[cfg(feature = "exception-smoke-test")]
    {
        serial_write(b"IDT smoke test: triggering INT3 breakpoint.\r\n");
        // SAFETY: The kernel IDT has a valid vector-3 gate. Its fatal handler
        // reports the exception and halts instead of returning to this point.
        unsafe {
            asm!("int3", options(nomem, nostack));
        }
    }

    serial_write(b"PIC: remapping legacy IRQs to vectors 0x20-0x2F.\r\n");
    let pic_status = pic::initialize_and_mask();
    serial_write(b"PIC master mask: 0x");
    serial_write_hex(u64::from(pic_status.master_mask));
    serial_write(b" slave mask: 0x");
    serial_write_hex(u64::from(pic_status.slave_mask));
    serial_write(b"\r\n");

    if !pic_status.all_masked() {
        serial_write(b"PIC: failed to mask all IRQ lines.\r\n");
        halt_forever();
    }

    serial_write(b"PIC: remap sequence issued; all IRQ masks verified.\r\n");
    serial_write(b"Interrupt flag remains clear until IRQ handlers are installed.\r\n");

    if !boot_info.is_null() {
        // SAFETY: The bootloader passes a pointer to a dedicated LOADER_DATA
        // page and retains that page for the lifetime of the kernel.
        let info = unsafe { &*boot_info };
        if info.magic == BOOT_INFO_MAGIC
            && info.version == BOOT_INFO_VERSION
            && info.is_valid()
        {
            serial_write(b"BootInfo: valid\r\n");

            serial_write(b"Memory map address: 0x");
            serial_write_hex(info.memory_map.physical_address);
            serial_write(b"\r\nMemory map size: 0x");
            serial_write_hex(info.memory_map.map_size);
            serial_write(b"\r\nMemory descriptor size: 0x");
            serial_write_hex(u64::from(info.memory_map.descriptor_size));
            serial_write(b"\r\nMemory descriptor version: 0x");
            serial_write_hex(u64::from(info.memory_map.descriptor_version));

            if info.memory_map.physical_address != 0
                && info.memory_map.map_size != 0
                && info.memory_map.descriptor_size != 0
            {
                let descriptor_count =
                    info.memory_map.map_size / u64::from(info.memory_map.descriptor_size);
                serial_write(b"\r\nMemory descriptors: 0x");
                serial_write_hex(descriptor_count);
            } else {
                serial_write(b"\r\nBootInfo: memory map metadata is incomplete");
            }

            serial_write(b"\r\nFramebuffer address: 0x");
            serial_write_hex(info.framebuffer.physical_address);
            serial_write(b"\r\nFramebuffer resolution: 0x");
            serial_write_hex(u64::from(info.framebuffer.width));
            serial_write(b" x 0x");
            serial_write_hex(u64::from(info.framebuffer.height));
            serial_write(b"\r\nACPI RSDP address: 0x");
            serial_write_hex(info.acpi_rsdp_address);
            serial_write(b"\r\n");

            serial_write(b"P2: initializing physical page allocator.\r\n");
            let memory_stats = match memory::initialize(info) {
                Ok(stats) => stats,
                Err(error) => {
                    serial_write(b"P2: physical page allocator initialization failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };

            let stack_guard_reservation = match stack_guard::reserve_before_allocations(info) {
                Ok(reservation) => reservation,
                Err(error) => {
                    serial_write(b"P2.6: kernel stack/guard bounds rejected: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            serial_write(b"Boot kernel stack base: 0x");
            serial_write_hex(stack_guard_reservation.stack_base);
            serial_write(b" size: 0x");
            serial_write_hex(stack_guard_reservation.stack_size);
            serial_write(b"\r\nReserved stack guard frame: 0x");
            serial_write_hex(stack_guard_reservation.guard_page);
            serial_write(b"\r\n");

            serial_write(b"Physical memory descriptors: 0x");
            serial_write_hex(memory_stats.descriptor_count);
            serial_write(b"\r\nManaged conventional pages: 0x");
            serial_write_hex(memory_stats.managed_pages);
            serial_write(b"\r\nFree physical pages: 0x");
            serial_write_hex(memory_stats.free_pages);
            serial_write(b"\r\nIgnored pages below 1 MiB: 0x");
            serial_write_hex(memory_stats.ignored_low_pages);
            serial_write(b"\r\nIgnored pages at/above 4 GiB: 0x");
            serial_write_hex(memory_stats.ignored_above_limit_pages);
            serial_write(b"\r\nHighest managed physical address: 0x");
            serial_write_hex(memory_stats.highest_managed_address);
            serial_write(b"\r\n");

            let reused_page = match memory::allocation_reuse_smoke_test() {
                Ok(address) => address,
                Err(error) => {
                    serial_write(b"P2: physical page allocation/reuse test failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            serial_write(b"Physical allocator smoke test: allocated, freed and reused page 0x");
            serial_write_hex(reused_page);
            serial_write(b"\r\nP2.1: physical page allocator ready.\r\n");

            serial_write(b"P2.2: building kernel-owned bootstrap page tables.\r\n");
            let paging_status = match paging::initialize_identity_map_4g(info) {
                Ok(status) => status,
                Err(error) => {
                    serial_write(b"P2.2: page-table initialization failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };

            serial_write(b"Page-table root (PML4): 0x");
            serial_write_hex(paging_status.root_address);
            serial_write(b"\r\nActive CR3: 0x");
            serial_write_hex(paging_status.active_cr3);
            serial_write(b"\r\nPage-table pages: 0x");
            serial_write_hex(paging_status.table_pages);
            serial_write(b"\r\nIdentity-mapped bytes: 0x");
            serial_write_hex(paging_status.mapped_bytes);
            serial_write(b"\r\nLarge-page size: 0x");
            serial_write_hex(paging_status.large_page_size);
            serial_write(b"\r\nVerified address ranges: 0x");
            serial_write_hex(paging_status.verified_ranges);
            serial_write(b"\r\nVerified stack address: 0x");
            serial_write_hex(paging_status.verified_stack_address);

            let kernel_address = _start as *const () as u64;
            let translated_kernel = match paging::translate_current(kernel_address) {
                Ok(address) => address,
                Err(error) => {
                    serial_write(b"\r\nP2.2: kernel address translation failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            serial_write(b"\r\nKernel entry translation: 0x");
            serial_write_hex(translated_kernel);

            if translated_kernel != kernel_address {
                serial_write(b"\r\nP2.2: kernel identity translation mismatch.\r\n");
                halt_forever();
            }

            serial_write(b"\r\nP2.2: kernel page tables active; CR3 and identity mappings verified.\r\n");

            serial_write(b"P2.3: initializing virtual memory manager.\r\n");
            let vmm_status = match vmm::initialize() {
                Ok(status) => status,
                Err(error) => {
                    serial_write(b"P2.3: VMM initialization failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            serial_write(b"4 KiB page size: 0x");
            serial_write_hex(vmm_status.page_size);
            serial_write(b"\r\nNX support: ");
            serial_write(if vmm_status.nx_enabled { b"enabled" } else { b"unavailable" });
            serial_write(b"\r\nVMM identity-map limit: 0x");
            serial_write_hex(vmm_status.identity_map_limit);
            serial_write(b"\r\nVMM virtual-address limit: 0x");
            serial_write_hex(vmm_status.virtual_address_limit);
            serial_write(b"\r\n");

            if let Err(error) = stack_guard::install(stack_guard_reservation) {
                serial_write(b"P2.6: failed to install kernel stack guard page: ");
                serial_write(error.message());
                serial_write(b"\r\n");
                halt_forever();
            }
            serial_write(b"P2.6: unmapped guard page below boot kernel stack: 0x");
            serial_write_hex(stack_guard_reservation.guard_page);
            serial_write(b"\r\nP2.6: boot kernel stack remains mapped and supervisor-only.\r\n");

            // Exercise a page inside an otherwise unused 1 GiB virtual range.
            // The test temporarily splits one 2 MiB huge page, maps a managed
            // physical frame with RW+NX supervisor permissions, and restores
            // the original identity mapping after validating the page.
            const VMM_SMOKE_VIRTUAL_PAGE: u64 = 0x4000_0000;
            let data_frame = match memory::allocate_frame() {
                Ok(frame) => frame,
                Err(error) => {
                    serial_write(b"P2.3: test data-frame allocation failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };

            let original_frame = match vmm::unmap_page(VMM_SMOKE_VIRTUAL_PAGE) {
                Ok(frame) => frame,
                Err(error) => {
                    let _ = memory::release_frame(data_frame);
                    serial_write(b"P2.3: initial 4 KiB unmap failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            if original_frame != VMM_SMOKE_VIRTUAL_PAGE {
                serial_write(b"P2.3: bootstrap identity mapping did not match expected frame.\r\n");
                halt_forever();
            }

            if let Err(error) = vmm::map_page(
                VMM_SMOKE_VIRTUAL_PAGE,
                data_frame,
                vmm::PageFlags::kernel_data(),
            ) {
                let _ = memory::release_frame(data_frame);
                serial_write(b"P2.3: 4 KiB map failed: ");
                serial_write(error.message());
                serial_write(b"\r\n");
                halt_forever();
            }

            let mapping = match vmm::query_page(VMM_SMOKE_VIRTUAL_PAGE) {
                Ok(mapping) => mapping,
                Err(error) => {
                    serial_write(b"P2.3: mapping query failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            if mapping.physical_page != data_frame
                || !mapping.flags.writable
                || mapping.flags.executable
                || mapping.flags.user
            {
                serial_write(b"P2.3: page permissions or physical translation mismatch.\r\n");
                halt_forever();
            }

            let test_bytes = [0x45u8, 0x58, 0x48, 0x49, 0x4F, 0x53, 0x21, 0xA5];
            // SAFETY: The test virtual page is mapped to an allocated,
            // writable physical frame above and remains mapped for this access.
            unsafe {
                for (index, byte) in test_bytes.iter().copied().enumerate() {
                    core::ptr::write_volatile((VMM_SMOKE_VIRTUAL_PAGE as *mut u8).add(index), byte);
                }
                for (index, expected) in test_bytes.iter().copied().enumerate() {
                    let actual =
                        core::ptr::read_volatile((VMM_SMOKE_VIRTUAL_PAGE as *const u8).add(index));
                    if actual != expected {
                        serial_write(b"P2.3: mapped-page read/write verification failed.\r\n");
                        halt_forever();
                    }
                }
            }

            let removed_frame = match vmm::unmap_page(VMM_SMOKE_VIRTUAL_PAGE) {
                Ok(frame) => frame,
                Err(error) => {
                    serial_write(b"P2.3: test-page unmap failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            if removed_frame != data_frame
                || paging::translate_current(VMM_SMOKE_VIRTUAL_PAGE) != Err(paging::PagingError::NotMapped)
            {
                serial_write(b"P2.3: TLB invalidation or unmap verification failed.\r\n");
                halt_forever();
            }
            if let Err(error) = memory::release_frame(data_frame) {
                serial_write(b"P2.3: released-frame bookkeeping failed: ");
                serial_write(error.message());
                serial_write(b"\r\n");
                halt_forever();
            }

            if let Err(error) = vmm::map_page(
                VMM_SMOKE_VIRTUAL_PAGE,
                original_frame,
                vmm::PageFlags::kernel_read_write_execute(),
            ) {
                serial_write(b"P2.3: identity-map restoration failed: ");
                serial_write(error.message());
                serial_write(b"\r\n");
                halt_forever();
            }
            if paging::translate_current(VMM_SMOKE_VIRTUAL_PAGE) != Ok(original_frame) {
                serial_write(b"P2.3: restored identity mapping verification failed.\r\n");
                halt_forever();
            }

            serial_write(b"P2.3: 2 MiB page split into 4 KiB entries.\r\n");
            serial_write(b"P2.3: RW supervisor NX map, read/write, unmap and TLB invalidation verified.\r\n");
            match vmm::try_coalesce_large_page(VMM_SMOKE_VIRTUAL_PAGE) {
                Ok(true) => {}
                Ok(false) => {
                    serial_write(b"P2.3: equivalent 4 KiB entries could not be coalesced.\r\n");
                    halt_forever();
                }
                Err(error) => {
                    serial_write(b"P2.3: safe page-table reclamation failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            }
            if paging::translate_current(VMM_SMOKE_VIRTUAL_PAGE) != Ok(original_frame) {
                serial_write(b"P2.3: huge-page coalescing changed the physical mapping.\r\n");
                halt_forever();
            }
            serial_write(b"P2.3: original identity mapping restored; page-table frame safely reclaimed.\r\n");
            serial_write(b"P2.3: virtual memory manager 4 KiB mapping smoke test passed.\r\n");

            serial_write(b"P2.4: initializing page-backed kernel heap.\r\n");
            let mut heap_status = match heap::initialize() {
                Ok(status) => status,
                Err(error) => {
                    serial_write(b"P2.4: kernel heap initialization failed: ");
                    serial_write(heap_init_error_message(error));
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            serial_write(b"Kernel heap virtual base: 0x");
            serial_write_hex(heap_status.virtual_base);
            serial_write(b"\r\nKernel heap mapped pages: 0x");
            serial_write_hex(heap_status.mapped_pages as u64);
            serial_write(b"\r\nKernel heap mapped bytes: 0x");
            serial_write_hex(heap_status.mapped_bytes);
            serial_write(b"\r\n");

            heap_status = match heap::allocation_smoke_test() {
                Ok(status) => status,
                Err(error) => {
                    serial_write(b"P2.4: kernel heap allocation test failed: ");
                    serial_write(heap_test_error_message(error));
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            serial_write(b"P2.4: aligned allocation, 48 KiB growth, cross-page read/write, free and reuse verified.\r\n");
            serial_write(b"Kernel heap pages after growth: 0x");
            serial_write_hex(heap_status.mapped_pages as u64);
            serial_write(b"\r\nKernel heap free bytes: 0x");
            serial_write_hex(heap_status.free_bytes as u64);
            serial_write(b"\r\nP2.4: kernel heap allocator ready.\r\n");

            serial_write(b"P2.5: initializing kernel address-space manager.\r\n");
            let kernel_root = match address_space::initialize() {
                Ok(root) => root,
                Err(error) => {
                    serial_write(b"P2.5: address-space initialization failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            serial_write(b"Kernel address-space root: 0x");
            serial_write_hex(kernel_root);
            serial_write(b"\r\n");

            let address_space_status = match address_space::smoke_test() {
                Ok(status) => status,
                Err(error) => {
                    let _ = address_space::restore_kernel();
                    serial_write(b"P2.5: address-space smoke test failed: ");
                    serial_write(error.message());
                    serial_write(b"\r\n");
                    halt_forever();
                }
            };
            serial_write(b"Address-space A PML4: 0x");
            serial_write_hex(address_space_status.first_root);
            serial_write(b"\r\nAddress-space B PML4: 0x");
            serial_write_hex(address_space_status.second_root);
            serial_write(b"\r\nShared kernel translation: 0x");
            serial_write_hex(address_space_status.shared_kernel_translation);
            serial_write(b"\r\nShared heap translation: 0x");
            serial_write_hex(address_space_status.shared_heap_translation);
            serial_write(b"\r\nUser virtual test page: 0x");
            serial_write_hex(address_space_status.user_virtual_page);
            serial_write(b"\r\nAddress-space A physical page: 0x");
            serial_write_hex(address_space_status.first_physical_page);
            serial_write(b"\r\nAddress-space B physical page: 0x");
            serial_write_hex(address_space_status.second_physical_page);
            serial_write(b"\r\nP2.5: separate roots, shared kernel maps, private user mappings and CR3 switching verified.\r\n");
            serial_write(b"P2.5: private page tables reclaimed; data frames safely returned.\r\n");
            serial_write(b"P2.5: kernel address-space smoke test passed.\r\n");

            #[cfg(feature = "qemu-debug-exit-smoke-test")]
            {
                serial_write(b"P2.6: forcing an exception-frame write into the unmapped stack guard page.\r\n");
                idt::trigger_double_fault_guard_stack_smoke_test(
                    stack_guard_reservation.stack_base,
                    stack_guard_reservation.guard_page,
                );
            }

        } else {
            serial_write(b"BootInfo: invalid; cannot initialize physical memory.\r\n");
            halt_forever();
        }
    } else {
        serial_write(b"BootInfo: not supplied; cannot initialize physical memory.\r\n");
        halt_forever();
    }

    serial_write(b"ExhiOS: kernel handoff successful.\r\n");

    #[cfg(feature = "qemu-debug-exit-smoke-test")]
    {
        serial_write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
        // SAFETY: The QEMU test script attaches isa-debug-exit at port 0xF4.
        // Writing value 0x10 makes QEMU terminate with status (0x10 << 1) | 1.
        unsafe {
            asm!(
                "out dx, eax",
                in("dx") QEMU_DEBUG_EXIT_PORT,
                in("eax") QEMU_DEBUG_EXIT_VALUE,
                options(nomem, nostack, preserves_flags)
            );
        }
        serial_write(b"QEMU debug-exit: device did not terminate the guest.\r\n");
    }

    serial_write(b"ExhiOS: interrupts remain disabled until IRQ handlers are installed.\r\n");
    serial_write(b"ExhiOS: halt loop entered.\r\n");

    halt_forever()
}

/// Fatal CPU-exception entry called by the assembly stubs.
/// Arguments follow the x86-64 System V ABI.
#[no_mangle]
pub extern "sysv64" fn exhi_exception_handler(
    vector: u64,
    error_code: u64,
    instruction_pointer: u64,
    code_selector: u64,
    flags: u64,
    exception_stack_pointer: u64,
) -> ! {
    serial_write(b"\r\nExhiOS CPU EXCEPTION\r\n");
    serial_write(b"Vector: 0x");
    serial_write_hex(vector);
    serial_write(b" (");
    serial_write(exception_name(vector));
    serial_write(b")\r\nError code: 0x");
    serial_write_hex(error_code);
    serial_write(b"\r\nRIP: 0x");
    serial_write_hex(instruction_pointer);
    serial_write(b"\r\nCS: 0x");
    serial_write_hex(code_selector);
    serial_write(b"\r\nRFLAGS: 0x");
    serial_write_hex(flags);
    serial_write(b"\r\n");

    if vector == 8 {
        serial_write(b"Exception stack pointer: 0x");
        serial_write_hex(exception_stack_pointer);
        serial_write(b"\r\n");
        if gdt::double_fault_ist_contains(exception_stack_pointer) {
            serial_write(b"P2.6: double fault arrived on the dedicated IST1 stack.\r\n");
            #[cfg(feature = "qemu-debug-exit-smoke-test")]
            {
                serial_write(b"P2.6: double-fault IST runtime smoke test passed.\r\n");
                // SAFETY: The QEMU debug-exit device is attached by the test
                // script. Reaching here proves #DF was delivered on IST1.
                unsafe {
                    asm!(
                        "out dx, eax",
                        in("dx") QEMU_DEBUG_EXIT_PORT,
                        in("eax") QEMU_DEBUG_EXIT_VALUE,
                        options(nomem, nostack, preserves_flags)
                    );
                }
                serial_write(b"QEMU debug-exit: device did not terminate after IST test.\r\n");
            }
        } else {
            serial_write(b"P2.6: double fault did not use the dedicated IST1 stack.\r\n");
        }
    }

    if vector == 14 {
        let fault_address: u64;
        // SAFETY: Reading CR2 is valid in ring 0 and does not modify CPU state.
        unsafe {
            asm!("mov {}, cr2", out(reg) fault_address, options(nomem, nostack, preserves_flags));
        }
        serial_write(b"CR2 (page-fault address): 0x");
        serial_write_hex(fault_address);
        serial_write(b"\r\n");
    }

    serial_write(b"ExhiOS: CPU halted after fatal exception.\r\n");
    halt_forever()
}

fn exception_name(vector: u64) -> &'static [u8] {
    match vector {
        0 => b"Divide Error",
        1 => b"Debug",
        2 => b"Non-maskable Interrupt",
        3 => b"Breakpoint",
        4 => b"Overflow",
        5 => b"Bound Range",
        6 => b"Invalid Opcode",
        7 => b"Device Not Available",
        8 => b"Double Fault",
        9 => b"Coprocessor Segment Overrun",
        10 => b"Invalid TSS",
        11 => b"Segment Not Present",
        12 => b"Stack-Segment Fault",
        13 => b"General Protection Fault",
        14 => b"Page Fault",
        15 => b"Reserved",
        16 => b"x87 Floating-Point Exception",
        17 => b"Alignment Check",
        18 => b"Machine Check",
        19 => b"SIMD Floating-Point Exception",
        20 => b"Virtualization Exception",
        21 => b"Control Protection Exception",
        22..=27 => b"Reserved",
        28 => b"Hypervisor Injection Exception",
        29 => b"VMM Communication Exception",
        30 => b"Security Exception",
        31 => b"Reserved",
        _ => b"Unknown CPU exception",
    }
}


fn heap_init_error_message(error: heap::HeapInitError) -> &'static [u8] {
    match error {
        heap::HeapInitError::AlreadyInitialized => b"heap was already initialized",
        heap::HeapInitError::OutOfVirtualSpace => b"heap virtual range exhausted",
        heap::HeapInitError::OutOfPhysicalFrames => b"physical frame allocator exhausted",
        heap::HeapInitError::InvalidPhysicalFrame => b"physical allocator returned an invalid frame",
        heap::HeapInitError::Vmm(error) => error.message(),
        heap::HeapInitError::Heap(_) => b"heap region registration failed",
    }
}

fn heap_test_error_message(error: heap::HeapTestError) -> &'static [u8] {
    match error {
        heap::HeapTestError::NotInitialized => b"heap is not initialized",
        heap::HeapTestError::AllocationFailed => b"allocation returned null",
        heap::HeapTestError::DataMismatch => b"alignment or memory contents mismatch",
        heap::HeapTestError::ReuseMismatch => b"freed allocation was not reused",
        heap::HeapTestError::FreeListNotCoalesced => b"free blocks did not coalesce",
    }
}

fn halt_forever() -> ! {
    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

fn serial_init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1, 0x03);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        outb(COM1 + 4, 0x0B);
    }
}

fn serial_write(bytes: &[u8]) {
    for &byte in bytes {
        let mut ready = false;
        for _ in 0..100_000 {
            if unsafe { inb(LINE_STATUS) } & 0x20 != 0 {
                ready = true;
                break;
            }
            spin_loop();
        }

        if !ready {
            return;
        }

        unsafe {
            outb(COM1, byte);
        }
    }
}

fn serial_write_hex(value: u64) {
    for shift in (0..16).rev() {
        let digit = ((value >> (shift * 4)) & 0xF) as usize;
        serial_write(&HEX_DIGITS[digit..digit + 1]);
    }
}

unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    serial_write(b"ExhiOS Kernel panic.\r\n");
    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
