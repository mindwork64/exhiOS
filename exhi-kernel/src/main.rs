#![no_main]
#![no_std]

use core::arch::asm;
use core::hint::spin_loop;
use core::panic::PanicInfo;

use exhi_abi::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};

const COM1: u16 = 0x3F8;
const LINE_STATUS: u16 = COM1 + 5;

#[no_mangle]
pub extern "sysv64" fn _start(boot_info: *const BootInfo) -> ! {
    serial_init();
    serial_write(b"ExhiOS Kernel 0.1\r\n");
    serial_write(b"Kernel entry point reached.\r\n");

    if !boot_info.is_null() {
        let info = unsafe { &*boot_info };
        if info.magic == BOOT_INFO_MAGIC
            && info.version == BOOT_INFO_VERSION
            && info.is_valid()
        {
            serial_write(b"BootInfo: valid\r\n");
        } else {
            serial_write(b"BootInfo: invalid\r\n");
        }
    } else {
        serial_write(b"BootInfo: not supplied (kernel build smoke test)\r\n");
    }

    serial_write(b"ExhiOS: halt loop entered.\r\n");

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
