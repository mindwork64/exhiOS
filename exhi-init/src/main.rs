#![no_main]
#![no_std]

use core::hint::spin_loop;
use core::panic::PanicInfo;

use exhi_config::ConfigParser;
use libexhi::{console_read, console_write, exit, yield_now};

const LINE_CAPACITY: usize = 128;
const INPUT_CHUNK: usize = 32;
const CONFIG_TEXT: &str = "# ExhiOS system configuration\nhostname=exhios\nprompt=exhios>\nversion=0.1\n";

#[no_mangle]
pub extern "sysv64" fn _start() -> ! {
    if console_write(b"ExhiOS init: first Ring 3 process is running.\r\n").is_err() {
        exit();
    }

    #[cfg(feature = "qemu-smoke-test")]
    {
        let mut input = [0u8; 1];
        match console_read(&mut input) {
            Ok(0) => {}
            Ok(_) | Err(_) => exit(),
        }
        for _ in 0..3 {
            if yield_now().is_err() {
                exit();
            }
        }
        exit();
    }

    #[cfg(not(feature = "qemu-smoke-test"))]
    run_shell()
}

fn run_shell() -> ! {
    let _ = console_write(
        b"ExhiOS userland 0.1\r\nType 'help' for available commands.\r\n",
    );
    write_prompt();

    let mut line = [0u8; LINE_CAPACITY];
    let mut line_length = 0usize;
    let mut input = [0u8; INPUT_CHUNK];
    let mut swallow_lf = false;

    loop {
        let count = match console_read(&mut input) {
            Ok(count) => count,
            Err(_) => {
                let _ = console_write(b"\r\nExhiOS init: console input failed.\r\n");
                exit();
            }
        };

        if count == 0 {
            if yield_now().is_err() {
                spin_loop();
            }
            continue;
        }

        for &byte in &input[..count] {
            if swallow_lf {
                swallow_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }

            match byte {
                b'\r' | b'\n' => {
                    swallow_lf = byte == b'\r';
                    let _ = console_write(b"\r\n");
                    let should_exit = execute_command(&line[..line_length]);
                    line_length = 0;
                    line.fill(0);
                    if should_exit {
                        exit();
                    }
                    write_prompt();
                }
                0x08 | 0x7F => {
                    if line_length != 0 {
                        line_length -= 1;
                        line[line_length] = 0;
                        let _ = console_write(b"\x08 \x08");
                    }
                }
                0x03 => {
                    line_length = 0;
                    line.fill(0);
                    let _ = console_write(b"^C\r\n");
                    write_prompt();
                }
                0x04 => {
                    if line_length == 0 {
                        let _ = console_write(b"^D\r\n");
                        exit();
                    }
                }
                0x20..=0x7E => {
                    if line_length + 1 < LINE_CAPACITY {
                        line[line_length] = byte;
                        line_length += 1;
                        let _ = console_write(&[byte]);
                    } else {
                        let _ = console_write(b"\x07");
                    }
                }
                _ => {}
            }
        }

        if yield_now().is_err() {
            spin_loop();
        }
    }
}

fn write_prompt() {
    let _ = console_write(b"exhios> ");
}

fn execute_command(line: &[u8]) -> bool {
    let Ok(line) = core::str::from_utf8(line) else {
        let _ = console_write(b"error: command is not valid UTF-8.\r\n");
        return false;
    };
    let line = trim_ascii(line);
    if line.is_empty() {
        return false;
    }

    let split = line
        .as_bytes()
        .iter()
        .position(|byte| *byte == b' ' || *byte == b'\t')
        .unwrap_or(line.len());
    let command = &line[..split];
    let arguments = trim_ascii(&line[split..]);

    match command {
        "help" => {
            let _ = console_write(
                b"commands: help version echo clear config exit\r\n",
            );
        }
        "version" => {
            let _ = console_write(b"ExhiOS userland 0.1\r\n");
        }
        "echo" => {
            let _ = console_write(arguments.as_bytes());
            let _ = console_write(b"\r\n");
        }
        "clear" => {
            let _ = console_write(b"\x1B[2J\x1B[H");
        }
        "config" => print_config(),
        "exit" => {
            let _ = console_write(b"ExhiOS init: exiting.\r\n");
            return true;
        }
        _ => {
            let _ = console_write(b"unknown command: ");
            let _ = console_write(command.as_bytes());
            let _ = console_write(b"\r\n");
        }
    }
    false
}

fn print_config() {
    for entry in ConfigParser::new(CONFIG_TEXT) {
        match entry {
            Ok(entry) => {
                let _ = console_write(entry.key.as_bytes());
                let _ = console_write(b"=");
                let _ = console_write(entry.value.as_bytes());
                let _ = console_write(b"\r\n");
            }
            Err(_) => {
                let _ = console_write(b"config: invalid built-in configuration.\r\n");
                return;
            }
        }
    }
}

fn trim_ascii(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && (bytes[start] == b' ' || bytes[start] == b'\t') {
        start += 1;
    }
    while end > start && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        end -= 1;
    }
    &value[start..end]
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    let _ = console_write(b"ExhiOS init: panic.\r\n");
    loop {
        spin_loop();
    }
}
