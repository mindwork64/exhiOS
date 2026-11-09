#![no_main]
#![no_std]

mod arch;
mod console;
mod heap;
mod memory;

use arch::x86_64::{
    address_space, framebuffer, gdt, idt, keyboard, paging, pic, serial, stack_guard, timer, vmm,
};
use console::KernelConsole;
use core::arch::asm;
use core::panic::PanicInfo;
use exhi_console::Color;
#[cfg(feature = "driver-registry-smoke-test")]
use exhi_driver::{DriverClass, DriverId};
use exhi_driver::{DriverRegistry, DriverState, RegistryError};

use exhi_abi::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};

#[cfg(feature = "qemu-debug-exit-smoke-test")]
const QEMU_DEBUG_EXIT_PORT: u16 = 0xF4;
#[cfg(feature = "qemu-debug-exit-smoke-test")]
const QEMU_DEBUG_EXIT_VALUE: u32 = 0x10;

#[cfg(feature = "process-load-smoke-test")]
const EMBEDDED_INIT_ELF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../target/x86_64-unknown-none/debug/exhi-init"
));

#[no_mangle]
pub extern "sysv64" fn _start(boot_info: *const BootInfo) -> ! {
    let mut framebuffer_console: Option<exhi_console::FramebufferConsole<'static>> = None;
    serial::COM1.initialize();
    serial::COM1.write(b"ExhiOS Kernel 0.1\r\n");
    serial::COM1.write(b"P3.1: COM1 polling serial driver ready.\r\n");
    serial::COM1.write(b"Kernel entry point reached.\r\n");

    // Firmware enters the kernel with IF clear. Keep it that way until the
    // IDT contains handlers for every IRQ line that may be unmasked.
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }

    serial::COM1.write(b"GDT: installing kernel GDT and 64-bit TSS.\r\n");
    let tss_status = gdt::initialize();

    let selectors = gdt::read_segment_selectors();
    serial::COM1.write(b"GDT selectors: CS=0x");
    serial::COM1.write_hex(u64::from(selectors.cs));
    serial::COM1.write(b" DS=0x");
    serial::COM1.write_hex(u64::from(selectors.ds));
    serial::COM1.write(b" ES=0x");
    serial::COM1.write_hex(u64::from(selectors.es));
    serial::COM1.write(b" SS=0x");
    serial::COM1.write_hex(u64::from(selectors.ss));
    serial::COM1.write(b"\r\n");
    serial::COM1.write(b"GDT TSS selector: 0x");
    serial::COM1.write_hex(u64::from(tss_status.selector));
    serial::COM1.write(b"\r\nTask register (TR): 0x");
    serial::COM1.write_hex(u64::from(tss_status.task_register));
    serial::COM1.write(b"\r\nTSS base: 0x");
    serial::COM1.write_hex(tss_status.tss_base);
    serial::COM1.write(b"\r\nDouble-fault IST1 stack: 0x");
    serial::COM1.write_hex(tss_status.double_fault_ist_bottom);
    serial::COM1.write(b"..0x");
    serial::COM1.write_hex(tss_status.double_fault_ist_top);
    serial::COM1.write(b"\r\n");

    if !selectors.is_valid() || !tss_status.valid {
        serial::COM1.write(b"GDT/TSS selector verification failed.\r\n");
        halt_forever();
    }

    serial::COM1.write(b"GDT: installed and verified.\r\n");

    serial::COM1.write(b"IDT: installing kernel IDT.\r\n");
    let idt_status = idt::initialize();
    serial::COM1.write(b"IDT base: 0x");
    serial::COM1.write_hex(idt_status.base);
    serial::COM1.write(b"\r\nIDT limit: 0x");
    serial::COM1.write_hex(u64::from(idt_status.limit));
    serial::COM1.write(b"\r\nDouble-fault IST index: 0x");
    serial::COM1.write_hex(u64::from(idt_status.double_fault_ist));
    serial::COM1.write(b"\r\n");

    if !idt_status.valid {
        serial::COM1.write(b"IDT: SIDT verification failed.\r\n");
        halt_forever();
    }

    serial::COM1.write(b"IDT: installed and verified (CPU exceptions 0-31).\r\n");
    serial::COM1.write(b"IDT IRQ0 timer gate: ");
    serial::COM1.write(if idt_status.timer_irq_valid {
        b"valid"
    } else {
        b"bad  "
    });
    serial::COM1.write(b"\r\nIDT IRQ1 keyboard gate: ");
    serial::COM1.write(if idt_status.keyboard_irq_valid {
        b"valid"
    } else {
        b"bad  "
    });
    serial::COM1.write(b"\r\n");

    #[cfg(feature = "exception-smoke-test")]
    {
        serial::COM1.write(b"IDT smoke test: triggering INT3 breakpoint.\r\n");
        // SAFETY: The kernel IDT has a valid vector-3 gate. Its fatal handler
        // reports the exception and halts instead of returning to this point.
        unsafe {
            asm!("int3", options(nomem, nostack));
        }
    }

    serial::COM1.write(b"PIC: remapping legacy IRQs to vectors 0x20-0x2F.\r\n");
    let pic_status = pic::initialize_and_mask();
    serial::COM1.write(b"PIC master mask: 0x");
    serial::COM1.write_hex(u64::from(pic_status.master_mask));
    serial::COM1.write(b" slave mask: 0x");
    serial::COM1.write_hex(u64::from(pic_status.slave_mask));
    serial::COM1.write(b"\r\n");

    if !pic_status.all_masked() {
        serial::COM1.write(b"PIC: failed to mask all IRQ lines.\r\n");
        halt_forever();
    }

    serial::COM1.write(b"PIC: remap sequence issued; all IRQ masks verified.\r\n");
    serial::COM1.write(b"Interrupt flag remains clear until IRQ handlers are installed.\r\n");

    // Mutated only by the opt-in timer, keyboard and registry smoke tests.
    #[allow(unused_mut)]
    let mut driver_registry = match register_builtin_drivers() {
        Ok(registry) => registry,
        Err(_) => {
            serial::COM1.write(b"P3.5: built-in driver registration failed.\r\n");
            halt_forever();
        }
    };

    if !boot_info.is_null() {
        // SAFETY: The bootloader passes a pointer to a dedicated LOADER_DATA
        // page and retains that page for the lifetime of the kernel.
        let info = unsafe { &*boot_info };
        if info.magic == BOOT_INFO_MAGIC && info.version == BOOT_INFO_VERSION && info.is_valid() {
            serial::COM1.write(b"BootInfo: valid\r\n");

            serial::COM1.write(b"Memory map address: 0x");
            serial::COM1.write_hex(info.memory_map.physical_address);
            serial::COM1.write(b"\r\nMemory map size: 0x");
            serial::COM1.write_hex(info.memory_map.map_size);
            serial::COM1.write(b"\r\nMemory descriptor size: 0x");
            serial::COM1.write_hex(u64::from(info.memory_map.descriptor_size));
            serial::COM1.write(b"\r\nMemory descriptor version: 0x");
            serial::COM1.write_hex(u64::from(info.memory_map.descriptor_version));

            if info.memory_map.physical_address != 0
                && info.memory_map.map_size != 0
                && info.memory_map.descriptor_size != 0
            {
                let descriptor_count =
                    info.memory_map.map_size / u64::from(info.memory_map.descriptor_size);
                serial::COM1.write(b"\r\nMemory descriptors: 0x");
                serial::COM1.write_hex(descriptor_count);
            } else {
                serial::COM1.write(b"\r\nBootInfo: memory map metadata is incomplete");
            }

            serial::COM1.write(b"\r\nFramebuffer address: 0x");
            serial::COM1.write_hex(info.framebuffer.physical_address);
            serial::COM1.write(b"\r\nFramebuffer resolution: 0x");
            serial::COM1.write_hex(u64::from(info.framebuffer.width));
            serial::COM1.write(b" x 0x");
            serial::COM1.write_hex(u64::from(info.framebuffer.height));
            serial::COM1.write(b"\r\nACPI RSDP address: 0x");
            serial::COM1.write_hex(info.acpi_rsdp_address);
            serial::COM1.write(b"\r\n");

            serial::COM1.write(b"P2: initializing physical page allocator.\r\n");
            let memory_stats = match memory::initialize(info) {
                Ok(stats) => stats,
                Err(error) => {
                    serial::COM1.write(b"P2: physical page allocator initialization failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };

            let stack_guard_reservation = match stack_guard::reserve_before_allocations(info) {
                Ok(reservation) => reservation,
                Err(error) => {
                    serial::COM1.write(b"P2.6: kernel stack/guard bounds rejected: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            serial::COM1.write(b"Boot kernel stack base: 0x");
            serial::COM1.write_hex(stack_guard_reservation.stack_base);
            serial::COM1.write(b" size: 0x");
            serial::COM1.write_hex(stack_guard_reservation.stack_size);
            serial::COM1.write(b"\r\nReserved stack guard frame: 0x");
            serial::COM1.write_hex(stack_guard_reservation.guard_page);
            serial::COM1.write(b"\r\n");

            serial::COM1.write(b"Physical memory descriptors: 0x");
            serial::COM1.write_hex(memory_stats.descriptor_count);
            serial::COM1.write(b"\r\nManaged conventional pages: 0x");
            serial::COM1.write_hex(memory_stats.managed_pages);
            serial::COM1.write(b"\r\nFree physical pages: 0x");
            serial::COM1.write_hex(memory_stats.free_pages);
            serial::COM1.write(b"\r\nIgnored pages below 1 MiB: 0x");
            serial::COM1.write_hex(memory_stats.ignored_low_pages);
            serial::COM1.write(b"\r\nIgnored pages at/above 4 GiB: 0x");
            serial::COM1.write_hex(memory_stats.ignored_above_limit_pages);
            serial::COM1.write(b"\r\nHighest managed physical address: 0x");
            serial::COM1.write_hex(memory_stats.highest_managed_address);
            serial::COM1.write(b"\r\n");

            let reused_page = match memory::allocation_reuse_smoke_test() {
                Ok(address) => address,
                Err(error) => {
                    serial::COM1.write(b"P2: physical page allocation/reuse test failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            serial::COM1
                .write(b"Physical allocator smoke test: allocated, freed and reused page 0x");
            serial::COM1.write_hex(reused_page);
            serial::COM1.write(b"\r\nP2.1: physical page allocator ready.\r\n");

            #[cfg(feature = "paging-rollback-smoke-test")]
            {
                serial::COM1.write(b"P2.2: exercising post-CR3-switch rollback.\r\n");
                if !paging::rollback_smoke_test(info) {
                    serial::COM1.write(b"P2.2: controlled CR3 rollback verification failed.\r\n");
                    halt_forever();
                }
                serial::COM1.write(b"P2.2: post-switch failure restored the original CR3.\r\n");
            }

            serial::COM1.write(b"P2.2: building kernel-owned bootstrap page tables.\r\n");
            let paging_status = match paging::initialize_identity_map_4g(info) {
                Ok(status) => status,
                Err(error) => {
                    serial::COM1.write(b"P2.2: page-table initialization failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };

            serial::COM1.write(b"Page-table root (PML4): 0x");
            serial::COM1.write_hex(paging_status.root_address);
            serial::COM1.write(b"\r\nActive CR3: 0x");
            serial::COM1.write_hex(paging_status.active_cr3);
            serial::COM1.write(b"\r\nPage-table pages: 0x");
            serial::COM1.write_hex(paging_status.table_pages);
            serial::COM1.write(b"\r\nIdentity-mapped bytes: 0x");
            serial::COM1.write_hex(paging_status.mapped_bytes);
            serial::COM1.write(b"\r\nLarge-page size: 0x");
            serial::COM1.write_hex(paging_status.large_page_size);
            serial::COM1.write(b"\r\nVerified address ranges: 0x");
            serial::COM1.write_hex(paging_status.verified_ranges);
            serial::COM1.write(b"\r\nVerified stack address: 0x");
            serial::COM1.write_hex(paging_status.verified_stack_address);

            let kernel_address = _start as *const () as u64;
            let translated_kernel = match paging::translate_current(kernel_address) {
                Ok(address) => address,
                Err(error) => {
                    serial::COM1.write(b"\r\nP2.2: kernel address translation failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            serial::COM1.write(b"\r\nKernel entry translation: 0x");
            serial::COM1.write_hex(translated_kernel);

            if translated_kernel != kernel_address {
                serial::COM1.write(b"\r\nP2.2: kernel identity translation mismatch.\r\n");
                halt_forever();
            }

            serial::COM1.write(
                b"\r\nP2.2: kernel page tables active; CR3 and identity mappings verified.\r\n",
            );

            serial::COM1.write(b"P2.3: initializing virtual memory manager.\r\n");
            let vmm_status = match vmm::initialize() {
                Ok(status) => status,
                Err(error) => {
                    serial::COM1.write(b"P2.3: VMM initialization failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            serial::COM1.write(b"4 KiB page size: 0x");
            serial::COM1.write_hex(vmm_status.page_size);
            serial::COM1.write(b"\r\nNX support: ");
            serial::COM1.write(if vmm_status.nx_enabled {
                b"enabled"
            } else {
                b"unavailable"
            });
            serial::COM1.write(b"\r\nVMM identity-map limit: 0x");
            serial::COM1.write_hex(vmm_status.identity_map_limit);
            serial::COM1.write(b"\r\nVMM virtual-address limit: 0x");
            serial::COM1.write_hex(vmm_status.virtual_address_limit);
            serial::COM1.write(b"\r\n");

            let invalid_unaligned_virtual = matches!(
                vmm::map_page(0x4000_0001, 0x1000, vmm::PageFlags::kernel_data(),),
                Err(vmm::VmmError::UnalignedAddress)
            );
            let invalid_virtual_limit = matches!(
                vmm::map_page(0x0000_0080_0000_0000, 0x1000, vmm::PageFlags::kernel_data(),),
                Err(vmm::VmmError::AddressOutOfRange)
            );
            let invalid_physical_limit = matches!(
                vmm::map_page(0x4000_0000, 0x1_0000_0000, vmm::PageFlags::kernel_data(),),
                Err(vmm::VmmError::AddressOutOfRange)
            );
            if !(invalid_unaligned_virtual && invalid_virtual_limit && invalid_physical_limit) {
                serial::COM1.write(b"P2.2: invalid page-range rejection checks failed.\r\n");
                halt_forever();
            }
            serial::COM1.write(
                b"P2.2: unaligned, out-of-window virtual and out-of-range physical mappings rejected.\r\n",
            );

            if let Err(error) = stack_guard::install(stack_guard_reservation) {
                serial::COM1.write(b"P2.6: failed to install kernel stack guard page: ");
                serial::COM1.write(error.message());
                serial::COM1.write(b"\r\n");
                halt_forever();
            }
            serial::COM1.write(b"P2.6: unmapped guard page below boot kernel stack: 0x");
            serial::COM1.write_hex(stack_guard_reservation.guard_page);
            serial::COM1
                .write(b"\r\nP2.6: boot kernel stack remains mapped and supervisor-only.\r\n");

            // Exercise a page inside an otherwise unused 1 GiB virtual range.
            // The test temporarily splits one 2 MiB huge page, maps a managed
            // physical frame with RW+NX supervisor permissions, and restores
            // the original identity mapping after validating the page.
            const VMM_SMOKE_VIRTUAL_PAGE: u64 = 0x4000_0000;
            let data_frame = match memory::allocate_frame() {
                Ok(frame) => frame,
                Err(error) => {
                    serial::COM1.write(b"P2.3: test data-frame allocation failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };

            let original_frame = match vmm::unmap_page(VMM_SMOKE_VIRTUAL_PAGE) {
                Ok(frame) => frame,
                Err(error) => {
                    let _ = memory::release_frame(data_frame);
                    serial::COM1.write(b"P2.3: initial 4 KiB unmap failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            if original_frame != VMM_SMOKE_VIRTUAL_PAGE {
                serial::COM1
                    .write(b"P2.3: bootstrap identity mapping did not match expected frame.\r\n");
                halt_forever();
            }

            if let Err(error) = vmm::map_page(
                VMM_SMOKE_VIRTUAL_PAGE,
                data_frame,
                vmm::PageFlags::kernel_data(),
            ) {
                let _ = memory::release_frame(data_frame);
                serial::COM1.write(b"P2.3: 4 KiB map failed: ");
                serial::COM1.write(error.message());
                serial::COM1.write(b"\r\n");
                halt_forever();
            }

            let mapping = match vmm::query_page(VMM_SMOKE_VIRTUAL_PAGE) {
                Ok(mapping) => mapping,
                Err(error) => {
                    serial::COM1.write(b"P2.3: mapping query failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            if mapping.physical_page != data_frame
                || !mapping.flags.writable
                || mapping.flags.executable
                || mapping.flags.user
            {
                serial::COM1.write(b"P2.3: page permissions or physical translation mismatch.\r\n");
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
                        serial::COM1
                            .write(b"P2.3: mapped-page read/write verification failed.\r\n");
                        halt_forever();
                    }
                }
            }

            let removed_frame = match vmm::unmap_page(VMM_SMOKE_VIRTUAL_PAGE) {
                Ok(frame) => frame,
                Err(error) => {
                    serial::COM1.write(b"P2.3: test-page unmap failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            if removed_frame != data_frame
                || paging::translate_current(VMM_SMOKE_VIRTUAL_PAGE)
                    != Err(paging::PagingError::NotMapped)
            {
                serial::COM1.write(b"P2.3: TLB invalidation or unmap verification failed.\r\n");
                halt_forever();
            }
            if let Err(error) = memory::release_frame(data_frame) {
                serial::COM1.write(b"P2.3: released-frame bookkeeping failed: ");
                serial::COM1.write(error.message());
                serial::COM1.write(b"\r\n");
                halt_forever();
            }

            if let Err(error) = vmm::map_page(
                VMM_SMOKE_VIRTUAL_PAGE,
                original_frame,
                vmm::PageFlags::kernel_read_write_execute(),
            ) {
                serial::COM1.write(b"P2.3: identity-map restoration failed: ");
                serial::COM1.write(error.message());
                serial::COM1.write(b"\r\n");
                halt_forever();
            }
            if paging::translate_current(VMM_SMOKE_VIRTUAL_PAGE) != Ok(original_frame) {
                serial::COM1.write(b"P2.3: restored identity mapping verification failed.\r\n");
                halt_forever();
            }

            serial::COM1.write(b"P2.3: 2 MiB page split into 4 KiB entries.\r\n");
            serial::COM1.write(
                b"P2.3: RW supervisor NX map, read/write, unmap and TLB invalidation verified.\r\n",
            );
            match vmm::try_coalesce_large_page(VMM_SMOKE_VIRTUAL_PAGE) {
                Ok(true) => {}
                Ok(false) => {
                    serial::COM1
                        .write(b"P2.3: equivalent 4 KiB entries could not be coalesced.\r\n");
                    halt_forever();
                }
                Err(error) => {
                    serial::COM1.write(b"P2.3: safe page-table reclamation failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            }
            if paging::translate_current(VMM_SMOKE_VIRTUAL_PAGE) != Ok(original_frame) {
                serial::COM1.write(b"P2.3: huge-page coalescing changed the physical mapping.\r\n");
                halt_forever();
            }
            serial::COM1.write(
                b"P2.3: original identity mapping restored; page-table frame safely reclaimed.\r\n",
            );
            serial::COM1
                .write(b"P2.3: virtual memory manager 4 KiB mapping smoke test passed.\r\n");

            serial::COM1.write(b"P2.4: initializing page-backed kernel heap.\r\n");
            let mut heap_status = match heap::initialize() {
                Ok(status) => status,
                Err(error) => {
                    serial::COM1.write(b"P2.4: kernel heap initialization failed: ");
                    serial::COM1.write(heap_init_error_message(error));
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            serial::COM1.write(b"Kernel heap virtual base: 0x");
            serial::COM1.write_hex(heap_status.virtual_base);
            serial::COM1.write(b"\r\nKernel heap mapped pages: 0x");
            serial::COM1.write_hex(heap_status.mapped_pages as u64);
            serial::COM1.write(b"\r\nKernel heap mapped bytes: 0x");
            serial::COM1.write_hex(heap_status.mapped_bytes);
            serial::COM1.write(b"\r\n");

            heap_status = match heap::allocation_smoke_test() {
                Ok(status) => status,
                Err(error) => {
                    serial::COM1.write(b"P2.4: kernel heap allocation test failed: ");
                    serial::COM1.write(heap_test_error_message(error));
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            serial::COM1.write(b"P2.4: aligned allocation, 48 KiB growth, cross-page read/write, free and reuse verified.\r\n");
            serial::COM1.write(b"Kernel heap pages after growth: 0x");
            serial::COM1.write_hex(heap_status.mapped_pages as u64);
            serial::COM1.write(b"\r\nKernel heap free bytes: 0x");
            serial::COM1.write_hex(heap_status.free_bytes as u64);
            serial::COM1.write(b"\r\nP2.4: kernel heap allocator ready.\r\n");

            serial::COM1.write(b"P2.5: initializing kernel address-space manager.\r\n");
            let kernel_root = match address_space::initialize() {
                Ok(root) => root,
                Err(error) => {
                    serial::COM1.write(b"P2.5: address-space initialization failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            serial::COM1.write(b"Kernel address-space root: 0x");
            serial::COM1.write_hex(kernel_root);
            serial::COM1.write(b"\r\n");

            let address_space_status = match address_space::smoke_test() {
                Ok(status) => status,
                Err(error) => {
                    let _ = address_space::restore_kernel();
                    serial::COM1.write(b"P2.5: address-space smoke test failed: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                    halt_forever();
                }
            };
            serial::COM1.write(b"Address-space A PML4: 0x");
            serial::COM1.write_hex(address_space_status.first_root);
            serial::COM1.write(b"\r\nAddress-space B PML4: 0x");
            serial::COM1.write_hex(address_space_status.second_root);
            serial::COM1.write(b"\r\nShared kernel translation: 0x");
            serial::COM1.write_hex(address_space_status.shared_kernel_translation);
            serial::COM1.write(b"\r\nShared heap translation: 0x");
            serial::COM1.write_hex(address_space_status.shared_heap_translation);
            serial::COM1.write(b"\r\nUser virtual test page: 0x");
            serial::COM1.write_hex(address_space_status.user_virtual_page);
            serial::COM1.write(b"\r\nAddress-space A physical page: 0x");
            serial::COM1.write_hex(address_space_status.first_physical_page);
            serial::COM1.write(b"\r\nAddress-space B physical page: 0x");
            serial::COM1.write_hex(address_space_status.second_physical_page);
            serial::COM1.write(b"\r\nP2.5: separate roots, shared kernel maps, private user mappings and CR3 switching verified.\r\n");
            serial::COM1
                .write(b"P2.5: private page tables reclaimed; data frames safely returned.\r\n");
            serial::COM1.write(b"P2.5: kernel address-space smoke test passed.\r\n");

            // SAFETY: GOP memory remains allocated after ExitBootServices and is
            // covered by the active 0..4 GiB supervisor identity mapping.
            match unsafe { framebuffer::initialize(&info.framebuffer) } {
                Ok(mut console) => {
                    console.set_colors(Color::new(205, 225, 255), Color::new(8, 12, 22));
                    console.clear();
                    console.write(b"EXHIOS KERNEL 0.1\r\n");
                    console.write(b"P3.2 FRAMEBUFFER CONSOLE READY\r\n");
                    console.write(b"RESOLUTION: ");
                    console.write_u32(info.framebuffer.width);
                    console.write(b" X ");
                    console.write_u32(info.framebuffer.height);
                    console.write(b"\r\nSCANLINE STRIDE: ");
                    console.write_u32(info.framebuffer.pixels_per_scan_line);
                    console.write(b"\r\nPIXEL FORMAT: ");
                    match info.framebuffer.pixel_format {
                        0 => console.write(b"RGBX"),
                        1 => console.write(b"BGRX"),
                        _ => console.write(b"UNSUPPORTED"),
                    }
                    console.write(b"\r\nCOM1 SERIAL: READY\r\n");
                    framebuffer_console = Some(console);
                    serial::COM1.write(b"P3.2: framebuffer console initialized; GOP format and stride validated.\r\n");
                }
                Err(error) => {
                    serial::COM1.write(b"P3.2: framebuffer console unavailable: ");
                    serial::COM1.write(error.message());
                    serial::COM1.write(b"\r\n");
                }
            }

            #[cfg(all(
                feature = "qemu-debug-exit-smoke-test",
                not(any(
                    feature = "timer-smoke-test",
                    feature = "keyboard-smoke-test",
                    feature = "driver-registry-smoke-test",
                    feature = "paging-rollback-smoke-test",
                    feature = "vfs-smoke-test",
                    feature = "process-load-smoke-test",
                    feature = "context-switch-smoke-test",
                    feature = "task-accounting-smoke-test",
                    feature = "timer-preemption-smoke-test"
                ))
            ))]
            {
                serial::COM1.write(b"P2.6: forcing an exception-frame write into the unmapped stack guard page.\r\n");
                idt::trigger_double_fault_guard_stack_smoke_test(
                    stack_guard_reservation.stack_base,
                    stack_guard_reservation.guard_page,
                );
            }
        } else {
            serial::COM1.write(b"BootInfo: invalid; cannot initialize physical memory.\r\n");
            halt_forever();
        }
    } else {
        serial::COM1.write(b"BootInfo: not supplied; cannot initialize physical memory.\r\n");
        halt_forever();
    }

    let mut kernel_console = KernelConsole::new(framebuffer_console);
    kernel_console.write_fmt(format_args!("ExhiOS: kernel handoff successful.\r\n"));
    if kernel_console.has_framebuffer() {
        kernel_console.write(b"P3.2: unified COM1/framebuffer writer ready.\r\n");
    }
    kernel_console.write(b"\r\nKERNEL HANDOFF: SUCCESS\r\n");

    kernel_console.write_fmt(format_args!(
        "P3.5: driver registry ready; registered={} initialized={} failed={}\r\n",
        driver_registry.len(),
        driver_registry.initialized_count(),
        driver_registry.failed_count()
    ));
    for entry in driver_registry.iter() {
        kernel_console.write_fmt(format_args!(
            "P3.5: driver {} id={:?} class={:?} state={:?}\r\n",
            entry.descriptor.name(),
            entry.descriptor.id(),
            entry.descriptor.class(),
            entry.state
        ));
    }

    #[cfg(feature = "vfs-smoke-test")]
    {
        let result = vfs_storage_smoke_test();
        match result {
            Ok(()) => kernel_console.write(b"P6: VFS/RAMFS runtime smoke test passed.\r\n"),
            Err(error) => {
                kernel_console.write(b"P6: VFS/RAMFS runtime smoke test failed: ");
                kernel_console.write(error.as_str().as_bytes());
                kernel_console.write(b"\r\n");
            }
        }
        let exit_value = if result.is_ok() { 0x10u32 } else { 0x11u32 };
        unsafe {
            asm!(
                "out dx, eax",
                in("dx") 0xF4u16,
                in("eax") exit_value,
                options(nomem, nostack, preserves_flags)
            );
        }
        halt_forever();
    }

    #[cfg(feature = "process-load-smoke-test")]
    {
        kernel_console.write(b"P7.1: loading the first user ET_EXEC image.\r\n");
        let result = arch::x86_64::process::smoke_test(EMBEDDED_INIT_ELF);
        let exit_value = match result {
            Ok(report) => {
                kernel_console.write_fmt(format_args!(
                    "P7.1: init ELF loaded; entry=0x{:016X} image-pages={} stack-pages={} stack=0x{:016X}.\r\n",
                    report.entry_point,
                    report.image_pages,
                    report.stack_pages,
                    report.user_stack_pointer,
                ));
                kernel_console.write(b"P7.1: RX/RW+NX PT_LOAD, entry bytes, zeroed BSS, guard page and full frame reclamation verified.\r\n");
                QEMU_DEBUG_EXIT_VALUE
            }
            Err(error) => {
                kernel_console
                    .write_fmt(format_args!("P7.1: init ELF load failed: {:?}.\r\n", error));
                QEMU_DEBUG_EXIT_VALUE + 1
            }
        };
        unsafe {
            asm!(
                "out dx, eax",
                in("dx") QEMU_DEBUG_EXIT_PORT,
                in("eax") exit_value,
                options(nomem, nostack, preserves_flags)
            );
        }
        halt_forever();
    }

    #[cfg(feature = "timer-smoke-test")]
    {
        let timer_status = timer::initialize();
        kernel_console.write_fmt(format_args!(
            "P3.3: PIT initialized at {} Hz (divisor {}); IRQ0 unmasked.\r\n",
            timer_status.requested_hz, timer_status.pit_divisor
        ));
        if !timer_status.irq0_unmasked {
            let _ = driver_registry.set_state(timer::DRIVER.id(), DriverState::Failed);
            kernel_console.write(b"P3.3: IRQ0 could not be enabled; refusing to execute STI.\r\n");
            halt_forever();
        }
        if driver_registry
            .set_state(timer::DRIVER.id(), DriverState::Initialized)
            .is_err()
        {
            kernel_console.write(b"P3.5: PIT driver registry transition failed.\r\n");
            halt_forever();
        }

        let start_tick = timer::ticks();
        // STI followed by HLT is an atomic wait pattern on x86: an interrupt
        // arriving after the condition check is delivered after HLT executes.
        while timer::elapsed_ticks(start_tick) < 5 {
            unsafe {
                asm!("sti", "hlt", options(nomem, nostack));
            }
        }
        unsafe {
            asm!("cli", options(nomem, nostack, preserves_flags));
        }

        let elapsed = timer::elapsed_ticks(start_tick);
        let pic_status = timer::mask_irq0();
        kernel_console.write_fmt(format_args!(
            "P3.3: monotonic timer advanced by {} ticks; IRQ0 masked again.\r\n",
            elapsed
        ));
        if elapsed < 5 || pic_status.master_mask & 0x01 == 0 || pic_status.slave_mask != 0xFF {
            kernel_console.write(b"P3.3: PIT timer smoke test failed.\r\n");
            halt_forever();
        }
        kernel_console.write(b"P3.3: PIT IRQ0 runtime smoke test passed.\r\n");
    }

    #[cfg(feature = "serial-rx-smoke-test")]
    {
        serial_rx_smoke_test()
    }

    #[cfg(feature = "driver-registry-smoke-test")]
    {
        let serial_entry = driver_registry.get(DriverId::SerialCom1);
        let pit_entry = driver_registry.get(DriverId::PitChannel0);
        let keyboard_entry = driver_registry.get(DriverId::Ps2Keyboard);
        let registry_valid = driver_registry.len() == 3
            && driver_registry.initialized_count() == 1
            && driver_registry.failed_count() == 0
            && serial_entry.is_some_and(|entry| {
                entry.descriptor.class() == DriverClass::Serial
                    && entry.state == DriverState::Initialized
            })
            && pit_entry.is_some_and(|entry| {
                entry.descriptor.class() == DriverClass::Clock
                    && entry.state == DriverState::Registered
            })
            && keyboard_entry.is_some_and(|entry| {
                entry.descriptor.class() == DriverClass::Input
                    && entry.state == DriverState::Registered
            });

        if registry_valid {
            kernel_console.write(b"P3.5: driver registry runtime smoke test passed.\r\n");
        } else {
            kernel_console.write(b"P3.5: driver registry runtime smoke test failed.\r\n");
            halt_forever();
        }

        kernel_console.write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
        unsafe {
            asm!(
                "out dx, eax",
                in("dx") QEMU_DEBUG_EXIT_PORT,
                in("eax") QEMU_DEBUG_EXIT_VALUE,
                options(nomem, nostack, preserves_flags)
            );
        }
        kernel_console.write(b"QEMU debug-exit: device did not terminate the guest.\r\n");
        halt_forever();
    }

    #[cfg(feature = "context-switch-smoke-test")]
    {
        kernel_console.write(b"P4.2: starting x86-64 cooperative context-switch smoke test.\r\n");
        let passed = arch::x86_64::task::context_switch_smoke_test();
        if passed {
            kernel_console.write(
                b"P4.2: two tasks yielded to each other and preserved independent stacks.\r\n",
            );
            kernel_console
                .write(b"P4.2: cooperative context-switch runtime smoke test passed.\r\n");
        } else {
            kernel_console
                .write(b"P4.2: cooperative context-switch runtime smoke test failed.\r\n");
        }

        let exit_value = if passed {
            QEMU_DEBUG_EXIT_VALUE
        } else {
            QEMU_DEBUG_EXIT_VALUE + 1
        };
        kernel_console.write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
        unsafe {
            asm!(
                "out dx, eax",
                in("dx") QEMU_DEBUG_EXIT_PORT,
                in("eax") exit_value,
                options(nomem, nostack, preserves_flags)
            );
        }
        kernel_console.write(b"QEMU debug-exit: device did not terminate the guest.\r\n");
        halt_forever();
    }

    #[cfg(feature = "task-accounting-smoke-test")]
    {
        kernel_console
            .write(b"P4.3: starting PIT-backed scheduler runtime-accounting smoke test.\r\n");
        let passed = arch::x86_64::task::timer_accounting_smoke_test();
        if passed {
            kernel_console
                .write(b"P4.3: scheduler recorded independent task runtime from PIT ticks.\r\n");
            kernel_console
                .write(b"P4.3: cooperative task runtime-accounting smoke test passed.\r\n");
        } else {
            kernel_console
                .write(b"P4.3: cooperative task runtime-accounting smoke test failed.\r\n");
        }

        let exit_value = if passed {
            QEMU_DEBUG_EXIT_VALUE
        } else {
            QEMU_DEBUG_EXIT_VALUE + 1
        };
        kernel_console.write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
        unsafe {
            asm!(
                "out dx, eax",
                in("dx") QEMU_DEBUG_EXIT_PORT,
                in("eax") exit_value,
                options(nomem, nostack, preserves_flags)
            );
        }
        kernel_console.write(b"QEMU debug-exit: device did not terminate the guest.\r\n");
        halt_forever();
    }

    #[cfg(feature = "user-task-lifecycle-smoke-test")]
    {
        kernel_console.write(b"P5.6: starting two-user-task syscall lifecycle test.\r\n");
        arch::x86_64::user_mode::user_task_lifecycle_smoke_test();
    }

    #[cfg(feature = "user-copy-smoke-test")]
    {
        kernel_console.write(b"P5.5: starting safe user-copy runtime test.\r\n");
        arch::x86_64::user_mode::user_copy_smoke_test();
    }

    #[cfg(feature = "user-pointer-range-smoke-test")]
    {
        kernel_console.write(b"P5.4: starting the user-pointer range validation test.\r\n");
        arch::x86_64::user_mode::user_pointer_validation_smoke_test();
    }

    #[cfg(feature = "ring3-syscall-smoke-test")]
    {
        kernel_console.write(b"P5.3: starting the Ring 3 SYSCALL/SYSRET round-trip test.\r\n");
        arch::x86_64::user_mode::ring3_syscall_smoke_test();
    }

    #[cfg(feature = "ring3-supervisor-fault-smoke-test")]
    {
        kernel_console.write(b"P5.2: starting the Ring 3 supervisor-only access fault test.\r\n");
        arch::x86_64::user_mode::ring3_supervisor_fault_smoke_test();
    }

    #[cfg(all(
        feature = "ring3-smoke-test",
        not(any(
            feature = "ring3-supervisor-fault-smoke-test",
            feature = "ring3-syscall-smoke-test",
            feature = "user-pointer-range-smoke-test",
            feature = "user-copy-smoke-test",
            feature = "user-task-lifecycle-smoke-test"
        ))
    ))]
    {
        kernel_console
            .write(b"P5.1: starting the Ring 3 entry and exception-return smoke test.\r\n");
        arch::x86_64::user_mode::ring3_smoke_test();
    }

    #[cfg(feature = "idle-wakeup-smoke-test")]
    {
        kernel_console.write(b"P4.6: starting idle-context and timed task wake-up smoke test.\r\n");
        arch::x86_64::task::idle_wakeup_smoke_test();
    }

    #[cfg(feature = "preemptive-task-exit-smoke-test")]
    {
        kernel_console.write(b"P4.5: starting preemptive task-exit lifecycle smoke test.\r\n");
        arch::x86_64::task::preemptive_task_exit_smoke_test();
    }

    #[cfg(all(
        feature = "timer-preemption-smoke-test",
        not(any(
            feature = "preemptive-task-exit-smoke-test",
            feature = "idle-wakeup-smoke-test",
            feature = "ring3-smoke-test"
        ))
    ))]
    {
        kernel_console.write(b"P4.4: starting timer-driven task preemption smoke test.\r\n");
        arch::x86_64::task::timer_preemption_smoke_test();
    }

    #[cfg(feature = "keyboard-smoke-test")]
    {
        keyboard_smoke_test(&mut kernel_console, &mut driver_registry);
    }

    #[cfg(not(any(
        feature = "serial-rx-smoke-test",
        feature = "keyboard-smoke-test",
        feature = "driver-registry-smoke-test",
        feature = "context-switch-smoke-test",
        feature = "task-accounting-smoke-test",
        feature = "timer-preemption-smoke-test",
        feature = "idle-wakeup-smoke-test"
    )))]
    {
        #[cfg(feature = "qemu-debug-exit-smoke-test")]
        {
            kernel_console.write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
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
            kernel_console.write(b"QEMU debug-exit: device did not terminate the guest.\r\n");
        }

        kernel_console
            .write(b"ExhiOS: interrupts remain disabled until IRQ handlers are installed.\r\n");
        kernel_console.write(b"ExhiOS: halt loop entered.\r\n");

        halt_forever()
    }
}

/// Fatal CPU-exception entry called by the assembly stubs.
/// Arguments follow the x86-64 System V ABI.
#[cfg(feature = "vfs-smoke-test")]
fn vfs_storage_smoke_test() -> Result<(), exhi_vfs::VfsError> {
    use exhi_vfs::{
        create_directory_path, create_file_path, resolve_path, DirectoryEntry, FileSystem,
        NodeKind, RamFs, VfsError,
    };

    let mut filesystem = RamFs::<8, 256>::new()?;
    let root = filesystem.root();
    let etc = create_directory_path(&mut filesystem, root, "/etc")?;
    let config = create_file_path(&mut filesystem, root, "/etc/exhi.conf")?;
    let data = b"boot.mode=ramfs\\nconsole=serial\\n";
    if filesystem.write(config, 0, data)? != data.len() {
        return Err(VfsError::FileTooLarge);
    }
    if resolve_path(&filesystem, root, "/etc//./exhi.conf")? != config
        || resolve_path(&filesystem, etc, "../etc/exhi.conf")? != config
    {
        return Err(VfsError::NotFound);
    }

    let mut readback = [0u8; 64];
    let count = filesystem.read(config, 0, &mut readback)?;
    if count != data.len() || &readback[..count] != data {
        return Err(VfsError::NotFound);
    }

    let placeholder = DirectoryEntry {
        id: root,
        name: filesystem.metadata(root)?.name,
        kind: NodeKind::Directory,
        size: 0,
    };
    let mut entries = [placeholder; 2];
    let mut cursor = 0;
    let first = filesystem.read_directory(etc, &mut cursor, &mut entries)?;
    if first != 1 || entries[0].id != config || entries[0].kind != NodeKind::File {
        return Err(VfsError::NotFound);
    }

    if filesystem.write(config, 250, b"too large") != Err(VfsError::FileTooLarge) {
        return Err(VfsError::FileTooLarge);
    }
    if filesystem.remove(etc) != Err(VfsError::DirectoryNotEmpty) {
        return Err(VfsError::DirectoryNotEmpty);
    }
    filesystem.remove(config)?;
    filesystem.remove(etc)?;

    let stale = filesystem.create_file(root, "stale")?;
    filesystem.remove(stale)?;
    let fresh = filesystem.create_file(root, "fresh")?;
    if stale.slot() != fresh.slot()
        || stale.generation() == fresh.generation()
        || filesystem.metadata(stale) != Err(VfsError::StaleNode)
    {
        return Err(VfsError::StaleNode);
    }
    Ok(())
}

#[no_mangle]
pub extern "sysv64" fn exhi_exception_handler(
    vector: u64,
    error_code: u64,
    instruction_pointer: u64,
    code_selector: u64,
    flags: u64,
    exception_stack_pointer: u64,
) -> ! {
    serial::COM1.write(b"\r\nExhiOS CPU EXCEPTION\r\n");
    serial::COM1.write(b"Vector: 0x");
    serial::COM1.write_hex(vector);
    serial::COM1.write(b" (");
    serial::COM1.write(exception_name(vector));
    serial::COM1.write(b")\r\nError code: 0x");
    serial::COM1.write_hex(error_code);
    serial::COM1.write(b"\r\nRIP: 0x");
    serial::COM1.write_hex(instruction_pointer);
    serial::COM1.write(b"\r\nCS: 0x");
    serial::COM1.write_hex(code_selector);
    serial::COM1.write(b"\r\nRFLAGS: 0x");
    serial::COM1.write_hex(flags);
    serial::COM1.write(b"\r\n");

    if code_selector & 3 == 3 {
        #[cfg(feature = "user-task-lifecycle-smoke-test")]
        arch::x86_64::user_mode::handle_user_exception(
            vector,
            error_code,
            instruction_pointer,
            code_selector,
            flags,
            exception_stack_pointer,
        );

        #[cfg(not(feature = "user-task-lifecycle-smoke-test"))]
        {
            let user_exception = match arch::x86_64::user::capture_exception(
                vector,
                error_code,
                instruction_pointer,
                code_selector,
                flags,
                exception_stack_pointer,
            ) {
                Ok(exception) => exception,
                Err(_) => {
                    serial::COM1.write(
                        b"User exception frame is outside the active kernel stack or malformed.\r\n",
                    );
                    halt_forever();
                }
            };
            serial::COM1.write(b"User exception: RIP=0x");
            serial::COM1.write_hex(user_exception.instruction_pointer);
            serial::COM1.write(b" user RSP=0x");
            serial::COM1.write_hex(user_exception.user_stack_pointer);
            serial::COM1.write(b" error=0x");
            serial::COM1.write_hex(user_exception.error_code);
            serial::COM1.write(b"\r\n");

            #[cfg(feature = "ring3-smoke-test")]
            arch::x86_64::user_mode::handle_user_exception(
                vector,
                error_code,
                instruction_pointer,
                code_selector,
                flags,
                exception_stack_pointer,
            );

            serial::COM1.write(b"No user-task fault termination policy is installed.\r\n");
            halt_forever();
        }
    }

    // A fatal exception in the opt-in P4.4 QEMU test must terminate the guest
    // with the failure debug-exit value instead of leaving CI stuck in HLT.
    #[cfg(feature = "timer-preemption-smoke-test")]
    {
        serial::COM1.write(b"P4.4: fatal CPU exception; aborting preemption smoke test.\r\n");
        unsafe {
            asm!(
                "out dx, eax",
                in("dx") QEMU_DEBUG_EXIT_PORT,
                in("eax") (QEMU_DEBUG_EXIT_VALUE + 1),
                options(nomem, nostack, preserves_flags)
            );
        }
        serial::COM1.write(b"P4.4: QEMU debug-exit device did not terminate the guest.\r\n");
    }

    if vector == 8 {
        serial::COM1.write(b"Exception stack pointer: 0x");
        serial::COM1.write_hex(exception_stack_pointer);
        serial::COM1.write(b"\r\n");
        if gdt::double_fault_ist_contains(exception_stack_pointer) {
            serial::COM1.write(b"P2.6: double fault arrived on the dedicated IST1 stack.\r\n");
            #[cfg(feature = "qemu-debug-exit-smoke-test")]
            {
                serial::COM1.write(b"P2.6: double-fault IST runtime smoke test passed.\r\n");
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
                serial::COM1
                    .write(b"QEMU debug-exit: device did not terminate after IST test.\r\n");
            }
        } else {
            serial::COM1.write(b"P2.6: double fault did not use the dedicated IST1 stack.\r\n");
        }
    }

    if vector == 14 {
        let fault_address: u64;
        // SAFETY: Reading CR2 is valid in ring 0 and does not modify CPU state.
        unsafe {
            asm!("mov {}, cr2", out(reg) fault_address, options(nomem, nostack, preserves_flags));
        }
        serial::COM1.write(b"CR2 (page-fault address): 0x");
        serial::COM1.write_hex(fault_address);
        serial::COM1.write(b"\r\n");
    }

    serial::COM1.write(b"ExhiOS: CPU halted after fatal exception.\r\n");
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
        heap::HeapInitError::InvalidPhysicalFrame => {
            b"physical allocator returned an invalid frame"
        }
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

#[cfg(feature = "keyboard-smoke-test")]
fn keyboard_smoke_test(console: &mut KernelConsole, drivers: &mut DriverRegistry<8>) -> ! {
    let status = match keyboard::initialize() {
        Ok(status) => status,
        Err(error) => {
            let _ = drivers.set_state(keyboard::DRIVER.id(), DriverState::Failed);
            console.write(b"P3.4: keyboard initialization failed: ");
            console.write(error.message());
            if let Some(response) = error.response() {
                console.write_fmt(format_args!(" (response 0x{:02X})", response));
            }
            console.write(b"\r\n");
            halt_forever();
        }
    };
    if !status.is_valid() {
        console.write(b"P3.4: keyboard status verification failed.\r\n");
        halt_forever();
    }

    if drivers
        .set_state(keyboard::DRIVER.id(), DriverState::Initialized)
        .is_err()
    {
        console.write(b"P3.5: PS/2 driver registry transition failed.\\r\\n");
        halt_forever();
    }
    console.write_fmt(format_args!(
        "P3.4: PS/2 keyboard ready; i8042 config=0x{:02X}, IRQ1 unmasked.\r\n",
        status.controller_config
    ));
    console.write(b"P3.4: waiting for injected A, Shift+B and Ctrl+D key transitions.\r\n");

    let mut event_count = 0u64;
    let mut saw_a_press = false;
    let mut saw_shift_b_press = false;
    loop {
        while let Some(event) = keyboard::try_read_event() {
            event_count = event_count.saturating_add(1);
            saw_a_press |= event.pressed && event.key == exhi_input::KeyCode::A;
            saw_shift_b_press |=
                event.pressed && event.key == exhi_input::KeyCode::B && event.modifiers.shift;
            console.write_fmt(format_args!(
                "P3.4: {} key=0x{:02X} shift={} ctrl={} alt={} meta={} caps={} num={} scroll={}\r\n",
                if event.pressed { "DOWN" } else { "UP" },
                event.key.0,
                event.modifiers.shift,
                event.modifiers.control,
                event.modifiers.alt,
                event.modifiers.meta,
                event.modifiers.caps_lock,
                event.modifiers.num_lock,
                event.modifiers.scroll_lock
            ));
            if event.pressed && event.key == exhi_input::KeyCode::D && event.modifiers.control {
                unsafe {
                    asm!("cli", options(nomem, nostack));
                }
                let pic_status = keyboard::mask_irq1();
                let dropped = keyboard::dropped_scancodes();
                console.write_fmt(format_args!(
                    "P3.4: events={} dropped-scancodes={} IRQ1-mask=0x{:02X}\r\n",
                    event_count, dropped, pic_status.master_mask
                ));
                if dropped == 0
                    && event_count >= 6
                    && saw_a_press
                    && saw_shift_b_press
                    && pic_status.master_mask & 0x02 != 0
                {
                    console.write(
                        b"P3.4: A, Shift+B and Ctrl+D events verified; IRQ1 masked cleanly.\r\n",
                    );
                } else {
                    console.write(b"P3.4: keyboard queue or PIC mask verification failed.\r\n");
                    halt_forever();
                }
                console.write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
                unsafe {
                    asm!(
                        "out dx, eax",
                        in("dx") QEMU_DEBUG_EXIT_PORT,
                        in("eax") QEMU_DEBUG_EXIT_VALUE,
                        options(nomem, nostack, preserves_flags)
                    );
                }
                console.write(b"QEMU debug-exit: device did not terminate the guest.\r\n");
                halt_forever();
            }
        }
        unsafe {
            asm!("sti", "hlt", options(nomem, nostack));
        }
    }
}

#[cfg(feature = "serial-rx-smoke-test")]
fn serial_rx_smoke_test() -> ! {
    serial::COM1.write(
        b"\r\nP3.1 RX test ready. Type text to see it echoed; press Ctrl+D to stop.\r\nRX> ",
    );

    let mut received_bytes = 0u64;
    let mut previous_was_cr = false;

    loop {
        let byte = serial::COM1.read_byte_blocking();
        if byte == 0x04 {
            serial::COM1.write(b"\r\nP3.1: received input bytes: 0x");
            serial::COM1.write_hex(received_bytes);
            serial::COM1.write(b"\r\nP3.1: interactive COM1 RX test ended.\r\n");
            halt_forever();
        }

        received_bytes = received_bytes.saturating_add(1);

        if byte == b'\n' && previous_was_cr {
            previous_was_cr = false;
            continue;
        }
        previous_was_cr = byte == b'\r';

        match byte {
            b'\r' | b'\n' => serial::COM1.write(b"\r\nRX> "),
            0x08 | 0x7F => serial::COM1.write(b"\x08 \x08"),
            0x20..=0x7E => {
                let _ = serial::COM1.write_byte(byte);
            }
            b'\t' => serial::COM1.write(b"    "),
            _ => {}
        }
    }
}

fn register_builtin_drivers() -> Result<DriverRegistry<8>, RegistryError> {
    let mut registry = DriverRegistry::<8>::new();

    registry.register(serial::DRIVER)?;
    registry.set_state(serial::DRIVER.id(), DriverState::Initialized)?;

    registry.register(timer::DRIVER)?;
    registry.register(keyboard::DRIVER)?;

    Ok(registry)
}

fn halt_forever() -> ! {
    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    serial::COM1.write(b"ExhiOS Kernel panic.\r\n");
    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
