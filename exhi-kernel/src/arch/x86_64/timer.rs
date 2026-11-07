//! Legacy PIT channel 0 polling-free tick source.
//!
//! The timer generates IRQ0 at approximately 100 Hz. The assembly ISR only
//! performs an atomic tick increment and acknowledges the master 8259 PIC;
//! it does not call Rust, touch the heap, or modify general registers.
//!
//! Startup code must install IDT vector 0x20, keep all other PIC lines masked,
//! program the PIT with interrupts disabled, and only then execute STI.

use core::arch::global_asm;
use core::sync::atomic::AtomicU64;
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
use core::sync::atomic::Ordering;

use exhi_driver::{DriverClass, DriverDescriptor, DriverId};

pub const DRIVER: DriverDescriptor =
    DriverDescriptor::new(DriverId::PitChannel0, DriverClass::Clock, "PIT channel 0");

use super::pic;

#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub const TICK_HZ: u32 = 100;
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub const PIT_INPUT_HZ: u32 = 1_193_182;
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub const PIT_DIVISOR: u16 = ((PIT_INPUT_HZ + (TICK_HZ / 2)) / TICK_HZ) as u16;
pub const IRQ0_VECTOR: u8 = pic::MASTER_VECTOR_OFFSET;

/// Monotonic tick counter. The IRQ stub increments this with a locked
/// instruction; readers use relaxed ordering because the counter publishes
/// no other memory.
#[no_mangle]
pub static EXHIOS_PIT_TICKS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub struct TimerStatus {
    pub requested_hz: u32,
    pub pit_divisor: u16,
    pub irq0_unmasked: bool,
    pub master_mask: u8,
    pub slave_mask: u8,
}

/// Programs PIT channel 0 in mode 2 and unmasks only PIC IRQ0.
///
/// Call this with IF clear after IDT vector 0x20 has been installed. This
/// function does not execute STI; interrupt policy remains with the caller.
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub fn initialize() -> TimerStatus {
    EXHIOS_PIT_TICKS.store(0, Ordering::Relaxed);

    // Channel 0, low/high byte access, mode 2 rate generator, binary count.
    unsafe {
        outb(0x43, 0x34);
        outb(0x40, PIT_DIVISOR as u8);
        outb(0x40, (PIT_DIVISOR >> 8) as u8);
    }

    let pic = pic::unmask_irq0();
    TimerStatus {
        requested_hz: TICK_HZ,
        pit_divisor: PIT_DIVISOR,
        irq0_unmasked: pic.master_mask & 0x01 == 0 && pic.slave_mask == 0xFF,
        master_mask: pic.master_mask,
        slave_mask: pic.slave_mask,
    }
}

/// Returns the monotonic number of PIT ticks since timer initialization.
#[inline]
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub fn ticks() -> u64 {
    EXHIOS_PIT_TICKS.load(Ordering::Relaxed)
}

/// Returns elapsed ticks with wrapping subtraction, valid across counter wrap.
#[inline]
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub fn elapsed_ticks(start: u64) -> u64 {
    ticks().wrapping_sub(start)
}

/// Masks IRQ0 again. The caller should execute CLI before calling this when
/// it is shutting down a timer test or changing interrupt policy.
#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
pub fn mask_irq0() -> pic::PicStatus {
    pic::mask_irq0()
}

#[cfg(not(feature = "timer-preemption-smoke-test"))]
pub fn irq0_stub_address() -> u64 {
    exhi_irq0_stub as *const () as usize as u64
}

#[cfg(not(feature = "timer-preemption-smoke-test"))]
unsafe extern "sysv64" {
    fn exhi_irq0_stub();
}

#[cfg(any(
    feature = "timer-smoke-test",
    feature = "task-accounting-smoke-test",
    feature = "timer-preemption-smoke-test"
))]
unsafe fn outb(port: u16, value: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

global_asm!(
    r#"
    .section .text
    .global exhi_irq0_stub
    .type exhi_irq0_stub, @function
exhi_irq0_stub:
    push rax
    lock inc qword ptr [rip + EXHIOS_PIT_TICKS]
    mov al, 0x20
    out 0x20, al
    pop rax
    iretq
"#
);
