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

    fn interrupt_gate(handler: u64, ist_index: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector: KERNEL_CODE_SELECTOR,
            ist: ist_index & 0x7,
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
    pub double_fault_ist: u8,
    pub timer_irq_valid: bool,
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

    // SAFETY: The IDT is initialized once during single-core startup with IF clear.
    let entries = unsafe { ptr::addr_of_mut!(IDT.0).cast::<IdtEntry>() };

    // SAFETY: IDT is a 16-byte-aligned static table in the kernel image.
    // Each handler is a kernel text symbol and each entry is written before
    // LIDT makes the table visible to the processor.
    unsafe {
        for (index, handler) in handlers.iter().copied().enumerate() {
            // Vector 8 (double fault) is the only initial exception gate that
            // uses IST. Its stack is prepared and loaded in the TSS by GDT init.
            let ist_index = if index == 8 { 1 } else { 0 };
            ptr::write_volatile(
                entries.add(index),
                IdtEntry::interrupt_gate(handler, ist_index),
            );
        }
    }

    // Install the first hardware IRQ gate. All other PIC IRQs remain masked
    // until their own handlers and routing are explicitly initialized.
    let timer_irq_handler = super::timer::irq0_stub_address();
    unsafe {
        ptr::write_volatile(
            entries.add(usize::from(super::timer::IRQ0_VECTOR)),
            IdtEntry::interrupt_gate(timer_irq_handler, 0),
        );
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
    // SAFETY: IDT entry 8 was initialized above and remains in writable kernel
    // storage. Read the byte field without creating a reference to a packed field.
    let double_fault_ist = unsafe {
        let gate = entries.add(8);
        ptr::read_volatile(ptr::addr_of!((*gate).ist)) & 0x7
    };
    // Copy the packed entry before reading its fields so no unaligned
    // references to IDT storage are created.
    let timer_irq_gate = unsafe {
        ptr::read_volatile(entries.add(usize::from(super::timer::IRQ0_VECTOR)))
    };
    let timer_irq_gate_address = u64::from(timer_irq_gate.offset_low)
        | (u64::from(timer_irq_gate.offset_middle) << 16)
        | (u64::from(timer_irq_gate.offset_high) << 32);
    let timer_irq_valid = timer_irq_gate_address == timer_irq_handler
        && timer_irq_gate.selector == KERNEL_CODE_SELECTOR
        && timer_irq_gate.ist & 0x7 == 0
        && timer_irq_gate.attributes == INTERRUPT_GATE_ATTRIBUTES;

    IdtStatus {
        base: active_base,
        limit: active_limit,
        double_fault_ist,
        timer_irq_valid,
        valid: active_base == expected_base
            && active_limit == expected_limit
            && double_fault_ist == 1
            && timer_irq_valid,
    }
}

/// Deliberately makes #PF delivery fail so the CPU raises #DF.
///
/// The debug-exit build invokes this only after the GDT/TSS and IDT are live.
/// The faulting address is in the reserved, unmapped upper part of the current
/// lower-canonical kernel window. This function never returns on success.
#[cfg(feature = "qemu-debug-exit-smoke-test")]
unsafe extern "sysv64" {
    fn exhi_trigger_guard_stack_overflow(stack_bottom: u64, fault_address: u64) -> !;
}

/// Forces a real page fault while RSP points at the first byte of the mapped
/// boot stack. CPU exception-frame pushes descend into the unmapped guard page,
/// so #PF delivery faults and the processor must escalate to #DF on IST1.
#[cfg(feature = "qemu-debug-exit-smoke-test")]
pub fn trigger_double_fault_guard_stack_smoke_test(stack_bottom: u64, guard_page: u64) {
    if guard_page.checked_add(4096) != Some(stack_bottom)
        || super::vmm::query_page(guard_page) != Err(super::vmm::VmmError::NotMapped)
        || super::vmm::query_page(stack_bottom).is_err()
    {
        unsafe {
            asm!("ud2", options(nomem, nostack));
        }
    }

    // SAFETY: The test verifies that stack_bottom is mapped and guard_page is
    // the immediately preceding unmapped page. The assembly sets RSP to the
    // bottom of the mapped stack, then reads from guard_page. The resulting
    // #PF cannot push its exception frame below RSP because those writes also
    // hit guard_page, so delivery escalates to #DF through IST1.
    unsafe {
        exhi_trigger_guard_stack_overflow(stack_bottom, guard_page);
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
        mov r9,  r12

        # An exception may arrive at any instruction, so do not assume the
        # interrupted RSP has ABI alignment. Align the new call stack explicitly.
        and rsp, -16
        call exhi_exception_handler
        ud2
    "#
);


#[cfg(feature = "qemu-debug-exit-smoke-test")]
global_asm!(
    r#"
    .section .text
    .global exhi_trigger_guard_stack_overflow
    .type exhi_trigger_guard_stack_overflow, @function
exhi_trigger_guard_stack_overflow:
    cli
    cld
    mov rsp, rdi
    mov rax, [rsi]
    ud2
"#
);
