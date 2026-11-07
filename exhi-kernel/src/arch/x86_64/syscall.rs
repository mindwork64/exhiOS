//! x86-64 SYSCALL/SYSRET MSR configuration.
//!
//! This module only installs the processor's syscall MSR contract. It does
//! not install an entry stack or enable user tasks; callers must provide a
//! valid kernel entry point and must not expose Ring 3 until the runtime is
//! ready to handle it.

use core::arch::{asm, x86_64::__cpuid};

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

    let max_extended = unsafe { __cpuid(0x8000_0000) };
    if max_extended.eax < 0x8000_0001 {
        return Err(SyscallInitError::UnsupportedCpu);
    }
    let features = unsafe { __cpuid(0x8000_0001) };
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
    address >> 48 == 0 || address >> 48 == 0xFFFF
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
