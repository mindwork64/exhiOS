use core::arch::asm;
use core::hint::spin_loop;

use exhi_driver::{DriverClass, DriverDescriptor, DriverId};

pub const DRIVER: DriverDescriptor = DriverDescriptor::new(
    DriverId::SerialCom1,
    DriverClass::Serial,
    "16550 UART COM1",
);

/// A minimal polling 16550-compatible UART port.
///
/// This driver is intentionally single-core and non-interrupt-driven for the
/// early kernel. Callers must not invoke it concurrently until a locking or
/// interrupt-safe console layer is introduced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SerialPort {
    base: u16,
}

impl SerialPort {
    pub const fn new(base: u16) -> Self {
        Self { base }
    }

    /// Configures the UART as 38400 baud, 8 data bits, no parity, one stop bit.
    pub fn initialize(&self) {
        unsafe {
            self.outb(1, 0x00); // Disable UART interrupts.
            self.outb(3, 0x80); // Enable divisor latch.
            self.outb(0, 0x03); // Divisor low byte: 3 (38400 baud at 115200 base).
            self.outb(1, 0x00); // Divisor high byte.
            self.outb(3, 0x03); // 8N1, divisor latch disabled.
            self.outb(2, 0xC7); // Enable FIFO, clear RX/TX queues, 14-byte threshold.
            self.outb(4, 0x0B); // DTR, RTS and OUT2.
        }
    }

    /// Sends one byte. Returns false if the transmitter never becomes ready.
    pub fn write_byte(&self, byte: u8) -> bool {
        for _ in 0..100_000 {
            if unsafe { self.inb(5) } & 0x20 != 0 {
                unsafe {
                    self.outb(0, byte);
                }
                return true;
            }
            spin_loop();
        }
        false
    }

    /// Attempts to receive one byte without waiting.
    ///
    /// Returns None when the UART receive-data-ready bit is clear.
    pub fn try_read_byte(&self) -> Option<u8> {
        if unsafe { self.inb(5) } & 0x01 != 0 {
            Some(unsafe { self.inb(0) })
        } else {
            None
        }
    }

    /// Polls until one byte is received. This is intended for the early
    /// single-core serial shell while interrupts remain disabled.
    pub fn read_byte_blocking(&self) -> u8 {
        loop {
            if let Some(byte) = self.try_read_byte() {
                return byte;
            }
            spin_loop();
        }
    }

    /// Writes bytes until the complete slice is sent or the UART times out.
    pub fn write(&self, bytes: &[u8]) {
        for &byte in bytes {
            if !self.write_byte(byte) {
                break;
            }
        }
    }

    /// Writes a fixed-width, 16-digit uppercase hexadecimal u64.
    pub fn write_hex(&self, value: u64) {
        const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
        for shift in (0..16).rev() {
            let digit = ((value >> (shift * 4)) & 0xF) as usize;
            self.write(&HEX_DIGITS[digit..digit + 1]);
        }
    }

    unsafe fn outb(&self, offset: u16, value: u8) {
        unsafe {
            asm!(
                "out dx, al",
                in("dx") self.base + offset,
                in("al") value,
                options(nomem, nostack, preserves_flags)
            );
        }
    }

    unsafe fn inb(&self, offset: u16) -> u8 {
        let value: u8;
        unsafe {
            asm!(
                "in al, dx",
                in("dx") self.base + offset,
                out("al") value,
                options(nomem, nostack, preserves_flags)
            );
        }
        value
    }
}

/// Primary COM1 console used during boot and early kernel initialization.
pub const COM1: SerialPort = SerialPort::new(0x3F8);
