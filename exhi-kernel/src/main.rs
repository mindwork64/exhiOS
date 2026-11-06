#![no_main]
#![no_std]

mod arch;

use arch::x86_64::{gdt, idt, pic};
use core::arch::asm;
use core::hint::spin_loop;
use core::panic::PanicInfo;

use exhi_abi::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};

const COM1: u16 = 0x3F8;
const LINE_STATUS: u16 = COM1 + 5;
const QEMU_DEBUG_EXIT_PORT: u16 = 0xF4;
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

    serial_write(b"GDT: installing kernel GDT.\r\n");
    gdt::initialize();

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

    if !selectors.is_valid() {
        serial_write(b"GDT: selector verification failed.\r\n");
        halt_forever();
    }

    serial_write(b"GDT: installed and verified.\r\n");

    serial_write(b"IDT: installing kernel IDT.\r\n");
    let idt_status = idt::initialize();
    serial_write(b"IDT base: 0x");
    serial_write_hex(idt_status.base);
    serial_write(b"\r\nIDT limit: 0x");
    serial_write_hex(u64::from(idt_status.limit));
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

    serial_write(b"PIC: remapped and verified; all IRQ lines remain masked.\r\n");
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
        } else {
            serial_write(b"BootInfo: invalid\r\n");
        }
    } else {
        serial_write(b"BootInfo: not supplied\r\n");
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
