use core::arch::{asm, global_asm};
use core::hint::spin_loop;
use core::ptr;

use exhi_driver::{DriverClass, DriverDescriptor, DriverId};
use exhi_input::{KeyEvent, KeyboardDecoder, EXHIOS_KEYBOARD_SCANCODE_QUEUE};

pub const DRIVER: DriverDescriptor = DriverDescriptor::new(
    DriverId::Ps2Keyboard,
    DriverClass::Input,
    "i8042 PS/2 keyboard",
);

use super::pic;

const DATA_PORT: u16 = 0x60;
const STATUS_COMMAND_PORT: u16 = 0x64;
const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const WAIT_LIMIT: usize = 200_000;
const ENABLE_SCANNING: u8 = 0xF4;
const ACK: u8 = 0xFA;
const RESEND: u8 = 0xFE;

pub const IRQ1_VECTOR: u8 = pic::MASTER_VECTOR_OFFSET + 1;

static mut DECODER: KeyboardDecoder = KeyboardDecoder::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardError {
    InputBufferTimeout,
    OutputBufferTimeout,
    UnexpectedDeviceResponse(u8),
    ResendLimit,
    IrqUnmaskFailed,
}

impl KeyboardError {
    pub const fn message(self) -> &'static [u8] {
        match self {
            Self::InputBufferTimeout => b"i8042 input buffer did not become empty",
            Self::OutputBufferTimeout => b"keyboard did not respond",
            Self::UnexpectedDeviceResponse(_) => b"keyboard returned an unexpected response",
            Self::ResendLimit => b"keyboard resend limit reached",
            Self::IrqUnmaskFailed => b"PIC IRQ1 could not be unmasked",
        }
    }

    pub const fn response(self) -> Option<u8> {
        match self {
            Self::UnexpectedDeviceResponse(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardStatus {
    pub controller_config: u8,
    pub translated_set1: bool,
    pub irq1_unmasked: bool,
    pub master_mask: u8,
    pub slave_mask: u8,
}

impl KeyboardStatus {
    pub const fn is_valid(self) -> bool {
        self.translated_set1 && self.irq1_unmasked && self.master_mask & 0x02 == 0
    }
}

/// Initializes the legacy i8042 keyboard port and enables only PIC IRQ1.
/// It executes CLI and returns with maskable interrupts disabled; the caller
/// owns the later STI after all required interrupt handlers are installed.
pub fn initialize() -> Result<KeyboardStatus, KeyboardError> {
    unsafe {
        asm!("cli", options(nomem, nostack));
    }
    let _ = pic::mask_irq1();
    EXHIOS_KEYBOARD_SCANCODE_QUEUE.clear();
    // SAFETY: Only the foreground consumer accesses this decoder. IRQ1 writes
    // raw scancode bytes to the atomic queue and never touches decoder state.
    unsafe {
        ptr::write(ptr::addr_of_mut!(DECODER), KeyboardDecoder::new());
    }

    controller_command(0xAD)?; // Disable keyboard port.
    controller_command(0xA7)?; // Disable auxiliary (mouse) port.
    flush_output_buffer();

    controller_command(0x20)?; // Read controller command byte.
    wait_output_full()?;
    let original_config = unsafe { inb(DATA_PORT) };

    // IRQ1 on, IRQ12 off, keyboard clock enabled, auxiliary clock disabled,
    // and Set 2 translated to Set 1 for a stable scan-code decoder.
    let config = (original_config | 0x41 | 0x20) & !0x12;
    controller_command(0x60)?; // Write controller command byte.
    write_data(config)?;
    controller_command(0xAE)?; // Re-enable keyboard port.
    send_keyboard_command(ENABLE_SCANNING)?;

    let pic_status = pic::unmask_irq1();
    let status = KeyboardStatus {
        controller_config: config,
        translated_set1: config & (1 << 6) != 0,
        irq1_unmasked: pic_status.master_mask & 0x02 == 0,
        master_mask: pic_status.master_mask,
        slave_mask: pic_status.slave_mask,
    };
    if status.is_valid() {
        Ok(status)
    } else {
        let _ = pic::mask_irq1();
        Err(KeyboardError::IrqUnmaskFailed)
    }
}

/// Returns the next decoded physical key transition from the foreground path.
pub fn try_read_event() -> Option<KeyEvent> {
    while let Some(scancode) = EXHIOS_KEYBOARD_SCANCODE_QUEUE.try_pop() {
        // SAFETY: This is the single foreground consumer. The IRQ handler only
        // touches the separate atomic queue, so the decoder cannot be aliased.
        let decoder = unsafe { &mut *ptr::addr_of_mut!(DECODER) };
        if let Some(event) = decoder.feed(scancode) {
            return Some(event);
        }
    }
    None
}

pub fn dropped_scancodes() -> u64 {
    EXHIOS_KEYBOARD_SCANCODE_QUEUE.dropped_count()
}

pub fn mask_irq1() -> pic::PicStatus {
    pic::mask_irq1()
}

pub fn irq1_stub_address() -> u64 {
    exhi_irq1_stub as *const () as usize as u64
}

unsafe extern "sysv64" {
    fn exhi_irq1_stub();
}

fn controller_command(value: u8) -> Result<(), KeyboardError> {
    wait_input_empty()?;
    unsafe { outb(STATUS_COMMAND_PORT, value) };
    Ok(())
}

fn write_data(value: u8) -> Result<(), KeyboardError> {
    wait_input_empty()?;
    unsafe { outb(DATA_PORT, value) };
    Ok(())
}

fn read_data() -> Result<u8, KeyboardError> {
    wait_output_full()?;
    Ok(unsafe { inb(DATA_PORT) })
}

fn send_keyboard_command(command: u8) -> Result<(), KeyboardError> {
    for _ in 0..3 {
        write_data(command)?;
        match read_data()? {
            ACK => return Ok(()),
            RESEND => continue,
            response => return Err(KeyboardError::UnexpectedDeviceResponse(response)),
        }
    }
    Err(KeyboardError::ResendLimit)
}

fn flush_output_buffer() {
    for _ in 0..32 {
        if unsafe { inb(STATUS_COMMAND_PORT) } & STATUS_OUTPUT_FULL == 0 {
            break;
        }
        let _ = unsafe { inb(DATA_PORT) };
    }
}

fn wait_input_empty() -> Result<(), KeyboardError> {
    for _ in 0..WAIT_LIMIT {
        if unsafe { inb(STATUS_COMMAND_PORT) } & STATUS_INPUT_FULL == 0 {
            return Ok(());
        }
        spin_loop();
    }
    Err(KeyboardError::InputBufferTimeout)
}

fn wait_output_full() -> Result<(), KeyboardError> {
    for _ in 0..WAIT_LIMIT {
        if unsafe { inb(STATUS_COMMAND_PORT) } & STATUS_OUTPUT_FULL != 0 {
            return Ok(());
        }
        spin_loop();
    }
    Err(KeyboardError::OutputBufferTimeout)
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

global_asm!(
    r#"
    .section .text
    .global exhi_irq1_stub
    .type exhi_irq1_stub, @function
exhi_irq1_stub:
    push rax
    push rcx
    push rdx
    push r9

    in al, 0x60
    mov r9b, al

    mov rax, qword ptr [rip + EXHIOS_KEYBOARD_SCANCODE_QUEUE]
    lea rcx, [rax + 1]
    and ecx, 63
    mov rdx, qword ptr [rip + EXHIOS_KEYBOARD_SCANCODE_QUEUE + 8]
    cmp rcx, rdx
    je 1f

    lea rdx, [rip + EXHIOS_KEYBOARD_SCANCODE_QUEUE + 24]
    mov byte ptr [rdx + rax], r9b
    mov qword ptr [rip + EXHIOS_KEYBOARD_SCANCODE_QUEUE], rcx
    jmp 2f

1:
    lock inc qword ptr [rip + EXHIOS_KEYBOARD_SCANCODE_QUEUE + 16]

2:
    mov al, 0x20
    out 0x20, al

    pop r9
    pop rdx
    pop rcx
    pop rax
    iretq
    .size exhi_irq1_stub, .-exhi_irq1_stub
"#
);
