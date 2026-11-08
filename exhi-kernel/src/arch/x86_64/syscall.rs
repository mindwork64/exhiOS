//! x86-64 SYSCALL/SYSRET MSR configuration.
//!
//! This module owns x86-64 syscall MSR setup and the feature-gated Ring 3
//! lifecycle entry stub. The current assembly entry is a smoke-test adapter:
//! it uses a GS-backed boot-CPU context, but the storage is still a single-CPU
//! harness rather than an AP-ready per-CPU allocator. Exception/NMI paths must
//! account for the SWAPGS transition window. Do not expose production Ring 3
//! tasks until CPU-local state, kernel stacks, and production dispatch are
//! initialized for every online CPU.

use core::arch::{asm, global_asm, x86_64::__cpuid};
use core::mem::{offset_of, size_of};
use core::ptr;

use super::gdt::{
    KERNEL_CODE_SELECTOR, KERNEL_DATA_SELECTOR, USER_CODE_SELECTOR, USER_DATA_SELECTOR,
};

const IA32_EFER: u32 = 0xC000_0080;
const IA32_STAR: u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;
const IA32_GS_BASE: u32 = 0xC000_0101;
const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;
const EFER_SCE: u64 = 1;
const FMASK_BITS: u64 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 18);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallInitError {
    InvalidEntryPoint,
    UnsupportedCpu,
    InvalidSegmentSelectors,
    ReadbackMismatch,
    CpuLocalNotInitialized,
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
    if !boot_cpu_context_is_ready() {
        return Err(SyscallInitError::CpuLocalNotInitialized);
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

/// Boot-CPU syscall entry data for the current single-CPU kernel.
///
/// The structure is 64-byte aligned and accessed by fixed GS-relative offsets.
/// An AP must receive its own instance before SYSCALL is enabled on that CPU.
#[repr(C, align(64))]
struct SyscallCpuLocal {
    user_rsp: u64,
    kernel_stack_top: u64,
    fx_state: usize,
    dispatcher: usize,
}

static mut BOOT_CPU_SYSCALL_LOCAL: SyscallCpuLocal = SyscallCpuLocal {
    user_rsp: 0,
    kernel_stack_top: 0,
    fx_state: 0,
    dispatcher: 0,
};

const _: [(); 0] = [(); offset_of!(SyscallCpuLocal, user_rsp)];
const _: [(); 8] = [(); offset_of!(SyscallCpuLocal, kernel_stack_top)];
const _: [(); 16] = [(); offset_of!(SyscallCpuLocal, fx_state)];
const _: [(); 24] = [(); offset_of!(SyscallCpuLocal, dispatcher)];
const _: [(); 64] = [(); size_of::<SyscallCpuLocal>()];

/// Stable register image and IRETQ privilege-return tail.
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
const _: [(); 20 * size_of::<u64>()] = [(); size_of::<UserTaskSyscallFrame>()];

/// Installs the boot CPU context and trusted dispatcher before enabling SCE.
/// User GS is zero until the TLS ABI is implemented. This BSP-only state must
/// be replaced by an AP-local allocation before bringing up additional CPUs.
pub fn initialize_cpu_local(kernel_stack_top: u64, fx_state: usize, dispatcher: usize) -> bool {
    if !valid_entry_context(kernel_stack_top, fx_state)
        || dispatcher == 0
        || !is_canonical_address(dispatcher as u64)
    {
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
        ptr::write_volatile(
            ptr::addr_of_mut!(BOOT_CPU_SYSCALL_LOCAL.dispatcher),
            dispatcher,
        );
    }
    let old_gs = unsafe { read_msr(IA32_GS_BASE) };
    let old_kernel_gs = unsafe { read_msr(IA32_KERNEL_GS_BASE) };
    unsafe {
        write_msr(IA32_GS_BASE, 0);
        write_msr(IA32_KERNEL_GS_BASE, local);
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    let valid = unsafe {
        read_msr(IA32_GS_BASE) == 0
            && read_msr(IA32_KERNEL_GS_BASE) == local
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.kernel_stack_top))
                == kernel_stack_top
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.fx_state)) == fx_state
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.dispatcher)) == dispatcher
    };
    if !valid {
        unsafe {
            write_msr(IA32_GS_BASE, old_gs);
            write_msr(IA32_KERNEL_GS_BASE, old_kernel_gs);
        }
    }
    valid
}

/// Updates trusted entry state before returning to another scheduled task.
pub fn update_current_task_context(kernel_stack_top: u64, fx_state: usize) -> bool {
    // This function is called from the syscall dispatcher after the entry stub
    // has executed SWAPGS, so the trusted per-CPU base is IA32_GS_BASE here.
    if !valid_entry_context(kernel_stack_top, fx_state) || !boot_cpu_context_is_active() {
        return false;
    }
    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(BOOT_CPU_SYSCALL_LOCAL.kernel_stack_top),
            kernel_stack_top,
        );
        ptr::write_volatile(ptr::addr_of_mut!(BOOT_CPU_SYSCALL_LOCAL.fx_state), fx_state);
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    true
}

fn valid_entry_context(kernel_stack_top: u64, fx_state: usize) -> bool {
    kernel_stack_top != 0
        && kernel_stack_top & 0xF == 0
        && is_canonical_address(kernel_stack_top)
        && fx_state != 0
        && fx_state & 0xF == 0
        && is_canonical_address(fx_state as u64)
}

fn boot_cpu_context_is_ready() -> bool {
    let local = ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL) as u64;
    unsafe {
        is_canonical_address(local)
            && local & 0x3F == 0
            && read_msr(IA32_GS_BASE) == 0
            && read_msr(IA32_KERNEL_GS_BASE) == local
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.kernel_stack_top)) != 0
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.fx_state)) != 0
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.dispatcher)) != 0
    }
}

/// True only while the syscall dispatcher is executing after SWAPGS.
fn boot_cpu_context_is_active() -> bool {
    let local = ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL) as u64;
    unsafe {
        is_canonical_address(local)
            && local & 0x3F == 0
            && read_msr(IA32_GS_BASE) == local
            && read_msr(IA32_KERNEL_GS_BASE) == 0
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.kernel_stack_top)) != 0
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.fx_state)) != 0
            && ptr::read_volatile(ptr::addr_of!(BOOT_CPU_SYSCALL_LOCAL.dispatcher)) != 0
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
pub(crate) fn initialize_smoke_test_cpu_local(kernel_stack_top: u64, fx_state: usize) -> bool {
    unsafe extern "sysv64" {
        fn exhi_user_task_syscall_handler(frame: *mut UserTaskSyscallFrame) -> usize;
    }
    initialize_cpu_local(
        kernel_stack_top,
        fx_state,
        exhi_user_task_syscall_handler as *const () as usize,
    )
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
pub(crate) fn update_smoke_test_task_context(kernel_stack_top: u64, fx_state: usize) -> bool {
    update_current_task_context(kernel_stack_top, fx_state)
}

/// Address to install in IA32_LSTAR.
pub fn entry_address() -> u64 {
    exhi_syscall_entry as *const () as u64
}

unsafe extern "sysv64" {
    fn exhi_syscall_entry();
}

global_asm!(
    r#"
    .section .text
    .global exhi_syscall_entry
    .type exhi_syscall_entry, @function
exhi_syscall_entry:
    swapgs
    mov qword ptr gs:[0], rsp
    mov rsp, qword ptr gs:[8]
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
    jz .Lexhi_syscall_fatal
    fxsave64 [rax]
    mov rdi, rsp
    and rsp, -16
    cld
    mov rax, qword ptr gs:[24]
    test rax, rax
    jz .Lexhi_syscall_fatal
    call rax
    test rax, rax
    jz .Lexhi_syscall_fatal
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
    swapgs
    iretq
.Lexhi_syscall_fatal:
    mov dx, 0xF4
    mov eax, 0x11
    out dx, eax
    cli
.Lexhi_syscall_halt:
    hlt
    jmp .Lexhi_syscall_halt
    "#
);
