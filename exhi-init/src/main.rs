#![no_main]
#![no_std]

use core::hint::spin_loop;
use core::panic::PanicInfo;

use libexhi::{console_write, yield_now};

#[no_mangle]
pub extern "sysv64" fn _start() -> ! {
    let _ = console_write(b"ExhiOS init: first Ring 3 process is running.\r\n");
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
