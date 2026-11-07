//! x86-64 SYSCALL/SYSRET MSR configuration.
//!
//! This module owns x86-64 syscall MSR setup and the feature-gated Ring 3
//! lifecycle entry stub. The current assembly entry is a smoke-test adapter:
//! its stack and FX-state slots are global test state, not production per-CPU
//! state. Do not expose production Ring 3 tasks until a safe runtime provides
//! per-CPU entry state, kernel stacks, and a production dispatch path.

#[cfg(feature = "user-task-lifecycle-smoke-test")]
use core::arch::global_asm;
use core::arch::{asm, x86_64::__cpuid};
#[cfg(feature = "user-task-lifecycle-smoke-test")]
use core::mem::{offset_of, size_of};
#[cfg(feature = "user-task-lifecycle-smoke-test")]
use core::ptr;

use super::gdt::{
    KERNEL_CODE_SELECTOR, KERNEL_DATA_SELECTOR, USER_CODE_SELECTOR, USER_DATA_SELECTOR,
};

const IA32_EFER: u32 = 0xC000_0080;
const IA32_STAR: u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;
#[cfg(feature = "user-task-lifecycle-smoke-test")]
const IA32_GS_BASE: u32 = 0xC000_0101;
#[cfg(feature = "user-task-lifecycle-smoke-test")]
const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;
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
    let star =
        ((u64::from(USER_CODE_SELECTOR) - 16) << 48) | (u64::from(KERNEL_CODE_SELECTOR) << 32);

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

/// Boot-CPU syscall entry data used only by the Ring 3 lifecycle smoke test.
///
/// The layout is consumed directly by the entry stub through GS-relative
/// offsets. This is a single-CPU test adapter, not an AP-ready per-CPU allocator.
#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[repr(C, align(64))]
struct SyscallCpuLocal {
    user_rsp: u64,
    kernel_stack_top: u64,
    fx_state: usize,
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut BOOT_CPU_SYSCALL_LOCAL: SyscallCpuLocal = SyscallCpuLocal {
    user_rsp: 0,
    kernel_stack_top: 0,
    fx_state: 0,
};

#[cfg(feature = "user-task-lifecycle-smoke-test")]
const _: [(); 0] = [(); offset_of!(SyscallCpuLocal, user_rsp)];
#[cfg(feature = "user-task-lifecycle-smoke-test")]
const _: [(); 8] = [(); offset_of!(SyscallCpuLocal, kernel_stack_top)];
#[cfg(feature = "user-task-lifecycle-smoke-test")]
const _: [(); 16] = [(); offset_of!(SyscallCpuLocal, fx_state)];
#[cfg(feature = "user-task-lifecycle-smoke-test")]
const _: [(); 24] = [(); size_of::<SyscallCpuLocal>()];

/// Installs the smoke-test boot CPU's kernel GS base before enabling SYSCALL.
///
/// User GS starts at zero in this harness. SYSCALL swaps to the trusted kernel
/// GS base before reading any entry state. Exception handlers must not assume
/// GS is kernel-owned: an NMI can arrive on either side of SWAPGS.
#[cfg(feature = "user-task-lifecycle-smoke-test")]
pub(crate) fn initialize_smoke_test_cpu_local(kernel_stack_top: u64, fx_state: usize) -> bool {
    if !valid_entry_context(kernel_stack_top, fx_state) {
        return false;
    }

    let local = ptr::addr_of_mut!(BOOT_CPU_SYSCALL_LOCAL) as u64;
    if !is_canonical_address(local) || local & 0x3F != 0 {
        return false;
    }

    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(BOOT_CPU_SYSCALL_LOCAL.kernel_stack_top),
            kernel_stack_top,
        );
        ptr::write_volatile(ptr::addr_of_mut!(BOOT_CPU_SYSCALL_LOCAL.fx_state), fx_state);
    }

    let previous_gs = unsafe { read_msr(IA32_GS_BASE) };
    let previous_kernel_gs = unsafe { read_msr(IA32_KERNEL_GS_BASE) };
    unsafe {
        write_msr(IA32_GS_BASE, 0);
        write_msr(IA32_KERNEL_GS_BASE, local);
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

    let valid = unsafe { read_msr(IA32_GS_BASE) == 0 && read_msr(IA32_KERNEL_GS_BASE) == local };
    if !valid {
        unsafe {
            write_msr(IA32_GS_BASE, previous_gs);
            write_msr(IA32_KERNEL_GS_BASE, previous_kernel_gs);
        }
    }
    valid
}

/// Publishes the next task's trusted syscall stack and FX-state pointer.
#[cfg(feature = "user-task-lifecycle-smoke-test")]
pub(crate) fn update_smoke_test_task_context(kernel_stack_top: u64, fx_state: usize) -> bool {
    if !valid_entry_context(kernel_stack_top, fx_state) {
        return false;
    }

    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(BOOT_CPU_SYSCALL_LOCAL.kernel_stack_top),
            kernel_stack_top,
        );
        ptr::write_volatile(
            ptr::addr_of_mut!(BOOT_CPU_SYSCALL_LOCAL.fx_state),
            fx_state,
        );
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    true
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn valid_entry_context(kernel_stack_top: u64, fx_state: usize) -> bool {
    kernel_stack_top != 0
        && kernel_stack_top & 0xF == 0
        && is_canonical_address(kernel_stack_top)
        && fx_state != 0
        && fx_state & 0xF == 0
        && is_canonical_address(fx_state as u64)
}

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
    # SYSCALL leaves RSP on the untrusted user stack. SWAPGS selects the
    # trusted boot-CPU entry context before touching memory or changing stacks.
    swapgs
    mov qword ptr gs:[0], rsp
    mov rsp, qword ptr gs:[8]

    # First lay down the five-word privilege-return tail. The subsequent
    # GPR pushes place r15..rax before this tail, matching InterruptContext.
    push 0x2B
    push qword ptr gs:[0]
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

    mov rax, qword ptr gs:[16]
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
    mov rcx, qword ptr gs:[16]
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
    # Restore the user GS base before IRETQ. The kernel GS base remains in
    # IA32_KERNEL_GS_BASE for the next entry.
    swapgs
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
