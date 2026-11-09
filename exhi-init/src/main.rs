#![no_main]
#![no_std]

use core::hint::spin_loop;
use core::panic::PanicInfo;

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
use exhi_config::ConfigParser;
#[cfg(not(feature = "user-fault-smoke-test"))]
use libexhi::{console_read, yield_now};
#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
use libexhi::file_read;
use libexhi::{console_write, exit};

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
const LINE_CAPACITY: usize = 128;
#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
const INPUT_CHUNK: usize = 32;
#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
#[derive(Clone, Copy)]
struct ShellConfig<'a> {
    hostname: &'a str,
    prompt: &'a str,
    version: &'a str,
}

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
const DEFAULT_CONFIG: ShellConfig<'static> = ShellConfig {
    hostname: "exhios",
    prompt: "exhios>",
    version: "0.1",
};

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
fn load_shell_config(source: &str) -> Result<ShellConfig<'_>, ()> {
    let mut config = DEFAULT_CONFIG;
    let mut hostname_seen = false;
    let mut prompt_seen = false;
    let mut version_seen = false;

    for entry in ConfigParser::new(source) {
        let entry = entry.map_err(|_| ())?;
        match entry.key {
            "hostname" => {
                if hostname_seen
                    || entry.value.is_empty()
                    || !entry
                        .value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
                {
                    return Err(());
                }
                config.hostname = entry.value;
                hostname_seen = true;
            }
            "prompt" => {
                if prompt_seen || entry.value.is_empty() {
                    return Err(());
                }
                config.prompt = entry.value;
                prompt_seen = true;
            }
            "version" => {
                if version_seen || entry.value.is_empty() {
                    return Err(());
                }
                config.version = entry.value;
                version_seen = true;
            }
            _ => {}
        }
    }

    Ok(config)
}

#[no_mangle]
pub extern "sysv64" fn _start() -> ! {
    if console_write(b"ExhiOS init: first Ring 3 process is running.\r\n").is_err() {
        exit();
    }

    #[cfg(feature = "user-fault-smoke-test")]
    trigger_user_fault();

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

    #[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
    run_shell()
}

#[cfg(feature = "user-fault-smoke-test")]
fn trigger_user_fault() -> ! {
    let _ = console_write(b"ExhiOS init: triggering user-mode invalid opcode.\r\n");
    unsafe {
        core::arch::asm!("ud2", options(noreturn));
    }
}

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
fn run_shell() -> ! {
    let mut config_buffer = [0u8; 256];
    let config = match file_read("/etc/exhi.conf", 0, &mut config_buffer) {
        Ok(count) => match core::str::from_utf8(&config_buffer[..count])
            .ok()
            .and_then(|source| load_shell_config(source).ok())
        {
            Some(config) => config,
            None => {
                let _ = console_write(
                    b"ExhiOS init: invalid VFS configuration; using defaults.\r\n",
                );
                DEFAULT_CONFIG
            }
        },
        Err(_) => {
            let _ = console_write(
                b"ExhiOS init: configuration read failed; using defaults.\r\n",
            );
            DEFAULT_CONFIG
        }
    };
    let _ = console_write(b"ExhiOS userland ");
    let _ = console_write(config.version.as_bytes());
    let _ = console_write(b"\r\nType 'help' for available commands.\r\n");
    write_prompt(&config);

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
                    let should_exit = execute_command(&line[..line_length], &config);
                    line_length = 0;
                    line.fill(0);
                    if should_exit {
                        exit();
                    }
                    write_prompt(&config);
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
                    write_prompt(&config);
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

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
fn write_prompt(config: &ShellConfig<'_>) {
    let _ = console_write(config.prompt.as_bytes());
    let _ = console_write(b" ");
}

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
fn execute_command(line: &[u8], config: &ShellConfig<'_>) -> bool {
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
            let _ = console_write(b"commands: help hostname version echo clear config exit\r\n");
        }
        "hostname" => {
            let _ = console_write(config.hostname.as_bytes());
            let _ = console_write(b"\r\n");
        }
        "version" => {
            let _ = console_write(b"ExhiOS userland ");
            let _ = console_write(config.version.as_bytes());
            let _ = console_write(b"\r\n");
        }
        "echo" => {
            let _ = console_write(arguments.as_bytes());
            let _ = console_write(b"\r\n");
        }
        "clear" => {
            let _ = console_write(b"\x1B[2J\x1B[H");
        }
        "config" => print_config(config),
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

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
fn print_config(config: &ShellConfig<'_>) {
    let _ = console_write(b"hostname=");
    let _ = console_write(config.hostname.as_bytes());
    let _ = console_write(b"\r\nprompt=");
    let _ = console_write(config.prompt.as_bytes());
    let _ = console_write(b"\r\nversion=");
    let _ = console_write(config.version.as_bytes());
    let _ = console_write(b"\r\n");
}

#[cfg(not(any(feature = "qemu-smoke-test", feature = "user-fault-smoke-test")))]
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
