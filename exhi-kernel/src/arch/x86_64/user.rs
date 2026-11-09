//! Production x86-64 user-entry primitives.
//!
//! This module owns the common IRETQ transition, validates user mappings and
//! the dedicated supervisor stack, and decodes processor-originated exceptions.
//! The initial implementation is single-CPU; per-CPU syscall state and task
//! fault scheduling are separate milestones.

use core::arch::asm;
#[cfg(feature = "process-runtime")]
use core::arch::global_asm;
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
use core::mem::size_of;
use core::ptr;

use super::address_space::{AddressSpace, AddressSpaceError};
use super::{gdt, vmm};

const PAGE_SIZE: u64 = 4096;
#[cfg(feature = "process-runtime")]
const FX_STATE_SIZE: usize = 512;
#[cfg(feature = "process-runtime")]
const DEFAULT_MXCSR: u32 = 0x1F80;
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
const USER_CANONICAL_LIMIT: u64 = 0x0000_8000_0000_0000;
const RFLAGS_IF: u64 = 1 << 9;
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
const RFLAGS_RESERVED_ONE: u64 = 1 << 1;
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
const RFLAGS_IOPL: u64 = 3 << 12;
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
const RFLAGS_NT: u64 = 1 << 14;
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
const RFLAGS_VM: u64 = 1 << 17;

#[cfg(feature = "process-runtime")]
#[repr(C, align(16))]
pub struct FxState([u8; FX_STATE_SIZE]);

#[cfg(feature = "process-runtime")]
impl FxState {
    pub const fn zeroed() -> Self {
        Self([0; FX_STATE_SIZE])
    }

    /// Initializes the CPU legacy x87/SSE state and stores its clean image.
    /// Call during single-CPU startup with IF=0, before kernel FP use. FXSAVE
    /// covers only x87/SSE; AVX or other XSTATE components are rejected.
    pub fn initialize(&mut self) -> Result<(), UserEntryError> {
        if interrupts_enabled() {
            return Err(UserEntryError::InterruptsEnabled);
        }
        let features = core::arch::x86_64::__cpuid(1);
        const REQUIRED_FEATURES: u32 = (1 << 24) | (1 << 25) | (1 << 26);
        if features.edx & REQUIRED_FEATURES != REQUIRED_FEATURES {
            return Err(UserEntryError::FxStateUnsupported);
        }

        let mut cr0: u64;
        let mut cr4: u64;
        unsafe {
            asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
            asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
        }
        cr0 = (cr0 | (1 << 1) | (1 << 5)) & !((1 << 2) | (1 << 3));

        if cr4 & (1 << 18) != 0 {
            let eax: u32;
            let edx: u32;
            unsafe {
                asm!(
                    "xgetbv",
                    in("ecx") 0u32,
                    out("eax") eax,
                    out("edx") edx,
                    options(nomem, nostack, preserves_flags)
                );
            }
            let xcr0 = (u64::from(edx) << 32) | u64::from(eax);
            if xcr0 & 0x3 != 0x3 || xcr0 & !0x3 != 0 {
                return Err(UserEntryError::ExtendedXstateUnsupported);
            }
        }

        cr4 |= (1 << 9) | (1 << 10); // OSFXSR | OSXMMEXCPT
        let state_pointer = ptr::addr_of_mut!(self.0).cast::<u8>();
        unsafe {
            asm!(
                "mov cr0, {cr0}",
                "mov cr4, {cr4}",
                "fninit",
                "ldmxcsr [{mxcsr}]",
                "fxsave64 [{state}]",
                cr0 = in(reg) cr0,
                cr4 = in(reg) cr4,
                mxcsr = in(reg) &DEFAULT_MXCSR,
                state = in(reg) state_pointer,
                options(nostack)
            );
        }
        Ok(())
    }
}

/// Stable register image consumed by the assembly IRETQ entry routine.
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
#[repr(C, align(16))]
pub struct UserEntryFrame {
    pub(crate) r15: u64,
    pub(crate) r14: u64,
    pub(crate) r13: u64,
    pub(crate) r12: u64,
    pub(crate) r11: u64,
    pub(crate) r10: u64,
    pub(crate) r9: u64,
    pub(crate) r8: u64,
    pub(crate) rdi: u64,
    pub(crate) rsi: u64,
    pub(crate) rbp: u64,
    pub(crate) rbx: u64,
    pub(crate) rdx: u64,
    pub(crate) rcx: u64,
    pub(crate) rax: u64,
    pub(crate) rip: u64,
    pub(crate) cs: u64,
    pub(crate) rflags: u64,
    pub(crate) rsp: u64,
    pub(crate) ss: u64,
}
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
const _: [(); 20 * size_of::<u64>()] = [(); size_of::<UserEntryFrame>()];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserEntryError {
    InterruptsEnabled,
    AddressSpaceNotActive,
    InvalidInstructionPointer,
    InvalidUserStackPointer,
    CodeMappingRejected,
    StackMappingRejected,
    InvalidKernelStack,
    KernelStackMappingRejected,
    InvalidFxState,
    FxMappingRejected,
    EntryFrameMappingRejected,
    TssStackRejected,
    InvalidExceptionFrame,
    #[cfg(feature = "process-runtime")]
    FxStateUnsupported,
    #[cfg(feature = "process-runtime")]
    ExtendedXstateUnsupported,
}

#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserException {
    pub vector: u64,
    pub error_code: u64,
    pub instruction_pointer: u64,
    pub flags: u64,
    pub user_stack_pointer: u64,
}

static mut ACTIVE_KERNEL_STACK_BOTTOM: u64 = 0;
static mut ACTIVE_KERNEL_STACK_TOP: u64 = 0;

/// Changes the trusted kernel stack bounds used for the next CPL3-to-CPL0
/// transition. The address space must already be active; callers serialize
/// this operation with IF=0 before restoring a user context.
pub fn activate_kernel_stack(
    address_space: &AddressSpace,
    kernel_stack_bottom: u64,
    kernel_stack_top: u64,
) -> Result<(), UserEntryError> {
    if interrupts_enabled() {
        return Err(UserEntryError::InterruptsEnabled);
    }
    if !address_space
        .is_active()
        .map_err(|_| UserEntryError::AddressSpaceNotActive)?
    {
        return Err(UserEntryError::AddressSpaceNotActive);
    }
    if kernel_stack_bottom == 0
        || kernel_stack_bottom & (PAGE_SIZE - 1) != 0
        || kernel_stack_top <= kernel_stack_bottom
        || kernel_stack_top & 0xF != 0
        || !is_kernel_canonical(kernel_stack_bottom)
        || !is_kernel_canonical(kernel_stack_top - 1)
    {
        return Err(UserEntryError::InvalidKernelStack);
    }
    validate_kernel_range(kernel_stack_bottom, kernel_stack_top)
        .map_err(|_| UserEntryError::KernelStackMappingRejected)?;
    if !gdt::set_rsp0(kernel_stack_top) || gdt::read_rsp0() != kernel_stack_top {
        return Err(UserEntryError::TssStackRejected);
    }

    // The exception-frame validator must follow the process selected by the
    // scheduler, not retain the first process's stack bounds globally.
    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(ACTIVE_KERNEL_STACK_BOTTOM),
            kernel_stack_bottom,
        );
        ptr::write_volatile(ptr::addr_of_mut!(ACTIVE_KERNEL_STACK_TOP), kernel_stack_top);
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    let bottom = unsafe { ptr::read_volatile(ptr::addr_of!(ACTIVE_KERNEL_STACK_BOTTOM)) };
    let top = unsafe { ptr::read_volatile(ptr::addr_of!(ACTIVE_KERNEL_STACK_TOP)) };
    if bottom != kernel_stack_bottom || top != kernel_stack_top {
        return Err(UserEntryError::InvalidKernelStack);
    }
    Ok(())
}

/// Clears the active user-entry stack when execution is handed to a kernel-only
/// context. A future user dispatch must install its own bounds again.
#[cfg(feature = "process-runtime")]
pub fn clear_active_kernel_stack() -> Result<(), UserEntryError> {
    if interrupts_enabled() {
        return Err(UserEntryError::InterruptsEnabled);
    }
    if !gdt::set_rsp0(0) || gdt::read_rsp0() != 0 {
        return Err(UserEntryError::TssStackRejected);
    }
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(ACTIVE_KERNEL_STACK_BOTTOM), 0);
        ptr::write_volatile(ptr::addr_of_mut!(ACTIVE_KERNEL_STACK_TOP), 0);
    }
    Ok(())
}

#[cfg(feature = "process-runtime")]
unsafe extern "sysv64" {
    fn exhi_user_iret_entry(frame: *const UserEntryFrame, fx_state: *const FxState) -> !;
}

/// Validates user mappings and installs the stack used on CPL3-to-CPL0 entry.
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
pub fn prepare_entry(
    address_space: &AddressSpace,
    kernel_stack_bottom: u64,
    kernel_stack_top: u64,
    user_instruction_pointer: u64,
    user_stack_pointer: u64,
) -> Result<UserEntryFrame, UserEntryError> {
    if interrupts_enabled() {
        return Err(UserEntryError::InterruptsEnabled);
    }
    if !address_space
        .is_active()
        .map_err(|_| UserEntryError::AddressSpaceNotActive)?
    {
        return Err(UserEntryError::AddressSpaceNotActive);
    }
    if !is_user_address(user_instruction_pointer) {
        return Err(UserEntryError::InvalidInstructionPointer);
    }
    if !is_user_address(user_stack_pointer) {
        return Err(UserEntryError::InvalidUserStackPointer);
    }

    validate_user_range(
        address_space,
        user_instruction_pointer,
        1,
        UserAccess::Execute,
    )
    .map_err(|_| UserEntryError::CodeMappingRejected)?;
    let stack = address_space
        .translate(user_stack_pointer - 1)
        .map_err(|_| UserEntryError::StackMappingRejected)?;
    if !stack.user || !stack.writable || stack.executable {
        return Err(UserEntryError::StackMappingRejected);
    }

    activate_kernel_stack(address_space, kernel_stack_bottom, kernel_stack_top)?;

    Ok(UserEntryFrame {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: 0,
        r11: 0,
        r10: 0,
        r9: 0,
        r8: 0,
        rdi: 0,
        rsi: 0,
        rbp: 0,
        rbx: 0,
        rdx: 0,
        rcx: 0,
        rax: 0,
        rip: user_instruction_pointer,
        cs: u64::from(gdt::USER_CODE_SELECTOR),
        rflags: 0x202,
        rsp: user_stack_pointer,
        ss: u64::from(gdt::USER_DATA_SELECTOR),
    })
}

/// Restores the initialized user FX image and performs the CPL3 IRETQ.
/// A successful call transfers control to user mode and never returns here.
#[cfg(feature = "process-runtime")]
pub fn enter_user(frame: &UserEntryFrame, fx_state: &FxState) -> Result<(), UserEntryError> {
    let frame_pointer = frame as *const UserEntryFrame as u64;
    let fx_pointer = fx_state as *const FxState as u64;
    if frame_pointer & 0xF != 0 || !is_kernel_canonical(frame_pointer) {
        return Err(UserEntryError::EntryFrameMappingRejected);
    }
    if fx_pointer & 0xF != 0 || !is_kernel_canonical(fx_pointer) {
        return Err(UserEntryError::InvalidFxState);
    }
    validate_kernel_range(
        frame_pointer,
        frame_pointer + size_of::<UserEntryFrame>() as u64,
    )
    .map_err(|_| UserEntryError::EntryFrameMappingRejected)?;
    validate_kernel_range(fx_pointer, fx_pointer + FX_STATE_SIZE as u64)
        .map_err(|_| UserEntryError::FxMappingRejected)?;

    // SAFETY: prepare_entry made the TSS stack active; both objects are
    // supervisor RW+NX mappings shared by the active address space.
    unsafe { exhi_user_iret_entry(frame, fx_state) }
}

/// Validates the full seven-qword hardware privilege-return frame before
/// dereferencing it. The pointer must lie on the stack installed by prepare_entry.
#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
pub fn capture_exception(
    vector: u64,
    error_code: u64,
    instruction_pointer: u64,
    code_selector: u64,
    flags: u64,
    exception_stack_pointer: u64,
) -> Result<UserException, UserEntryError> {
    let bottom = unsafe { ptr::read_volatile(ptr::addr_of!(ACTIVE_KERNEL_STACK_BOTTOM)) };
    let top = unsafe { ptr::read_volatile(ptr::addr_of!(ACTIVE_KERNEL_STACK_TOP)) };
    let frame_end = exception_stack_pointer
        .checked_add(7 * size_of::<u64>() as u64)
        .ok_or(UserEntryError::InvalidExceptionFrame)?;
    if bottom == 0
        || top <= bottom
        || exception_stack_pointer & 7 != 0
        || exception_stack_pointer < bottom
        || frame_end > top
    {
        return Err(UserEntryError::InvalidExceptionFrame);
    }

    let words = exception_stack_pointer as *const u64;
    let frame = unsafe {
        [
            ptr::read_volatile(words),
            ptr::read_volatile(words.add(1)),
            ptr::read_volatile(words.add(2)),
            ptr::read_volatile(words.add(3)),
            ptr::read_volatile(words.add(4)),
            ptr::read_volatile(words.add(5)),
            ptr::read_volatile(words.add(6)),
        ]
    };
    if frame[0] != vector
        || frame[1] != error_code
        || frame[2] != instruction_pointer
        || frame[3] != code_selector
        || frame[4] != flags
        || code_selector != u64::from(gdt::USER_CODE_SELECTOR)
        || frame[6] != u64::from(gdt::USER_DATA_SELECTOR)
        || !is_user_address(frame[2])
        || !is_user_address(frame[5])
        || flags & RFLAGS_RESERVED_ONE == 0
        || flags & (RFLAGS_IOPL | RFLAGS_NT | RFLAGS_VM) != 0
    {
        return Err(UserEntryError::InvalidExceptionFrame);
    }
    Ok(UserException {
        vector,
        error_code,
        instruction_pointer,
        flags,
        user_stack_pointer: frame[5],
    })
}

#[allow(dead_code)] // Variants are exercised by distinct user-pointer/copy feature sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserAccess {
    Read,
    Write,
    Execute,
}

#[allow(dead_code)] // Copy-context errors only occur in copy-capable feature sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserRangeError {
    AddressOverflow,
    RangeTooLarge,
    NotUserMemory,
    PermissionDenied,
    AddressSpace(AddressSpaceError),
    AddressSpaceNotActive,
    InterruptsEnabled,
}

const MAX_USER_POINTER_RANGE: u64 = 16 * 1024 * 1024;

/// Validates every page touched by a user buffer before a syscall accesses it.
/// Zero-length buffers are accepted without dereferencing their pointer.
pub fn validate_user_range(
    space: &AddressSpace,
    address: u64,
    length: usize,
    access: UserAccess,
) -> Result<(), UserRangeError> {
    if length == 0 {
        return Ok(());
    }
    let length = u64::try_from(length).map_err(|_| UserRangeError::RangeTooLarge)?;
    if length > MAX_USER_POINTER_RANGE {
        return Err(UserRangeError::RangeTooLarge);
    }
    let last = address
        .checked_add(length - 1)
        .ok_or(UserRangeError::AddressOverflow)?;
    let mut page = address & !(PAGE_SIZE - 1);
    let last_page = last & !(PAGE_SIZE - 1);

    loop {
        let mapping = space
            .translate(page)
            .map_err(UserRangeError::AddressSpace)?;
        if !mapping.user {
            return Err(UserRangeError::NotUserMemory);
        }
        let permitted = match access {
            UserAccess::Read => true,
            UserAccess::Write => mapping.writable,
            UserAccess::Execute => mapping.executable && !mapping.writable,
        };
        if !permitted {
            return Err(UserRangeError::PermissionDenied);
        }
        if page == last_page {
            break;
        }
        page = page
            .checked_add(PAGE_SIZE)
            .ok_or(UserRangeError::AddressOverflow)?;
    }
    Ok(())
}

#[cfg(any(
    feature = "process-runtime",
    feature = "user-copy-smoke-test",
    feature = "user-task-lifecycle-smoke-test"
))]
/// Copies a user buffer into a kernel-owned slice after validating every page.
/// IF must be clear and the address space active so it cannot be switched while
/// the validated mapping is accessed.
pub fn copy_from_user(
    space: &AddressSpace,
    user_source: u64,
    kernel_destination: &mut [u8],
) -> Result<(), UserRangeError> {
    ensure_user_copy_context(space)?;
    validate_user_range(
        space,
        user_source,
        kernel_destination.len(),
        UserAccess::Read,
    )?;
    if !kernel_destination.is_empty() {
        unsafe {
            ptr::copy(
                user_source as *const u8,
                kernel_destination.as_mut_ptr(),
                kernel_destination.len(),
            );
        }
    }
    Ok(())
}

/// Copies a kernel-owned slice into user memory only after the whole destination
/// range has been validated. No partial writes occur for rejected ranges.
#[cfg(any(feature = "process-runtime", feature = "user-copy-smoke-test"))]
pub fn copy_to_user(
    space: &AddressSpace,
    user_destination: u64,
    kernel_source: &[u8],
) -> Result<(), UserRangeError> {
    ensure_user_copy_context(space)?;
    validate_user_range(
        space,
        user_destination,
        kernel_source.len(),
        UserAccess::Write,
    )?;
    if !kernel_source.is_empty() {
        unsafe {
            ptr::copy(
                kernel_source.as_ptr(),
                user_destination as *mut u8,
                kernel_source.len(),
            );
        }
    }
    Ok(())
}

#[cfg(any(
    feature = "process-runtime",
    feature = "user-copy-smoke-test",
    feature = "user-task-lifecycle-smoke-test"
))]
fn ensure_user_copy_context(space: &AddressSpace) -> Result<(), UserRangeError> {
    if interrupts_enabled() {
        return Err(UserRangeError::InterruptsEnabled);
    }
    if !space.is_active().map_err(UserRangeError::AddressSpace)? {
        return Err(UserRangeError::AddressSpaceNotActive);
    }
    Ok(())
}

fn validate_kernel_range(start: u64, end: u64) -> Result<(), ()> {
    if start == 0 || end <= start || !is_kernel_canonical(start) || !is_kernel_canonical(end - 1) {
        return Err(());
    }
    let mut page = start & !(PAGE_SIZE - 1);
    let last_page = (end - 1) & !(PAGE_SIZE - 1);
    loop {
        let mapping = vmm::query_page(page).map_err(|_| ())?;
        // The bootstrap 2 MiB identity map is currently RWX. These kernel
        // objects remain supervisor-only; a later kernel W^X pass will make
        // writable stacks/data explicitly NX while keeping text RX.
        if mapping.flags.user || !mapping.flags.writable {
            return Err(());
        }
        if page == last_page {
            break;
        }
        page = page.checked_add(PAGE_SIZE).ok_or(())?;
    }
    Ok(())
}

#[cfg(any(
    feature = "process-runtime",
    all(feature = "ring3-smoke-test", not(feature = "user-task-lifecycle-smoke-test"))
))]
fn is_user_address(address: u64) -> bool {
    address != 0 && address < USER_CANONICAL_LIMIT
}

fn is_kernel_canonical(address: u64) -> bool {
    address >> 47 == 0 || address >> 47 == 0x1_FFFF
}

fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags));
    }
    flags & RFLAGS_IF != 0
}

#[cfg(feature = "process-runtime")]
global_asm!(
    r#"
    .section .text
    .global exhi_user_iret_entry
    .type exhi_user_iret_entry, @function
exhi_user_iret_entry:
    fxrstor64 [rsi]
    mov rsp, rdi
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
    "#
);
