//! x86-64 SYSCALL/SYSRET MSR configuration.
//!
//! This module only installs the processor's syscall MSR contract. It does
//! not install an entry stack or enable user tasks; callers must provide a
//! valid kernel entry point and must not expose Ring 3 until the runtime is
//! ready to handle it.

use core::arch::{asm, x86_64::__cpuid};
#[cfg(feature = "user-task-lifecycle-smoke-test")]
use core::arch::global_asm;

use super::gdt::{
    KERNEL_CODE_SELECTOR, KERNEL_DATA_SELECTOR, USER_CODE_SELECTOR, USER_DATA_SELECTOR,
};

const IA32_EFER: u32 = 0xC000_0080;
const IA32_STAR: u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;
const EFER_SCE: u64 = 1;
const FMASK_BITS: u64 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 18);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallInitError {
    InvalidEntryPoint,
    UnsupportedCpu,
    InvalidSegmentSelectors,
    ReadbackMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyscallMsrState {
    pub efer: u64,
    pub star: u64,
    pub lstar: u64,
    pub fmask: u64,
}

/// Configures SYSCALL/SYSRET selectors, entry point and interrupt masking.
///
/// EFER.SCE is enabled last, after the other MSRs have been written. The
/// function deliberately does not configure a kernel stack: the entry stub
/// must switch away from the untrusted user RSP before using the stack.
pub fn initialize(entry: u64) -> Result<SyscallMsrState, SyscallInitError> {
    if entry == 0 || !is_canonical_address(entry) {
        return Err(SyscallInitError::InvalidEntryPoint);
    }
    if !segment_selectors_are_valid() {
        return Err(SyscallInitError::InvalidSegmentSelectors);
    }

    let max_extended = __cpuid(0x8000_0000);
    if max_extended.eax < 0x8000_0001 {
        return Err(SyscallInitError::UnsupportedCpu);
    }
    let features = __cpuid(0x8000_0001);
    if features.edx & (1 << 11) == 0 {
        return Err(SyscallInitError::UnsupportedCpu);
    }

    let previous_efer = unsafe { read_msr(IA32_EFER) };
    let star = ((u64::from(USER_CODE_SELECTOR) - 16) << 48)
        | (u64::from(KERNEL_CODE_SELECTOR) << 32);

    // Program the inert descriptor/entry state first. SCE remains unchanged
    // until all three values are present and have been checked.
    unsafe {
        write_msr(IA32_STAR, star);
        write_msr(IA32_LSTAR, entry);
        write_msr(IA32_FMASK, FMASK_BITS);
        write_msr(IA32_EFER, previous_efer | EFER_SCE);
    }

    let state = unsafe {
        SyscallMsrState {
            efer: read_msr(IA32_EFER),
            star: read_msr(IA32_STAR),
            lstar: read_msr(IA32_LSTAR),
            fmask: read_msr(IA32_FMASK),
        }
    };

    if state.efer & EFER_SCE == 0
        || state.star != star
        || state.lstar != entry
        || state.fmask != FMASK_BITS
    {
        // Do not leave SYSCALL enabled if the complete contract did not
        // survive read-back verification.
        unsafe {
            write_msr(IA32_EFER, previous_efer);
        }
        return Err(SyscallInitError::ReadbackMismatch);
    }

    Ok(state)
}

const fn segment_selectors_are_valid() -> bool {
    KERNEL_CODE_SELECTOR & 3 == 0
        && KERNEL_DATA_SELECTOR == KERNEL_CODE_SELECTOR + 8
        && USER_CODE_SELECTOR & 3 == 3
        && USER_DATA_SELECTOR & 3 == 3
        && USER_DATA_SELECTOR == USER_CODE_SELECTOR - 8
}

const fn is_canonical_address(address: u64) -> bool {
    address >> 47 == 0 || address >> 47 == 0x1_FFFF
}

unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nostack, preserves_flags)
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

unsafe fn write_msr(msr: u32, value: u64) {
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nostack, preserves_flags)
        );
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[no_mangle]
pub(crate) static mut USER_TASK_SYSCALL_USER_RSP: u64 = 0;
#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[no_mangle]
pub(crate) static mut USER_TASK_SYSCALL_KERNEL_STACK_TOP: u64 = 0;
#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[no_mangle]
pub(crate) static mut USER_TASK_SYSCALL_FX_STATE: usize = 0;

#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[repr(C, align(16))]
pub(crate) struct UserTaskSyscallFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}
#[cfg(feature = "user-task-lifecycle-smoke-test")]
const _: [(); 20 * core::mem::size_of::<u64>()] =
    [(); core::mem::size_of::<UserTaskSyscallFrame>()];


#[cfg(feature = "user-task-lifecycle-smoke-test")]
global_asm!(
    r#"
    .section .text
    .global exhi_user_task_syscall_entry
    .type exhi_user_task_syscall_entry, @function
exhi_user_task_syscall_entry:
    # SYSCALL leaves RSP on the untrusted user stack. Save it before moving
    # to the active task's dedicated kernel stack.
    mov qword ptr [rip + USER_TASK_SYSCALL_USER_RSP], rsp
    mov rsp, qword ptr [rip + USER_TASK_SYSCALL_KERNEL_STACK_TOP]

    # First lay down the five-word privilege-return tail. The subsequent
    # GPR pushes place r15..rax before this tail, matching InterruptContext.
    push 0x2B
    push qword ptr [rip + USER_TASK_SYSCALL_USER_RSP]
    push r11
    push 0x33
    push rcx

    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    mov rax, qword ptr [rip + USER_TASK_SYSCALL_FX_STATE]
    test rax, rax
    jz .Luser_task_syscall_fatal
    fxsave64 [rax]

    mov rdi, rsp
    and rsp, -16
    cld
    call exhi_user_task_syscall_handler
    test rax, rax
    jz .Luser_task_syscall_fatal

    # The scheduler handler selected the next task and updated CR3, TSS.RSP0,
    # kernel-stack top and FX-state pointer before returning its saved frame.
    mov rcx, qword ptr [rip + USER_TASK_SYSCALL_FX_STATE]
    fxrstor64 [rcx]
    mov rsp, rax

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    iretq

.Luser_task_syscall_fatal:
    mov dx, 0xF4
    mov eax, 0x11
    out dx, eax
    cli
.Luser_task_syscall_halt:
    hlt
    jmp .Luser_task_syscall_halt
    "#
);
