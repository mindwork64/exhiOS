use core::arch::asm;

const MASTER_COMMAND: u16 = 0x20;
const MASTER_DATA: u16 = 0x21;
const SLAVE_COMMAND: u16 = 0xA0;
const SLAVE_DATA: u16 = 0xA1;
const PIC_IO_WAIT: u16 = 0x80;

pub const MASTER_VECTOR_OFFSET: u8 = 0x20;
pub const SLAVE_VECTOR_OFFSET: u8 = 0x28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PicStatus {
    pub master_mask: u8,
    pub slave_mask: u8,
}

impl PicStatus {
    pub const fn all_masked(self) -> bool {
        self.master_mask == 0xFF && self.slave_mask == 0xFF
    }
}

/// Remaps the legacy dual 8259 PIC away from CPU exception vectors and masks
/// every IRQ line. The kernel keeps IF clear until matching IDT gates and
/// device handlers have been installed.
///
/// The current QEMU q35 stage uses the PIC only as a compatibility baseline.
/// Local APIC/IOAPIC routing is deferred until ACPI MADT parsing is available.
/// Unmasks only IRQ0 while preserving the current state of every other
/// master PIC line. Slave PIC lines remain untouched.
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    all(
        feature = "timer-preemption-smoke-test",
        not(feature = "ring3-smoke-test")
    )
))]
pub fn unmask_irq0() -> PicStatus {
    // SAFETY: Called during single-core startup with IF clear, after the IRQ0
    // IDT gate is installed. The caller enables CPU interrupts separately.
    unsafe {
        let master = inb(MASTER_DATA) & !0x01;
        outb(MASTER_DATA, master);
        PicStatus {
            master_mask: inb(MASTER_DATA),
            slave_mask: inb(SLAVE_DATA),
        }
    }
}

/// Masks IRQ0 without changing the state of any other PIC line.
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub fn mask_irq0() -> PicStatus {
    // SAFETY: Called with IF clear while changing timer interrupt policy.
    unsafe {
        let master = inb(MASTER_DATA) | 0x01;
        outb(MASTER_DATA, master);
        PicStatus {
            master_mask: inb(MASTER_DATA),
            slave_mask: inb(SLAVE_DATA),
        }
    }
}

/// Unmasks only master PIC IRQ1 while preserving every other line.
#[cfg(feature = "keyboard-smoke-test")]
pub fn unmask_irq1() -> PicStatus {
    // SAFETY: Called with IF clear after the keyboard IDT gate is installed.
    unsafe {
        let master = inb(MASTER_DATA) & !0x02;
        outb(MASTER_DATA, master);
        PicStatus {
            master_mask: inb(MASTER_DATA),
            slave_mask: inb(SLAVE_DATA),
        }
    }
}

/// Masks IRQ1 without changing the state of any other PIC line.
#[cfg(feature = "keyboard-smoke-test")]
pub fn mask_irq1() -> PicStatus {
    // SAFETY: Called with IF clear while changing keyboard interrupt policy.
    unsafe {
        let master = inb(MASTER_DATA) | 0x02;
        outb(MASTER_DATA, master);
        PicStatus {
            master_mask: inb(MASTER_DATA),
            slave_mask: inb(SLAVE_DATA),
        }
    }
}

pub fn initialize_and_mask() -> PicStatus {
    // SAFETY: This code runs in ring 0 during single-CPU kernel startup.
    // Maskable interrupts remain disabled while the PIC is reprogrammed.
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));

        // Start the standard 8259 initialization sequence (ICW1).
        outb(MASTER_COMMAND, 0x11);
        io_wait();
        outb(SLAVE_COMMAND, 0x11);
        io_wait();

        // Move IRQ0-7 to vectors 0x20-0x27 and IRQ8-15 to 0x28-0x2F.
        outb(MASTER_DATA, MASTER_VECTOR_OFFSET);
        io_wait();
        outb(SLAVE_DATA, SLAVE_VECTOR_OFFSET);
        io_wait();

        // Tell the master that a slave PIC is attached to IRQ2.
        outb(MASTER_DATA, 0x04);
        io_wait();
        outb(SLAVE_DATA, 0x02);
        io_wait();

        // Use 8086/88 mode.
        outb(MASTER_DATA, 0x01);
        io_wait();
        outb(SLAVE_DATA, 0x01);
        io_wait();

        // No hardware IRQ has an IDT handler yet. Keep every line masked.
        outb(MASTER_DATA, 0xFF);
        outb(SLAVE_DATA, 0xFF);

        PicStatus {
            master_mask: inb(MASTER_DATA),
            slave_mask: inb(SLAVE_DATA),
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

fn io_wait() {
    // SAFETY: Port 0x80 is the conventional legacy I/O delay port on x86 PCs.
    unsafe { outb(PIC_IO_WAIT, 0) };
}
