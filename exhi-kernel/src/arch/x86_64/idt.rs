use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::ptr;

use super::gdt::KERNEL_CODE_SELECTOR;

const IDT_ENTRY_COUNT: usize = 256;
const EXCEPTION_COUNT: usize = 32;
const INTERRUPT_GATE_ATTRIBUTES: u8 = 0x8E;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    fn interrupt_gate(handler: u64) -> Self {
        Self {
            offset_low: handler as u16,
            selector: KERNEL_CODE_SELECTOR,
            ist: 0,
            attributes: INTERRUPT_GATE_ATTRIBUTES,
            offset_middle: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, align(16))]
struct InterruptDescriptorTable([IdtEntry; IDT_ENTRY_COUNT]);

static mut IDT: InterruptDescriptorTable =
    InterruptDescriptorTable([IdtEntry::MISSING; IDT_ENTRY_COUNT]);

#[repr(C, packed)]
struct IdtRegister {
    limit: u16,
    base: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdtStatus {
    pub base: u64,
    pub limit: u16,
    pub valid: bool,
}

/// Installs gates for all architecturally defined x86 exceptions (vectors 0–31)
/// and verifies the processor's IDTR with SIDT.
pub fn initialize() -> IdtStatus {
    let handlers: [u64; EXCEPTION_COUNT] = [
        exhi_isr_0 as *const () as usize as u64,
        exhi_isr_1 as *const () as usize as u64,
        exhi_isr_2 as *const () as usize as u64,
        exhi_isr_3 as *const () as usize as u64,
        exhi_isr_4 as *const () as usize as u64,
        exhi_isr_5 as *const () as usize as u64,
        exhi_isr_6 as *const () as usize as u64,
        exhi_isr_7 as *const () as usize as u64,
        exhi_isr_8 as *const () as usize as u64,
        exhi_isr_9 as *const () as usize as u64,
        exhi_isr_10 as *const () as usize as u64,
        exhi_isr_11 as *const () as usize as u64,
        exhi_isr_12 as *const () as usize as u64,
        exhi_isr_13 as *const () as usize as u64,
        exhi_isr_14 as *const () as usize as u64,
        exhi_isr_15 as *const () as usize as u64,
        exhi_isr_16 as *const () as usize as u64,
        exhi_isr_17 as *const () as usize as u64,
        exhi_isr_18 as *const () as usize as u64,
        exhi_isr_19 as *const () as usize as u64,
        exhi_isr_20 as *const () as usize as u64,
        exhi_isr_21 as *const () as usize as u64,
        exhi_isr_22 as *const () as usize as u64,
        exhi_isr_23 as *const () as usize as u64,
        exhi_isr_24 as *const () as usize as u64,
        exhi_isr_25 as *const () as usize as u64,
        exhi_isr_26 as *const () as usize as u64,
        exhi_isr_27 as *const () as usize as u64,
        exhi_isr_28 as *const () as usize as u64,
        exhi_isr_29 as *const () as usize as u64,
        exhi_isr_30 as *const () as usize as u64,
        exhi_isr_31 as *const () as usize as u64,
    ];

    // SAFETY: IDT is a 16-byte-aligned static table in the kernel image.
    // Each handler is a kernel text symbol and each entry is written before
    // LIDT makes the table visible to the processor.
    unsafe {
        let entries = ptr::addr_of_mut!(IDT.0).cast::<IdtEntry>();
        for (index, handler) in handlers.iter().copied().enumerate() {
            ptr::write_volatile(entries.add(index), IdtEntry::interrupt_gate(handler));
        }
    }

    let expected_base = ptr::addr_of!(IDT) as u64;
    let expected_limit = (size_of::<InterruptDescriptorTable>() - 1) as u16;
    let descriptor = IdtRegister {
        limit: expected_limit,
        base: expected_base,
    };

    // SAFETY: The descriptor points to the fully initialized static IDT and
    // is valid for the duration of the LIDT instruction.
    unsafe {
        asm!(
            "lidt [{}]",
            in(reg) &descriptor as *const IdtRegister,
            options(readonly, nostack, preserves_flags)
        );
    }

    let mut active = IdtRegister { limit: 0, base: 0 };

    // SAFETY: SIDT writes the ten-byte IDTR value into this local buffer.
    unsafe {
        asm!(
            "sidt [{}]",
            in(reg) &mut active as *mut IdtRegister,
            options(nostack, preserves_flags)
        );
    }

    let active_base = active.base;
    let active_limit = active.limit;

    IdtStatus {
        base: active_base,
        limit: active_limit,
        valid: active_base == expected_base && active_limit == expected_limit,
    }
}

unsafe extern "sysv64" {
        fn exhi_isr_0();
        fn exhi_isr_1();
        fn exhi_isr_2();
        fn exhi_isr_3();
        fn exhi_isr_4();
        fn exhi_isr_5();
        fn exhi_isr_6();
        fn exhi_isr_7();
        fn exhi_isr_8();
        fn exhi_isr_9();
        fn exhi_isr_10();
        fn exhi_isr_11();
        fn exhi_isr_12();
        fn exhi_isr_13();
        fn exhi_isr_14();
        fn exhi_isr_15();
        fn exhi_isr_16();
        fn exhi_isr_17();
        fn exhi_isr_18();
        fn exhi_isr_19();
        fn exhi_isr_20();
        fn exhi_isr_21();
        fn exhi_isr_22();
        fn exhi_isr_23();
        fn exhi_isr_24();
        fn exhi_isr_25();
        fn exhi_isr_26();
        fn exhi_isr_27();
        fn exhi_isr_28();
        fn exhi_isr_29();
        fn exhi_isr_30();
        fn exhi_isr_31();
}

global_asm!(
    r#"
    .section .text
    .global exhi_isr_0
    .type exhi_isr_0, @function
exhi_isr_0:
        push 0
        push 0
        jmp exhi_exception_common

    .global exhi_isr_1
    .type exhi_isr_1, @function
exhi_isr_1:
        push 0
        push 1
        jmp exhi_exception_common

    .global exhi_isr_2
    .type exhi_isr_2, @function
exhi_isr_2:
        push 0
        push 2
        jmp exhi_exception_common

    .global exhi_isr_3
    .type exhi_isr_3, @function
exhi_isr_3:
        push 0
        push 3
        jmp exhi_exception_common

    .global exhi_isr_4
    .type exhi_isr_4, @function
exhi_isr_4:
        push 0
        push 4
        jmp exhi_exception_common

    .global exhi_isr_5
    .type exhi_isr_5, @function
exhi_isr_5:
        push 0
        push 5
        jmp exhi_exception_common

    .global exhi_isr_6
    .type exhi_isr_6, @function
exhi_isr_6:
        push 0
        push 6
        jmp exhi_exception_common

    .global exhi_isr_7
    .type exhi_isr_7, @function
exhi_isr_7:
        push 0
        push 7
        jmp exhi_exception_common

    .global exhi_isr_8
    .type exhi_isr_8, @function
exhi_isr_8:
        push 8
        jmp exhi_exception_common

    .global exhi_isr_9
    .type exhi_isr_9, @function
exhi_isr_9:
        push 0
        push 9
        jmp exhi_exception_common

    .global exhi_isr_10
    .type exhi_isr_10, @function
exhi_isr_10:
        push 10
        jmp exhi_exception_common

    .global exhi_isr_11
    .type exhi_isr_11, @function
exhi_isr_11:
        push 11
        jmp exhi_exception_common

    .global exhi_isr_12
    .type exhi_isr_12, @function
exhi_isr_12:
        push 12
        jmp exhi_exception_common

    .global exhi_isr_13
    .type exhi_isr_13, @function
exhi_isr_13:
        push 13
        jmp exhi_exception_common

    .global exhi_isr_14
    .type exhi_isr_14, @function
exhi_isr_14:
        push 14
        jmp exhi_exception_common

    .global exhi_isr_15
    .type exhi_isr_15, @function
exhi_isr_15:
        push 0
        push 15
        jmp exhi_exception_common

    .global exhi_isr_16
    .type exhi_isr_16, @function
exhi_isr_16:
        push 0
        push 16
        jmp exhi_exception_common

    .global exhi_isr_17
    .type exhi_isr_17, @function
exhi_isr_17:
        push 17
        jmp exhi_exception_common

    .global exhi_isr_18
    .type exhi_isr_18, @function
exhi_isr_18:
        push 0
        push 18
        jmp exhi_exception_common

    .global exhi_isr_19
    .type exhi_isr_19, @function
exhi_isr_19:
        push 0
        push 19
        jmp exhi_exception_common

    .global exhi_isr_20
    .type exhi_isr_20, @function
exhi_isr_20:
        push 0
        push 20
        jmp exhi_exception_common

    .global exhi_isr_21
    .type exhi_isr_21, @function
exhi_isr_21:
        push 21
        jmp exhi_exception_common

    .global exhi_isr_22
    .type exhi_isr_22, @function
exhi_isr_22:
        push 0
        push 22
        jmp exhi_exception_common

    .global exhi_isr_23
    .type exhi_isr_23, @function
exhi_isr_23:
        push 0
        push 23
        jmp exhi_exception_common

    .global exhi_isr_24
    .type exhi_isr_24, @function
exhi_isr_24:
        push 0
        push 24
        jmp exhi_exception_common

    .global exhi_isr_25
    .type exhi_isr_25, @function
exhi_isr_25:
        push 0
        push 25
        jmp exhi_exception_common

    .global exhi_isr_26
    .type exhi_isr_26, @function
exhi_isr_26:
        push 0
        push 26
        jmp exhi_exception_common

    .global exhi_isr_27
    .type exhi_isr_27, @function
exhi_isr_27:
        push 0
        push 27
        jmp exhi_exception_common

    .global exhi_isr_28
    .type exhi_isr_28, @function
exhi_isr_28:
        push 0
        push 28
        jmp exhi_exception_common

    .global exhi_isr_29
    .type exhi_isr_29, @function
exhi_isr_29:
        push 29
        jmp exhi_exception_common

    .global exhi_isr_30
    .type exhi_isr_30, @function
exhi_isr_30:
        push 30
        jmp exhi_exception_common

    .global exhi_isr_31
    .type exhi_isr_31, @function
exhi_isr_31:
        push 0
        push 31
        jmp exhi_exception_common

    .global exhi_exception_common
    .type exhi_exception_common, @function
exhi_exception_common:
        # The stubs normalize the stack to: vector, error code, RIP, CS, RFLAGS.
        # This path is fatal, so the interrupted register state need not be restored.
        mov r12, rsp
        mov rdi, [r12]
        mov rsi, [r12 + 8]
        mov rdx, [r12 + 16]
        mov rcx, [r12 + 24]
        mov r8,  [r12 + 32]

        # An exception may arrive at any instruction, so do not assume the
        # interrupted RSP has ABI alignment. Align the new call stack explicitly.
        and rsp, -16
        call exhi_exception_handler
        ud2
    "#
);
