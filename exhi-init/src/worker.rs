#![no_main]
#![no_std]

use core::hint::spin_loop;
use core::panic::PanicInfo;

use libexhi::{console_write, exit, spawn_worker, SyscallError};
#[cfg(not(feature = "worker-fault-smoke-test"))]
use libexhi::yield_now;

#[no_mangle]
pub extern "sysv64" fn _start() -> ! {
    if console_write(b"ExhiOS worker: second Ring 3 process started.\r\n").is_err() {
        exit();
    }

    // Worker must not inherit the parent's TaskSpawn capability. Check the
    // exact denial result, rather than accepting a later resource-slot error.
    if !matches!(spawn_worker(), Err(SyscallError::AccessDenied)) {
        let _ = console_write(b"ExhiOS worker: ERROR TaskSpawn was not denied.\r\n");
        exit();
    }
    if console_write(b"ExhiOS worker: TaskSpawn denied as expected.\r\n").is_err() {
        exit();
    }

    #[cfg(feature = "worker-fault-smoke-test")]
    {
        let _ = console_write(b"ExhiOS worker: triggering invalid-opcode fault.\r\n");
        unsafe {
            core::arch::asm!("ud2", options(noreturn));
        }
    }

    #[cfg(not(feature = "worker-fault-smoke-test"))]
    {
        for _ in 0..4 {
            if yield_now().is_err() {
                exit();
            }
        }

        let _ = console_write(b"ExhiOS worker: completed yield sequence; exiting.\r\n");
        exit();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    let _ = console_write(b"ExhiOS worker: panic.\r\n");
    loop {
        spin_loop();
    }
}
