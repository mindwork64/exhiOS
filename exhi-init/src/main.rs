#![no_main]
#![no_std]

use core::hint::spin_loop;
use core::panic::PanicInfo;

use libexhi::{console_write, exit, yield_now};

#[no_mangle]
pub extern "sysv64" fn _start() -> ! {
    if console_write(b"ExhiOS init: first Ring 3 process is running.\r\n").is_err() {
        exit();
    }

    #[cfg(feature = "qemu-smoke-test")]
    {
        for _ in 0..3 {
            if yield_now().is_err() {
                exit();
            }
        }
        exit();
    }

    #[cfg(not(feature = "qemu-smoke-test"))]
    loop {
        if yield_now().is_err() {
            spin_loop();
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    let _ = console_write(b"ExhiOS init: panic.\r\n");
    loop {
        spin_loop();
    }
}
