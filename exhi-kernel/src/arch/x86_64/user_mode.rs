//! Ring 3 transition, user-memory and syscall smoke-test harness.
//!
//! Feature-gated paths exercise the architecture boundary; they are not yet
//! the production user-process runtime.

use core::arch::asm;
#[cfg(any(
    feature = "ring3-syscall-smoke-test",
    feature = "user-task-lifecycle-smoke-test"
))]
use core::arch::global_asm;
use core::mem::size_of;
use core::ptr;

#[cfg(feature = "user-task-lifecycle-smoke-test")]
use exhi_abi::{SyscallError, SyscallNumber, SyscallRequest};
#[cfg(feature = "user-task-lifecycle-smoke-test")]
use exhi_syscall::{dispatch as dispatch_syscall, DispatchOutcome, SyscallServices};
#[cfg(feature = "user-task-lifecycle-smoke-test")]
use exhi_task::{Capability, CapabilitySet, Scheduler, TaskContext, TaskId, UserTaskMetadata};

use super::address_space::{self, AddressSpace, UserPageFlags};
#[cfg(any(
    feature = "ring3-syscall-smoke-test",
    feature = "user-task-lifecycle-smoke-test"
))]
use super::syscall;
#[cfg(any(
    feature = "ring3-syscall-smoke-test",
    feature = "user-task-lifecycle-smoke-test"
))]
use super::syscall::UserTaskSyscallFrame as UserTaskInterruptFrame;
#[cfg(not(feature = "user-task-lifecycle-smoke-test"))]
use super::user;
#[cfg(any(
    feature = "user-copy-smoke-test",
    feature = "user-task-lifecycle-smoke-test"
))]
pub use super::user::copy_from_user;
#[cfg(feature = "user-copy-smoke-test")]
pub use super::user::copy_to_user;
#[cfg(any(
    feature = "user-pointer-range-smoke-test",
    feature = "user-copy-smoke-test",
    feature = "user-task-lifecycle-smoke-test"
))]
pub use super::user::UserRangeError;
#[cfg(feature = "user-pointer-range-smoke-test")]
pub use super::user::{validate_user_range, UserAccess};
use super::{gdt, serial, task, vmm};
use crate::memory;

const PAGE_SIZE: u64 = 4096;
const STACK_USABLE_SIZE: usize = 16 * 1024;
const STACK_TOTAL_SIZE: usize = PAGE_SIZE as usize + STACK_USABLE_SIZE;
const USER_CODE_ADDRESS: u64 = 0x0000_0002_0000_0000;
const USER_STACK_ADDRESS: u64 = USER_CODE_ADDRESS + (2 * PAGE_SIZE);
const USER_STACK_TOP: u64 = USER_STACK_ADDRESS + PAGE_SIZE;
const USER_STACK_POINTER: u64 = USER_STACK_TOP - 8;
const USER_STACK_SENTINEL: u64 = 0x0000_0000_5A6B_7C8D;
#[cfg(not(feature = "user-task-lifecycle-smoke-test"))]
const USER_CODE_LENGTH: u64 = 10;

#[repr(u64)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Ring3TestMode {
    Breakpoint = 1,
    SupervisorFault = 2,
    Syscall = 3,
    #[cfg(feature = "user-pointer-range-smoke-test")]
    PointerValidation = 4,
    #[cfg(feature = "user-copy-smoke-test")]
    UserCopy = 5,
}

#[cfg(not(feature = "user-task-lifecycle-smoke-test"))]
static RING3_SUPERVISOR_PROBE: u64 = 0x4558_4849_4F53_5046;
static mut RING3_TEST_MODE: u64 = 0;
static mut RING3_EXPECTED_VECTOR: u64 = 3;
static mut RING3_EXPECTED_ERROR_MASK: u64 = u64::MAX;
static mut RING3_EXPECTED_ERROR_VALUE: u64 = 0;
static mut RING3_EXPECTED_RIP: u64 = 0;
static mut RING3_EXPECTED_FAULT_ADDRESS: u64 = 0;
#[no_mangle]
#[no_mangle]
static mut RING3_FX_STATE: usize = 0;
static mut RING3_SYSCALL_CALLS: u64 = 0;
static mut RING3_SYSCALL_VALID: u64 = 0;

#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_SCHEDULER: Scheduler<3> = Scheduler::new();
#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_KERNEL_STACKS: [KernelStack; 3] = [
    KernelStack([0; STACK_TOTAL_SIZE]),
    KernelStack([0; STACK_TOTAL_SIZE]),
    KernelStack([0; STACK_TOTAL_SIZE]),
];
#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_KERNEL_STACK_TOPS: [u64; 3] = [0; 3];
#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_SPACE_A: Option<AddressSpace> = None;
#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_SPACE_B: Option<AddressSpace> = None;
#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_CODE_FRAMES: [u64; 2] = [0; 2];
#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_STACK_FRAMES: [u64; 2] = [0; 2];
#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[repr(C, align(16))]
struct UserTaskFxState([u8; 512]);
#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_FX_STATES: [UserTaskFxState; 3] = [
    UserTaskFxState([0; 512]),
    UserTaskFxState([0; 512]),
    UserTaskFxState([0; 512]),
];
#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[repr(C, align(64))]
struct UserTaskCounters {
    syscall_calls: u64,
    yields: u64,
    exits: u64,
    denied: u64,
    console_writes: u64,
    switches: u64,
}
#[cfg(feature = "user-task-lifecycle-smoke-test")]
static mut USER_TASK_COUNTERS: UserTaskCounters = UserTaskCounters {
    syscall_calls: 0,
    yields: 0,
    exits: 0,
    denied: 0,
    console_writes: 0,
    switches: 0,
};

#[repr(C, align(4096))]
struct KernelStack([u8; STACK_TOTAL_SIZE]);

static mut RING3_KERNEL_STACK: KernelStack = KernelStack([0; STACK_TOTAL_SIZE]);
#[no_mangle]
static mut RING3_KERNEL_STACK_TOP: u64 = 0;
static mut RING3_ADDRESS_SPACE: Option<AddressSpace> = None;
static mut RING3_CODE_FRAME: u64 = 0;
static mut RING3_STACK_FRAME: u64 = 0;

unsafe extern "sysv64" {
    fn exhi_interrupt_context_start(initial_rsp: usize, fx_state: usize) -> !;
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
unsafe extern "sysv64" {
    fn exhi_user_task_kernel_entry() -> !;
}

#[cfg(all(
    feature = "ring3-smoke-test",
    not(any(
        feature = "ring3-supervisor-fault-smoke-test",
        feature = "ring3-syscall-smoke-test",
        feature = "user-pointer-range-smoke-test",
        feature = "user-copy-smoke-test",
        feature = "user-task-lifecycle-smoke-test"
    ))
))]
pub fn ring3_smoke_test() -> ! {
    run_ring3_test(Ring3TestMode::Breakpoint)
}

#[cfg(feature = "ring3-syscall-smoke-test")]
pub fn ring3_syscall_smoke_test() -> ! {
    run_ring3_test(Ring3TestMode::Syscall)
}

#[cfg(feature = "user-pointer-range-smoke-test")]
pub fn user_pointer_validation_smoke_test() -> ! {
    run_ring3_test(Ring3TestMode::PointerValidation)
}

#[cfg(feature = "user-copy-smoke-test")]
pub fn user_copy_smoke_test() -> ! {
    run_ring3_test(Ring3TestMode::UserCopy)
}

#[cfg(feature = "ring3-supervisor-fault-smoke-test")]
pub fn ring3_supervisor_fault_smoke_test() -> ! {
    run_ring3_test(Ring3TestMode::SupervisorFault)
}

#[cfg(not(feature = "user-task-lifecycle-smoke-test"))]
fn run_ring3_test(mode: Ring3TestMode) -> ! {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
        ptr::write(ptr::addr_of_mut!(RING3_ADDRESS_SPACE), None);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_CODE_FRAME), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_STACK_FRAME), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_KERNEL_STACK_TOP), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_TEST_MODE), mode as u64);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_EXPECTED_VECTOR), 3);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_EXPECTED_ERROR_MASK), u64::MAX);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_EXPECTED_ERROR_VALUE), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_EXPECTED_RIP), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_EXPECTED_FAULT_ADDRESS), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_FX_STATE), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_SYSCALL_CALLS), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_SYSCALL_VALID), 0);
    }

    serial::COM1.write(match mode {
        Ring3TestMode::Breakpoint => b"P5.1: preparing isolated Ring 3 address space.\r\n",
        Ring3TestMode::SupervisorFault => b"P5.2: preparing Ring 3 supervisor-isolation test.\r\n",
        Ring3TestMode::Syscall => b"P5.3: preparing Ring 3 SYSCALL/SYSRET test.\r\n",
        #[cfg(feature = "user-pointer-range-smoke-test")]
        Ring3TestMode::PointerValidation => {
            b"P5.4: preparing user-pointer range validation test.\r\n"
        }
        #[cfg(feature = "user-copy-smoke-test")]
        Ring3TestMode::UserCopy => b"P5.5: preparing safe user-copy runtime test.\r\n",
    });

    let Some(fx_state) = task::initialize_fx_state_for_user_mode() else {
        setup_failed(b"P5.1: FXSR/SSE context initialization failed.\r\n");
    };

    let kernel_stack_base = ptr::addr_of_mut!(RING3_KERNEL_STACK) as u64;
    let Some(kernel_stack_top) = kernel_stack_base.checked_add(STACK_TOTAL_SIZE as u64) else {
        setup_failed(b"P5.1: kernel stack address overflow.\r\n");
    };
    if kernel_stack_base % PAGE_SIZE != 0
        || kernel_stack_top % 16 != 0
        || kernel_stack_top >= 0x1_0000_0000
    {
        setup_failed(b"P5.1: invalid Ring 3 kernel stack bounds.\r\n");
    }
    if vmm::unmap_page(kernel_stack_base).ok() != Some(kernel_stack_base)
        || super::paging::translate_current(kernel_stack_base)
            != Err(super::paging::PagingError::NotMapped)
        || !gdt::set_rsp0(kernel_stack_top)
    {
        setup_failed(b"P5.1: could not install the guarded Ring 3 kernel stack.\r\n");
    }
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(RING3_KERNEL_STACK_TOP), kernel_stack_top);
    }

    let code_frame = allocate_zeroed_frame();
    let stack_frame = allocate_zeroed_frame();
    let (
        user_code,
        user_code_length,
        expected_vector,
        expected_error_mask,
        expected_error_value,
        expected_rip,
        expected_fault_address,
    ) = build_user_code(mode);
    unsafe {
        ptr::copy_nonoverlapping(user_code.as_ptr(), code_frame as *mut u8, user_code_length);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_EXPECTED_VECTOR), expected_vector);
        ptr::write_volatile(
            ptr::addr_of_mut!(RING3_EXPECTED_ERROR_MASK),
            expected_error_mask,
        );
        ptr::write_volatile(
            ptr::addr_of_mut!(RING3_EXPECTED_ERROR_VALUE),
            expected_error_value,
        );
        ptr::write_volatile(
            ptr::addr_of_mut!(RING3_EXPECTED_RIP),
            USER_CODE_ADDRESS + expected_rip,
        );
        ptr::write_volatile(
            ptr::addr_of_mut!(RING3_EXPECTED_FAULT_ADDRESS),
            expected_fault_address,
        );
        ptr::write_volatile(ptr::addr_of_mut!(RING3_CODE_FRAME), code_frame);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_STACK_FRAME), stack_frame);
    }

    let mut space = match AddressSpace::create() {
        Ok(space) => space,
        Err(_) => setup_failed(b"P5.1: could not create a private user address space.\r\n"),
    };

    // The code page is user-readable/executable but not writable. The stack
    // is user-readable/writable and NX; one unmapped page remains as a guard.
    if unsafe {
        space.map_user_page(
            USER_CODE_ADDRESS,
            code_frame,
            UserPageFlags {
                writable: false,
                executable: true,
            },
        )
    }
    .is_err()
    {
        setup_failed(b"P5.1: could not map the user executable page.\r\n");
    }
    if unsafe {
        space.map_user_page(
            USER_STACK_ADDRESS,
            stack_frame,
            UserPageFlags::read_write_nx(),
        )
    }
    .is_err()
    {
        setup_failed(b"P5.1: could not map the user stack page.\r\n");
    }

    let code_mapping = space.translate(USER_CODE_ADDRESS);
    let guard_mapping = space.translate(USER_CODE_ADDRESS + PAGE_SIZE);
    let stack_mapping = space.translate(USER_STACK_ADDRESS);
    let permissions_valid = code_mapping
        .is_ok_and(|mapping| mapping.user && mapping.executable && !mapping.writable)
        && guard_mapping.is_err()
        && stack_mapping
            .is_ok_and(|mapping| mapping.user && !mapping.executable && mapping.writable);
    if !permissions_valid {
        setup_failed(b"P5.1: user W^X or user/supervisor permissions are incorrect.\r\n");
    }

    unsafe {
        ptr::write(ptr::addr_of_mut!(RING3_ADDRESS_SPACE), Some(space));
    }

    if mode == Ring3TestMode::Breakpoint {
        let slot = unsafe { &*ptr::addr_of!(RING3_ADDRESS_SPACE) };
        let inactive_rejected = slot.as_ref().is_some_and(|space| {
            matches!(
                user::prepare_entry(
                    space,
                    kernel_stack_base + PAGE_SIZE,
                    kernel_stack_top,
                    USER_CODE_ADDRESS,
                    USER_STACK_POINTER,
                ),
                Err(user::UserEntryError::AddressSpaceNotActive)
            )
        });
        if !inactive_rejected {
            setup_failed(b"P5.1: inactive address space was accepted for user entry.\\r\\n");
        }
        serial::COM1.write(b"P5.1: inactive address-space entry rejected.\\r\\n");
    }

    serial::COM1.write(match mode {
        Ring3TestMode::Breakpoint => b"P5.1: user code mapped RX; user stack mapped RW+NX.\r\n",
        Ring3TestMode::SupervisorFault => {
            b"P5.2: user RX code and RW+NX stack mapped; kernel probe is supervisor-only.\r\n"
        }
        Ring3TestMode::Syscall => {
            b"P5.3: user RX code and RW+NX stack mapped; syscall MSRs will be verified.\r\n"
        }
        #[cfg(feature = "user-pointer-range-smoke-test")]
        Ring3TestMode::PointerValidation => {
            b"P5.4: user RX code, guard and RW+NX stack pages prepared.\r\n"
        }
        #[cfg(feature = "user-copy-smoke-test")]
        Ring3TestMode::UserCopy => b"P5.5: user RW+NX buffer and guard mappings prepared.\r\n",
    });
    serial::COM1.write(
        b"Ring 3: IRETQ enters CPL3; the expected CPU exception returns through TSS.RSP0.\r\n",
    );

    #[cfg(feature = "user-pointer-range-smoke-test")]
    if mode == Ring3TestMode::PointerValidation {
        let space_slot = unsafe { &*ptr::addr_of!(RING3_ADDRESS_SPACE) };
        let range_checks = space_slot
            .as_ref()
            .is_some_and(validate_user_pointer_ranges);
        let cleanup_ok = reclaim_user_address_space();
        let rsp0_cleared = gdt::set_rsp0(0) && gdt::read_rsp0() == 0;
        serial::COM1.write(if range_checks {
            b"P5.4: user/read/write/execute/guard/overflow range checks passed.\r\n"
        } else {
            b"P5.4: user-pointer range validation failed.\r\n"
        });
        finish(range_checks && cleanup_ok && rsp0_cleared);
    }

    #[cfg(feature = "user-copy-smoke-test")]
    if mode == Ring3TestMode::UserCopy {
        let space_slot = unsafe { &*ptr::addr_of!(RING3_ADDRESS_SPACE) };
        let inactive_rejected = space_slot.as_ref().is_some_and(|space| {
            let mut byte = [0u8; 1];
            copy_from_user(space, USER_STACK_ADDRESS, &mut byte)
                == Err(UserRangeError::AddressSpaceNotActive)
        });
        let copy_checks = if let Some(space) = space_slot.as_ref() {
            space.activate().is_ok() && validate_user_copy_runtime(space)
        } else {
            false
        };
        let cleanup_ok = reclaim_user_address_space();
        let rsp0_cleared = gdt::set_rsp0(0) && gdt::read_rsp0() == 0;
        serial::COM1.write(if inactive_rejected {
            b"P5.5: inactive address-space copy rejected.\r\n"
        } else {
            b"P5.5: inactive address-space copy was not rejected.\r\n"
        });
        serial::COM1.write(if copy_checks {
            b"P5.5: copy_from_user/copy_to_user round-trip and atomic rejection passed.\r\n"
        } else {
            b"P5.5: user-copy runtime test failed.\r\n"
        });
        finish(inactive_rejected && copy_checks && cleanup_ok && rsp0_cleared);
    }

    #[cfg(feature = "ring3-syscall-smoke-test")]
    if mode == Ring3TestMode::Syscall {
        unsafe {
            ptr::write_volatile(ptr::addr_of_mut!(RING3_FX_STATE), fx_state);
        }
        if !initialize_syscall_msrs() {
            setup_failed(b"P5.3: syscall MSR configuration or read-back failed.\r\n");
        }
    }

    let space_slot = unsafe { &*ptr::addr_of!(RING3_ADDRESS_SPACE) };
    let Some(space) = space_slot.as_ref() else {
        setup_failed(b"P5.1: Ring 3 address space disappeared before entry.\r\n");
    };
    if space.activate().is_err() {
        setup_failed(b"P5.1: CR3 activation for Ring 3 failed.\r\n");
    }

    if mode == Ring3TestMode::Breakpoint {
        let bottom = kernel_stack_base + PAGE_SIZE;
        let invalid_code_rejected = matches!(
            user::prepare_entry(
                space,
                bottom,
                kernel_stack_top,
                USER_STACK_ADDRESS,
                USER_STACK_POINTER,
            ),
            Err(user::UserEntryError::CodeMappingRejected)
        );
        let invalid_stack_rejected = matches!(
            user::prepare_entry(
                space,
                bottom,
                kernel_stack_top,
                USER_CODE_ADDRESS,
                USER_CODE_ADDRESS,
            ),
            Err(user::UserEntryError::StackMappingRejected)
        );
        let invalid_kernel_stack_rejected = matches!(
            user::prepare_entry(
                space,
                bottom + 1,
                kernel_stack_top,
                USER_CODE_ADDRESS,
                USER_STACK_POINTER,
            ),
            Err(user::UserEntryError::InvalidKernelStack)
        );
        if !(invalid_code_rejected
            && invalid_stack_rejected
            && invalid_kernel_stack_rejected
            && gdt::read_rsp0() == kernel_stack_top)
        {
            setup_failed(b"P5.1: invalid code/stack/kernel-stack entry checks failed.\r\n");
        }
        serial::COM1.write(
            b"P5.1: invalid code, non-writable user stack and malformed kernel stack rejected.\r\n",
        );
    }

    let fx_state = unsafe { &*(fx_state as *const user::FxState) };
    let kernel_stack_bottom = kernel_stack_base + PAGE_SIZE;
    let frame = user::prepare_entry(
        space,
        kernel_stack_bottom,
        kernel_stack_top,
        USER_CODE_ADDRESS,
        USER_STACK_POINTER,
    )
    .unwrap_or_else(|_| setup_failed(b"P5.1: production user-entry validation failed.\r\n"));
    match user::enter_user(&frame, fx_state) {
        Ok(()) => setup_failed(b"P5.1: user-entry assembly unexpectedly returned.\r\n"),
        Err(_) => setup_failed(b"P5.1: production user-entry handoff failed validation.\r\n"),
    }
}

#[cfg(not(feature = "user-task-lifecycle-smoke-test"))]
fn build_user_code(mode: Ring3TestMode) -> ([u8; 64], usize, u64, u64, u64, u64, u64) {
    // Each scenario first writes a sentinel through the actual CPL3 user
    // stack. P5.1 executes INT3, P5.2 faults on a supervisor-only read,
    // and P5.3 executes SYSCALL/SYSRET before the final INT3.
    let mut code = [0u8; 64];
    code[..10].copy_from_slice(&[0x48, 0xC7, 0x44, 0x24, 0xF8, 0x8D, 0x7C, 0x6B, 0x5A, 0x90]);
    match mode {
        Ring3TestMode::Breakpoint => {
            code[9] = 0xCC;
            (code, 10, 3, u64::MAX, 0, USER_CODE_LENGTH, 0)
        }
        Ring3TestMode::SupervisorFault => {
            let address = ptr::addr_of!(RING3_SUPERVISOR_PROBE) as u64;
            code[10] = 0x48;
            code[11] = 0xB8; // movabs rax, imm64
            code[12..20].copy_from_slice(&address.to_le_bytes());
            code[20..23].copy_from_slice(&[0x48, 0x8B, 0x00]); // mov rax, [rax]
            code[23] = 0xCC; // Should be unreachable if supervisor protection works.
            (code, 24, 14, 0x7, 0x5, 20, address)
        }
        #[cfg(feature = "user-pointer-range-smoke-test")]
        Ring3TestMode::PointerValidation => {
            code[9] = 0xCC;
            (code, 10, 3, u64::MAX, 0, USER_CODE_LENGTH, 0)
        }
        #[cfg(feature = "user-copy-smoke-test")]
        Ring3TestMode::UserCopy => {
            code[9] = 0xCC;
            (code, 10, 3, u64::MAX, 0, USER_CODE_LENGTH, 0)
        }
        Ring3TestMode::Syscall => {
            let mut position = 10;
            code[position..position + 5].copy_from_slice(&[0xB8, 1, 0, 0, 0]); // mov eax, 1
            position += 5;
            code[position..position + 5].copy_from_slice(&[0xBF, 0x11, 0, 0, 0]); // mov edi, 0x11
            position += 5;
            code[position..position + 5].copy_from_slice(&[0xBE, 0x22, 0, 0, 0]); // mov esi, 0x22
            position += 5;
            code[position..position + 5].copy_from_slice(&[0xBA, 0x33, 0, 0, 0]); // mov edx, 0x33
            position += 5;
            code[position..position + 6].copy_from_slice(&[0x41, 0xBA, 0x44, 0, 0, 0]); // mov r10d, 0x44
            position += 6;
            code[position..position + 6].copy_from_slice(&[0x41, 0xB8, 0x55, 0, 0, 0]); // mov r8d, 0x55
            position += 6;
            code[position..position + 2].copy_from_slice(&[0x0F, 0x05]); // syscall
            position += 2;
            code[position..position + 5].copy_from_slice(&[0x48, 0x89, 0x44, 0x24, 0xF0]); // store result at RSP-16
            position += 5;
            code[position] = 0xCC;
            position += 1;
            (code, position, 3, u64::MAX, 0, position as u64, 0)
        }
    }
}

/// Called for an exception that originated at CPL3. The interrupt stub passes
/// a pointer to the normalized vector/error-code prefix; hardware also saved
/// the old user RSP and SS because this was a privilege-level transition.
pub fn handle_user_exception(
    vector: u64,
    error_code: u64,
    instruction_pointer: u64,
    code_selector: u64,
    flags: u64,
    exception_stack_pointer: u64,
) -> ! {
    let mode = match unsafe { ptr::read_volatile(ptr::addr_of!(RING3_TEST_MODE)) } {
        2 => Ring3TestMode::SupervisorFault,
        3 => Ring3TestMode::Syscall,
        _ => Ring3TestMode::Breakpoint,
    };
    serial::COM1.write(match mode {
        Ring3TestMode::Breakpoint => b"P5.1: exception returned from user mode; vector=",
        Ring3TestMode::SupervisorFault => b"P5.2: exception returned from user mode; vector=",
        Ring3TestMode::Syscall => b"P5.3: exception returned from user mode after SYSCALL; vector=",
        #[cfg(feature = "user-pointer-range-smoke-test")]
        Ring3TestMode::PointerValidation => {
            b"P5.4: unexpected exception during pointer validation; vector="
        }
        #[cfg(feature = "user-copy-smoke-test")]
        Ring3TestMode::UserCopy => b"P5.5: unexpected exception during user-copy test; vector=",
    });
    serial::COM1.write_hex(vector);
    serial::COM1.write(b" CPL=");
    serial::COM1.write_hex(code_selector & 3);
    serial::COM1.write(b"\r\n");

    let kernel_stack_base = ptr::addr_of!(RING3_KERNEL_STACK) as u64;
    let kernel_stack_top = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_KERNEL_STACK_TOP)) };
    let frame_bytes = 7 * size_of::<u64>();
    let frame_end = (exception_stack_pointer as u64).checked_add(frame_bytes as u64);
    let frame_in_bounds = exception_stack_pointer != 0
        && (exception_stack_pointer as u64) >= kernel_stack_base + PAGE_SIZE
        && frame_end.is_some_and(|end| end <= kernel_stack_top);

    let frame = if frame_in_bounds {
        unsafe { core::slice::from_raw_parts(exception_stack_pointer as *const u64, 7) }
    } else {
        &[]
    };

    let stack_shape_valid = frame.len() == 7
        && frame[0] == vector
        && frame[1] == error_code
        && frame[2] == instruction_pointer
        && frame[3] == code_selector
        && frame[4] == flags;
    let saved_user_rsp = frame.get(5).copied().unwrap_or(0);
    let saved_user_ss = frame.get(6).copied().unwrap_or(0);

    let space = unsafe { &*ptr::addr_of!(RING3_ADDRESS_SPACE) };
    let mapping_checks = space.as_ref().is_some_and(|space| {
        let code = space.translate(USER_CODE_ADDRESS);
        let guard = space.translate(USER_CODE_ADDRESS + PAGE_SIZE);
        let stack = space.translate(USER_STACK_ADDRESS);
        code.is_ok_and(|mapping| mapping.user && mapping.executable && !mapping.writable)
            && guard.is_err()
            && stack.is_ok_and(|mapping| mapping.user && !mapping.executable && mapping.writable)
    });

    let expected_vector = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_VECTOR)) };
    let expected_error_mask =
        unsafe { ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_ERROR_MASK)) };
    let expected_error_value =
        unsafe { ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_ERROR_VALUE)) };
    let expected_rip = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_RIP)) };
    let expected_fault_address =
        unsafe { ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_FAULT_ADDRESS)) };
    let fault_address = if vector == 14 {
        let address: u64;
        unsafe {
            asm!("mov {}, cr2", out(reg) address, options(nomem, nostack, preserves_flags));
        }
        address
    } else {
        0
    };
    let fault_address_valid = if expected_vector == 14 {
        fault_address == expected_fault_address
    } else {
        expected_fault_address == 0 && vector == 3
    };
    let user_stack_write_valid = unsafe {
        ptr::read_volatile(
            (ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME)) + PAGE_SIZE - 16) as *const u64,
        ) == USER_STACK_SENTINEL
    };
    let syscall_result_valid = if mode == Ring3TestMode::Syscall {
        unsafe {
            ptr::read_volatile(
                (ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME)) + PAGE_SIZE - 24)
                    as *const u64,
            ) == 0xC0DE_CAFE_F00D_BEEFu64
                && ptr::read_volatile(ptr::addr_of!(RING3_SYSCALL_VALID)) == 1
                && ptr::read_volatile(ptr::addr_of!(RING3_SYSCALL_CALLS)) == 1
        }
    } else {
        true
    };

    let passed = vector == expected_vector
        && error_code & expected_error_mask == expected_error_value
        && code_selector == u64::from(gdt::USER_CODE_SELECTOR)
        && code_selector & 3 == 3
        && instruction_pointer == expected_rip
        && flags & (1 << 9) != 0
        && frame_in_bounds
        && stack_shape_valid
        && saved_user_rsp == USER_STACK_POINTER
        && saved_user_ss == u64::from(gdt::USER_DATA_SELECTOR)
        && mapping_checks
        && user_stack_write_valid
        && syscall_result_valid
        && fault_address_valid;

    serial::COM1.write(match mode {
        Ring3TestMode::Breakpoint => b"P5.1: saved user state RIP/RSP/SS verified.\r\n",
        Ring3TestMode::SupervisorFault => {
            b"P5.2: user supervisor-access fault RIP/RSP/SS verified.\r\n"
        }
        Ring3TestMode::Syscall => {
            b"P5.3: syscall returned to CPL3; return value and user frame verified.\r\n"
        }
        #[cfg(feature = "user-pointer-range-smoke-test")]
        Ring3TestMode::PointerValidation => b"P5.4: pointer-range verification completed.\r\n",
        #[cfg(feature = "user-copy-smoke-test")]
        Ring3TestMode::UserCopy => b"P5.5: user-copy round-trip verified.\r\n",
    });
    serial::COM1.write(b"Ring 3: CR2=0x");
    serial::COM1.write_hex(fault_address);
    serial::COM1.write(b" error=0x");
    serial::COM1.write_hex(error_code);
    serial::COM1.write(b" user stack write=yes guard unmapped=yes.\r\n");

    let cleanup_ok = reclaim_user_address_space();
    let rsp0_cleared = gdt::set_rsp0(0) && gdt::read_rsp0() == 0;
    finish(passed && cleanup_ok && rsp0_cleared)
}

#[cfg(feature = "ring3-syscall-smoke-test")]
fn initialize_syscall_msrs() -> bool {
    let kernel_stack_top = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_KERNEL_STACK_TOP)) };
    let fx_state = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_FX_STATE)) };
    syscall::initialize_cpu_local(
        kernel_stack_top,
        fx_state,
        exhi_syscall_smoke_dispatch as *const () as usize,
    ) && syscall::initialize(syscall::entry_address()).is_ok()
}

#[cfg(feature = "ring3-syscall-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_syscall_handler(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> u64 {
    let valid =
        number == 1 && arg1 == 0x11 && arg2 == 0x22 && arg3 == 0x33 && arg4 == 0x44 && arg5 == 0x55;
    unsafe {
        let calls = ptr::addr_of_mut!(RING3_SYSCALL_CALLS);
        ptr::write_volatile(calls, ptr::read_volatile(calls).saturating_add(1));
        ptr::write_volatile(ptr::addr_of_mut!(RING3_SYSCALL_VALID), u64::from(valid));
    }
    if valid {
        0xC0DE_CAFE_F00D_BEEFu64
    } else {
        0xBAD0_BAD0_BAD0_BAD0u64
    }
}

#[cfg(feature = "ring3-syscall-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_syscall_smoke_dispatch(frame: *mut UserTaskInterruptFrame) -> usize {
    if frame.is_null() {
        return 0;
    }
    let number = unsafe { ptr::read_volatile(ptr::addr_of!((*frame).rax)) };
    let arg1 = unsafe { ptr::read_volatile(ptr::addr_of!((*frame).rdi)) };
    let arg2 = unsafe { ptr::read_volatile(ptr::addr_of!((*frame).rsi)) };
    let arg3 = unsafe { ptr::read_volatile(ptr::addr_of!((*frame).rdx)) };
    let arg4 = unsafe { ptr::read_volatile(ptr::addr_of!((*frame).r10)) };
    let arg5 = unsafe { ptr::read_volatile(ptr::addr_of!((*frame).r8)) };
    let result = exhi_syscall_handler(number, arg1, arg2, arg3, arg4, arg5);
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!((*frame).rax), result);
    }
    frame as usize
}

#[cfg(feature = "user-pointer-range-smoke-test")]
const MAX_USER_POINTER_RANGE: u64 = 16 * 1024 * 1024;

#[cfg(feature = "user-pointer-range-smoke-test")]
fn validate_user_pointer_ranges(space: &AddressSpace) -> bool {
    // This validator is called before any raw user pointer is dereferenced.
    let code_read = validate_user_range(space, USER_CODE_ADDRESS, 10, UserAccess::Read).is_ok();
    let code_execute =
        validate_user_range(space, USER_CODE_ADDRESS, 10, UserAccess::Execute).is_ok();
    let code_write = validate_user_range(space, USER_CODE_ADDRESS, 1, UserAccess::Write)
        == Err(UserRangeError::PermissionDenied);
    let stack_read = validate_user_range(space, USER_STACK_ADDRESS, 32, UserAccess::Read).is_ok();
    let stack_write =
        validate_user_range(space, USER_STACK_TOP - 32, 32, UserAccess::Write).is_ok();
    let stack_execute = validate_user_range(space, USER_STACK_ADDRESS, 1, UserAccess::Execute)
        == Err(UserRangeError::PermissionDenied);
    let guard_rejected = matches!(
        validate_user_range(space, USER_CODE_ADDRESS + PAGE_SIZE, 1, UserAccess::Read,),
        Err(UserRangeError::AddressSpace(_))
    );
    let crossing_guard_rejected = matches!(
        validate_user_range(
            space,
            USER_CODE_ADDRESS + PAGE_SIZE - 8,
            16,
            UserAccess::Read,
        ),
        Err(UserRangeError::AddressSpace(_))
    );
    let supervisor_rejected = validate_user_range(
        space,
        ptr::addr_of!(RING3_SUPERVISOR_PROBE) as u64,
        size_of::<u64>(),
        UserAccess::Read,
    ) == Err(UserRangeError::NotUserMemory);
    let zero_length = validate_user_range(space, 0, 0, UserAccess::Read).is_ok();
    let overflow = validate_user_range(space, u64::MAX - 3, 8, UserAccess::Read)
        == Err(UserRangeError::AddressOverflow);
    let too_large = validate_user_range(
        space,
        USER_STACK_ADDRESS,
        (MAX_USER_POINTER_RANGE + 1) as usize,
        UserAccess::Read,
    ) == Err(UserRangeError::RangeTooLarge);

    code_read
        && code_execute
        && code_write
        && stack_read
        && stack_write
        && stack_execute
        && guard_rejected
        && crossing_guard_rejected
        && supervisor_rejected
        && zero_length
        && overflow
        && too_large
}

#[cfg(feature = "user-copy-smoke-test")]
fn validate_user_copy_runtime(space: &AddressSpace) -> bool {
    const PATTERN: [u8; 16] = [
        0x45, 0x58, 0x48, 0x49, 0x4F, 0x53, 0x2D, 0x55, 0x53, 0x45, 0x52, 0x2D, 0x43, 0x4F, 0x50,
        0x59,
    ];
    let mut received = [0u8; 16];
    let destination = USER_STACK_TOP - 64;

    if copy_to_user(space, destination, &PATTERN).is_err()
        || copy_from_user(space, destination, &mut received).is_err()
        || received != PATTERN
    {
        return false;
    }

    // The destination starts four bytes before the end of the writable
    // mapping and extends four bytes into the unmapped page. No prefix may
    // be copied before the whole range has been rejected.
    let boundary = USER_STACK_TOP - 4;
    let stack_frame = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME)) };
    let tail = (stack_frame + PAGE_SIZE - 4) as *const u32;
    let before = unsafe { ptr::read_volatile(tail) };
    let invalid_source = [0xA5u8; 8];
    let write_rejected = matches!(
        copy_to_user(space, boundary, &invalid_source),
        Err(UserRangeError::AddressSpace(_))
    );
    let after = unsafe { ptr::read_volatile(tail) };

    let mut invalid_destination = [0u8; 8];
    let read_rejected = matches!(
        copy_from_user(space, boundary, &mut invalid_destination),
        Err(UserRangeError::AddressSpace(_))
    );

    write_rejected && read_rejected && before == after
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
pub fn user_task_lifecycle_smoke_test() -> ! {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
        ptr::write(ptr::addr_of_mut!(USER_TASK_SCHEDULER), Scheduler::new());
        ptr::write(ptr::addr_of_mut!(USER_TASK_SPACE_A), None);
        ptr::write(ptr::addr_of_mut!(USER_TASK_SPACE_B), None);
        ptr::write_volatile(ptr::addr_of_mut!(USER_TASK_CODE_FRAMES), [0; 2]);
        ptr::write_volatile(ptr::addr_of_mut!(USER_TASK_STACK_FRAMES), [0; 2]);
        ptr::write_volatile(ptr::addr_of_mut!(USER_TASK_KERNEL_STACK_TOPS), [0; 3]);
        ptr::write_volatile(
            ptr::addr_of_mut!(USER_TASK_COUNTERS),
            UserTaskCounters {
                syscall_calls: 0,
                yields: 0,
                exits: 0,
                denied: 0,
                console_writes: 0,
                switches: 0,
            },
        );
    }

    serial::COM1
        .write(b"P5.6: preparing two isolated Ring 3 tasks and one kernel reaper task.\r\n");
    let Some(template_fx) = task::initialize_fx_state_for_user_mode() else {
        user_task_setup_failed(b"P5.6: FXSR/SSE context initialization failed.\r\n");
    };
    unsafe {
        for index in 0..3 {
            ptr::copy_nonoverlapping(
                template_fx as *const u8,
                ptr::addr_of_mut!(USER_TASK_FX_STATES[index]) as *mut u8,
                512,
            );
        }
    }

    let mut kernel_tops = [0u64; 3];
    for (index, top_slot) in kernel_tops.iter_mut().enumerate() {
        let base = unsafe { ptr::addr_of_mut!(USER_TASK_KERNEL_STACKS[index]) as u64 };
        let Some(top) = base.checked_add(STACK_TOTAL_SIZE as u64) else {
            user_task_setup_failed(b"P5.6: kernel stack address overflow.\r\n");
        };
        if base % PAGE_SIZE != 0 || top % 16 != 0 || top >= 0x1_0000_0000 {
            user_task_setup_failed(b"P5.6: invalid per-task kernel stack bounds.\r\n");
        }
        if vmm::unmap_page(base).ok() != Some(base)
            || super::paging::translate_current(base) != Err(super::paging::PagingError::NotMapped)
        {
            user_task_setup_failed(b"P5.6: could not install a per-task kernel stack guard.\r\n");
        }
        *top_slot = top;
    }
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(USER_TASK_KERNEL_STACK_TOPS), kernel_tops);
    }

    let Some((space_a, code_a, stack_a)) =
        create_user_task_address_space(0x211C_E001, 0x211C_E002, false)
    else {
        user_task_setup_failed(b"P5.6: user task A address space setup failed.\r\n");
    };
    let Some((space_b, code_b, stack_b)) =
        create_user_task_address_space(0x311C_E001, 0x311C_E002, true)
    else {
        user_task_setup_failed(b"P5.6: user task B address space setup failed.\r\n");
    };
    if space_a.root_address() == space_b.root_address() || code_a == code_b || stack_a == stack_b {
        user_task_setup_failed(b"P5.6: user tasks do not have private roots and data frames.\r\n");
    }
    let root_a = space_a.root_address();
    let root_b = space_b.root_address();
    unsafe {
        ptr::write(ptr::addr_of_mut!(USER_TASK_SPACE_A), Some(space_a));
        ptr::write(ptr::addr_of_mut!(USER_TASK_SPACE_B), Some(space_b));
        ptr::write_volatile(ptr::addr_of_mut!(USER_TASK_CODE_FRAMES), [code_a, code_b]);
        ptr::write_volatile(
            ptr::addr_of_mut!(USER_TASK_STACK_FRAMES),
            [stack_a, stack_b],
        );
    }

    let frame_a = prepare_user_task_initial_frame(0, kernel_tops[0]);
    let frame_b = prepare_user_task_initial_frame(1, kernel_tops[1]);
    let Some(frame_a) = frame_a else {
        user_task_setup_failed(b"P5.6: user task A initial context failed.\r\n");
    };
    let Some(frame_b) = frame_b else {
        user_task_setup_failed(b"P5.6: user task B initial context failed.\r\n");
    };
    let Some(kernel_frame) = prepare_user_task_kernel_frame(kernel_tops[2]) else {
        user_task_setup_failed(b"P5.6: kernel reaper initial context failed.\r\n");
    };

    let scheduler = ptr::addr_of_mut!(USER_TASK_SCHEDULER);
    let id_a = unsafe {
        (*scheduler)
            .spawn_user(
                "ring3-A",
                TaskContext::new(frame_a),
                UserTaskMetadata::new_with_capabilities(
                    root_a,
                    USER_STACK_POINTER,
                    kernel_tops[0],
                    ptr::addr_of!(USER_TASK_FX_STATES[0]) as usize,
                    CapabilitySet::empty()
                        .with(Capability::TaskYield)
                        .with(Capability::TaskExit)
                        .with(Capability::ConsoleWrite),
                ),
            )
            .unwrap_or_else(|_| {
                user_task_setup_failed(b"P5.6: could not register user task A.\r\n")
            })
    };
    let id_b = unsafe {
        (*scheduler)
            .spawn_user(
                "ring3-B",
                TaskContext::new(frame_b),
                UserTaskMetadata::new_with_capabilities(
                    root_b,
                    USER_STACK_POINTER,
                    kernel_tops[1],
                    ptr::addr_of!(USER_TASK_FX_STATES[1]) as usize,
                    CapabilitySet::empty().with(Capability::TaskExit),
                ),
            )
            .unwrap_or_else(|_| {
                user_task_setup_failed(b"P5.6: could not register user task B.\r\n")
            })
    };
    let id_kernel = unsafe {
        (*scheduler)
            .spawn("user-task-reaper", TaskContext::new(kernel_frame))
            .unwrap_or_else(|_| {
                user_task_setup_failed(b"P5.6: could not register the kernel reaper.\r\n")
            })
    };
    if id_a.get() != 1
        || id_b.get() != 2
        || id_kernel.get() != 3
        || unsafe { (*scheduler).block_ready(id_kernel).is_err() }
        || unsafe { (*scheduler).start() } != Ok(id_a)
    {
        user_task_setup_failed(b"P5.6: scheduler did not select user task A first.\r\n");
    }

    let initial_fx_state = unsafe { ptr::addr_of!(USER_TASK_FX_STATES[0]) as usize };
    if !syscall::initialize_smoke_test_cpu_local(kernel_tops[0], initial_fx_state) {
        user_task_setup_failed(b"P5.6: boot CPU syscall-local setup failed.\r\n");
    }
    if !syscall::initialize(syscall::entry_address()).is_ok() {
        user_task_setup_failed(b"P5.6: syscall entry MSR setup failed.\r\n");
    }
    let space_slot = unsafe { &*ptr::addr_of!(USER_TASK_SPACE_A) };
    if !space_slot
        .as_ref()
        .is_some_and(|space| space.activate().is_ok())
        || !gdt::set_rsp0(kernel_tops[0])
    {
        user_task_setup_failed(b"P5.6: could not activate user task A and its kernel stack.\r\n");
    }

    serial::COM1.write(
        b"P5.6: private CR3 roots, RX code, RW+NX stacks and guarded kernel stacks ready.\r\n",
    );
    serial::COM1.write(b"P5.6: A/B each yield twice and then exit through SYSCALL; kernel task reclaims later.\r\n");
    unsafe {
        exhi_interrupt_context_start(frame_a, ptr::addr_of!(USER_TASK_FX_STATES[0]) as usize);
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn user_task_space_a_root() -> u64 {
    unsafe {
        (&*ptr::addr_of!(USER_TASK_SPACE_A))
            .as_ref()
            .map(AddressSpace::root_address)
            .unwrap_or(0)
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn user_task_space_b_root() -> u64 {
    unsafe {
        (&*ptr::addr_of!(USER_TASK_SPACE_B))
            .as_ref()
            .map(AddressSpace::root_address)
            .unwrap_or(0)
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn create_user_task_address_space(
    initial_marker: u32,
    resumed_marker: u32,
    expect_yield_denied: bool,
) -> Option<(AddressSpace, u64, u64)> {
    let mut space = AddressSpace::create().ok()?;
    let code_frame = memory::allocate_frame().ok()?;
    let stack_frame = match memory::allocate_frame() {
        Ok(frame) => frame,
        Err(_) => {
            let _ = memory::release_frame(code_frame);
            return None;
        }
    };
    if code_frame == 0
        || stack_frame == 0
        || code_frame % PAGE_SIZE != 0
        || stack_frame % PAGE_SIZE != 0
        || code_frame >= 0x1_0000_0000
        || stack_frame >= 0x1_0000_0000
    {
        let _ = memory::release_frame(code_frame);
        let _ = memory::release_frame(stack_frame);
        return None;
    }
    unsafe {
        ptr::write_bytes(code_frame as *mut u8, 0, PAGE_SIZE as usize);
        ptr::write_bytes(stack_frame as *mut u8, 0, PAGE_SIZE as usize);
    }
    let (code, length) =
        build_user_task_program(initial_marker, resumed_marker, expect_yield_denied);
    unsafe {
        ptr::copy_nonoverlapping(code.as_ptr(), code_frame as *mut u8, length);
    }
    let code_mapped = unsafe {
        space.map_user_page(
            USER_CODE_ADDRESS,
            code_frame,
            UserPageFlags {
                writable: false,
                executable: true,
            },
        )
    }
    .is_ok();
    if !code_mapped {
        let _ = space.destroy();
        let _ = memory::release_frame(code_frame);
        let _ = memory::release_frame(stack_frame);
        return None;
    }
    let stack_mapped = unsafe {
        space.map_user_page(
            USER_STACK_ADDRESS,
            stack_frame,
            UserPageFlags::read_write_nx(),
        )
    }
    .is_ok();
    if !stack_mapped {
        let _ = space.unmap_user_page(USER_CODE_ADDRESS);
        let _ = space.destroy();
        let _ = memory::release_frame(code_frame);
        let _ = memory::release_frame(stack_frame);
        return None;
    }
    let code_ok = space
        .translate(USER_CODE_ADDRESS)
        .is_ok_and(|map| map.user && map.executable && !map.writable);
    let stack_ok = space
        .translate(USER_STACK_ADDRESS)
        .is_ok_and(|map| map.user && map.writable && !map.executable);
    if !code_ok || !stack_ok || space.translate(USER_CODE_ADDRESS + PAGE_SIZE).is_ok() {
        let _ = space.unmap_user_page(USER_CODE_ADDRESS);
        let _ = space.unmap_user_page(USER_STACK_ADDRESS);
        let _ = space.destroy();
        let _ = memory::release_frame(code_frame);
        let _ = memory::release_frame(stack_frame);
        return None;
    }
    Some((space, code_frame, stack_frame))
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn build_user_task_program(
    initial_marker: u32,
    resumed_marker: u32,
    expect_yield_denied: bool,
) -> ([u8; 256], usize) {
    const CONSOLE_MESSAGE_OFFSET: usize = 128;
    let mut code = [0u8; 256];
    let mut position = 0;
    code[position..position + 9].copy_from_slice(&[
        0x48,
        0xC7,
        0x44,
        0x24,
        0xF8,
        initial_marker as u8,
        (initial_marker >> 8) as u8,
        (initial_marker >> 16) as u8,
        (initial_marker >> 24) as u8,
    ]);
    position += 9;

    let yield_number = SyscallNumber::Yield as u32;
    code[position..position + 5].copy_from_slice(&[
        0xB8,
        yield_number as u8,
        (yield_number >> 8) as u8,
        (yield_number >> 16) as u8,
        (yield_number >> 24) as u8,
    ]);
    position += 5;
    code[position..position + 2].copy_from_slice(&[0x0F, 0x05]);
    position += 2;
    if expect_yield_denied {
        // B has no TaskYield grant. Trap unless the denial is returned as -1.
        code[position..position + 6].copy_from_slice(&[0x48, 0x83, 0xF8, 0xFF, 0x74, 0x01]);
        position += 6;
        code[position] = 0xCC;
        position += 1;
    }

    let message: &[u8] = if expect_yield_denied {
        b"P5.7: user B unauthorized output must not appear\r\n"
    } else {
        b"P5.7: user A authorized ConsoleWrite reached COM1\r\n"
    };
    let message_address = USER_CODE_ADDRESS + CONSOLE_MESSAGE_OFFSET as u64;
    code[position..position + 2].copy_from_slice(&[0x48, 0xBF]); // movabs rdi, imm64
    position += 2;
    code[position..position + 8].copy_from_slice(&message_address.to_le_bytes());
    position += 8;
    let message_length = message.len() as u32;
    code[position..position + 5].copy_from_slice(&[
        0xBE,
        message_length as u8,
        (message_length >> 8) as u8,
        (message_length >> 16) as u8,
        (message_length >> 24) as u8,
    ]); // mov esi, length
    position += 5;
    let console_number = SyscallNumber::ConsoleWrite as u32;
    code[position..position + 5].copy_from_slice(&[
        0xB8,
        console_number as u8,
        (console_number >> 8) as u8,
        (console_number >> 16) as u8,
        (console_number >> 24) as u8,
    ]);
    position += 5;
    code[position..position + 2].copy_from_slice(&[0x0F, 0x05]);
    position += 2;

    if expect_yield_denied {
        // The denied ConsoleWrite must not consume or print B's user buffer.
        code[position..position + 6].copy_from_slice(&[0x48, 0x83, 0xF8, 0xFF, 0x74, 0x01]);
        position += 6;
        code[position] = 0xCC;
        position += 1;
    } else {
        // A must receive the exact number of bytes actually written.
        code[position..position + 6].copy_from_slice(&[
            0x48,
            0x3D,
            message_length as u8,
            (message_length >> 8) as u8,
            (message_length >> 16) as u8,
            (message_length >> 24) as u8,
        ]);
        position += 6;
        code[position..position + 3].copy_from_slice(&[0x74, 0x01, 0xCC]);
        position += 3;
    }

    code[position..position + 9].copy_from_slice(&[
        0x48,
        0xC7,
        0x44,
        0x24,
        0xF0,
        resumed_marker as u8,
        (resumed_marker >> 8) as u8,
        (resumed_marker >> 16) as u8,
        (resumed_marker >> 24) as u8,
    ]);
    position += 9;
    let exit_number = SyscallNumber::Exit as u32;
    code[position..position + 5].copy_from_slice(&[
        0xB8,
        exit_number as u8,
        (exit_number >> 8) as u8,
        (exit_number >> 16) as u8,
        (exit_number >> 24) as u8,
    ]);
    position += 5;
    code[position..position + 2].copy_from_slice(&[0x0F, 0x05]);
    position += 2;
    code[position] = 0xCC; // Must be unreachable after SYS_EXIT.
    position += 1;

    code[CONSOLE_MESSAGE_OFFSET..CONSOLE_MESSAGE_OFFSET + message.len()].copy_from_slice(message);
    let code_length = core::cmp::max(position, CONSOLE_MESSAGE_OFFSET + message.len());
    (code, code_length)
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn prepare_user_task_initial_frame(index: usize, kernel_stack_top: u64) -> Option<usize> {
    if index >= 2 {
        return None;
    }
    let rsp = (kernel_stack_top as usize).checked_sub(size_of::<UserTaskInterruptFrame>())?;
    if rsp % 16 != 0 {
        return None;
    }
    let frame = UserTaskInterruptFrame {
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
        rip: USER_CODE_ADDRESS,
        cs: u64::from(gdt::USER_CODE_SELECTOR),
        rflags: 0x2,
        rsp: USER_STACK_POINTER,
        ss: u64::from(gdt::USER_DATA_SELECTOR),
    };
    unsafe {
        ptr::write(rsp as *mut UserTaskInterruptFrame, frame);
    }
    Some(rsp)
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn prepare_user_task_kernel_frame(kernel_stack_top: u64) -> Option<usize> {
    let rsp = (kernel_stack_top as usize).checked_sub(size_of::<UserTaskInterruptFrame>())?;
    if rsp % 16 != 0 {
        return None;
    }
    let frame = UserTaskInterruptFrame {
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
        rip: exhi_user_task_kernel_entry as *const () as usize as u64,
        cs: u64::from(gdt::KERNEL_CODE_SELECTOR),
        rflags: 0x2,
        rsp: kernel_stack_top,
        ss: u64::from(gdt::KERNEL_DATA_SELECTOR),
    };
    unsafe {
        ptr::write(rsp as *mut UserTaskInterruptFrame, frame);
    }
    Some(rsp)
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn user_task_stack_bounds(index: usize) -> Option<(u64, u64)> {
    if index >= 3 {
        return None;
    }
    let base = unsafe { ptr::addr_of!(USER_TASK_KERNEL_STACKS[index]) as u64 };
    // The top is immutable geometry, not task state. Derive it from the
    // static stack object rather than trusting a separately mutable copy.
    let top = base.checked_add(STACK_TOTAL_SIZE as u64)?;
    Some((base, top))
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn user_task_select_context(id: TaskId) -> usize {
    let scheduler = ptr::addr_of_mut!(USER_TASK_SCHEDULER);
    let index = usize::try_from(id.get())
        .ok()
        .and_then(|raw| raw.checked_sub(1))
        .filter(|index| *index < 3)
        .unwrap_or_else(|| {
            user_task_runtime_failed(b"P5.6: scheduler selected an unknown task id.\r\n")
        });
    let current = unsafe { (*scheduler).get(id) }.unwrap_or_else(|| {
        user_task_runtime_failed(b"P5.6: scheduler selected a missing task.\r\n")
    });
    let frame = current.context().saved_stack_pointer();
    let Some((base, top)) = user_task_stack_bounds(index) else {
        user_task_runtime_failed(b"P5.6: selected task stack metadata missing.\r\n");
    };
    let end = (frame as u64).checked_add(size_of::<UserTaskInterruptFrame>() as u64);
    if frame == 0 || (frame as u64) < base + PAGE_SIZE || end.map_or(true, |value| value > top) {
        user_task_runtime_failed(b"P5.6: selected task interrupt frame is out of bounds.\r\n");
    }

    if let Some(metadata) = current.user_metadata() {
        let expected_root = match index {
            0 => user_task_space_a_root(),
            1 => user_task_space_b_root(),
            _ => 0,
        };
        if expected_root == 0
            || metadata.address_space_root() != expected_root
            || metadata.kernel_stack_top() != top
            || metadata.fx_state_pointer()
                != unsafe { ptr::addr_of!(USER_TASK_FX_STATES[index]) as usize }
        {
            user_task_runtime_failed(
                b"P5.6: user task metadata does not match owned resources.\r\n",
            );
        }
        let space_ok = match index {
            0 => unsafe { (&*ptr::addr_of!(USER_TASK_SPACE_A)).as_ref() }
                .is_some_and(|space| space.activate().is_ok()),
            1 => unsafe { (&*ptr::addr_of!(USER_TASK_SPACE_B)).as_ref() }
                .is_some_and(|space| space.activate().is_ok()),
            _ => false,
        };
        if !space_ok || !gdt::set_rsp0(top) {
            user_task_runtime_failed(
                b"P5.6: failed to activate the selected user address space.\r\n",
            );
        }
        if !syscall::update_smoke_test_task_context(top, metadata.fx_state_pointer()) {
            user_task_runtime_failed(
                b"P5.6: failed to publish the selected task syscall context.\r\n",
            );
        }
    } else {
        if index != 2 || address_space::restore_kernel().is_err() || !gdt::set_rsp0(top) {
            user_task_runtime_failed(b"P5.6: failed to switch to the kernel reaper context.\r\n");
        }
        let reaper_fx_state = unsafe { ptr::addr_of!(USER_TASK_FX_STATES[2]) as usize };
        if !syscall::update_smoke_test_task_context(top, reaper_fx_state) {
            user_task_runtime_failed(
                b"P5.6: failed to publish the kernel reaper syscall context.\r\n",
            );
        }
    }

    unsafe {
        let count = ptr::addr_of_mut!(USER_TASK_COUNTERS.switches);
        ptr::write_volatile(count, ptr::read_volatile(count).saturating_add(1));
    }
    frame
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
struct UserTaskSyscallServices {
    scheduler: *mut Scheduler<3>,
    task_index: usize,
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
impl SyscallServices for UserTaskSyscallServices {
    type SwitchTarget = TaskId;

    fn current_capabilities(&self) -> CapabilitySet {
        if self.scheduler.is_null() {
            return CapabilitySet::EMPTY;
        }
        // SAFETY: The adapter is constructed only inside the syscall entry
        // with a pointer to the static scheduler; interrupts remain disabled.
        unsafe { (*self.scheduler).current_capabilities() }
    }

    fn yield_current(&mut self) -> Result<Option<Self::SwitchTarget>, SyscallError> {
        if self.scheduler.is_null() {
            return Err(SyscallError::Io);
        }
        // SAFETY: This method runs on the dedicated kernel stack with IF clear.
        let decision =
            unsafe { (*self.scheduler).yield_current() }.map_err(|_| SyscallError::Io)?;
        unsafe {
            let count = ptr::addr_of_mut!(USER_TASK_COUNTERS.yields);
            ptr::write_volatile(count, ptr::read_volatile(count).saturating_add(1));
        }
        Ok(match decision {
            exhi_task::ScheduleDecision::Switch { to, .. } => Some(to),
            exhi_task::ScheduleDecision::Continue { .. } => None,
        })
    }

    fn exit_current(&mut self) -> Result<Self::SwitchTarget, SyscallError> {
        if self.scheduler.is_null() {
            return Err(SyscallError::Io);
        }
        // SAFETY: The caller has validated the active task and IF is clear.
        let next = unsafe { (*self.scheduler).finish_current() }.map_err(|_| SyscallError::Io)?;
        let target = if let Some(next) = next {
            next
        } else {
            let reaper = TaskId::from_raw(3);
            unsafe {
                (*self.scheduler)
                    .wake(reaper)
                    .map_err(|_| SyscallError::Io)?;
                (*self.scheduler)
                    .schedule_from_idle()
                    .map_err(|_| SyscallError::Io)?
                    .ok_or(SyscallError::Io)?
            }
        };
        unsafe {
            let count = ptr::addr_of_mut!(USER_TASK_COUNTERS.exits);
            ptr::write_volatile(count, ptr::read_volatile(count).saturating_add(1));
        }
        Ok(target)
    }

    fn console_write(&mut self, user_address: u64, length: u64) -> Result<u64, SyscallError> {
        user_task_console_write(self.task_index, user_address, length)
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_user_task_syscall_handler(frame: *mut UserTaskInterruptFrame) -> usize {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
    let frame_address = frame as usize;
    let scheduler = ptr::addr_of_mut!(USER_TASK_SCHEDULER);
    let current_id = unsafe { (*scheduler).current().map(|task| task.id()) }.unwrap_or_else(|| {
        user_task_runtime_failed(b"P5.6: syscall arrived without a current task.\r\n")
    });
    let current_task = unsafe { (*scheduler).get(current_id) }.unwrap_or_else(|| {
        user_task_runtime_failed(b"P5.6: current user task metadata missing.\r\n")
    });
    let Some(metadata) = current_task.user_metadata() else {
        user_task_runtime_failed(b"P5.6: kernel reaper unexpectedly invoked from CPL3.\r\n");
    };
    let index = usize::try_from(current_id.get())
        .ok()
        .and_then(|raw| raw.checked_sub(1))
        .filter(|index| *index < 2)
        .unwrap_or_else(|| {
            user_task_runtime_failed(b"P5.6: syscall came from a non-user task.\r\n")
        });
    let Some((base, top)) = user_task_stack_bounds(index) else {
        user_task_runtime_failed(b"P5.6: syscall task stack bounds missing.\r\n");
    };
    let end = (frame_address as u64).checked_add(size_of::<UserTaskInterruptFrame>() as u64);
    if frame_address == 0
        || (frame_address as u64) < base + PAGE_SIZE
        || end.map_or(true, |value| value > top)
        || metadata.kernel_stack_top() != top
    {
        user_task_runtime_failed(b"P5.6: syscall frame escaped the active task kernel stack.\r\n");
    }

    let request = unsafe {
        SyscallRequest::from_registers(
            ptr::read_volatile(ptr::addr_of!((*frame).rax)),
            ptr::read_volatile(ptr::addr_of!((*frame).rdi)),
            ptr::read_volatile(ptr::addr_of!((*frame).rsi)),
            ptr::read_volatile(ptr::addr_of!((*frame).rdx)),
            ptr::read_volatile(ptr::addr_of!((*frame).r10)),
            ptr::read_volatile(ptr::addr_of!((*frame).r8)),
            ptr::read_volatile(ptr::addr_of!((*frame).r9)),
        )
    };
    unsafe {
        (*scheduler)
            .get_mut(current_id)
            .unwrap()
            .context_mut()
            .set_saved_stack_pointer(frame_address);
        let user_rsp = ptr::read_volatile(ptr::addr_of!((*frame).rsp));
        (*scheduler)
            .get_mut(current_id)
            .unwrap()
            .user_metadata_mut()
            .unwrap()
            .set_user_stack_pointer(user_rsp);
        let count = ptr::addr_of_mut!(USER_TASK_COUNTERS.syscall_calls);
        ptr::write_volatile(count, ptr::read_volatile(count).saturating_add(1));
    }

    let mut services = UserTaskSyscallServices {
        scheduler,
        task_index: index,
    };
    match dispatch_syscall(request, &mut services) {
        DispatchOutcome::Return(value) => {
            if value == SyscallError::AccessDenied.as_return_value() {
                unsafe {
                    let count = ptr::addr_of_mut!(USER_TASK_COUNTERS.denied);
                    ptr::write_volatile(count, ptr::read_volatile(count).saturating_add(1));
                }
            }
            unsafe {
                ptr::write_volatile(ptr::addr_of_mut!((*frame).rax), value);
            }
            frame_address
        }
        DispatchOutcome::Switch(next) => user_task_select_context(next),
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn user_task_console_write(
    index: usize,
    user_address: u64,
    requested_length: u64,
) -> Result<u64, SyscallError> {
    const MAX_CONSOLE_WRITE: usize = 256;
    let length = usize::try_from(requested_length).map_err(|_| SyscallError::InvalidArgument)?;
    if length > MAX_CONSOLE_WRITE {
        return Err(SyscallError::InvalidArgument);
    }

    let space = match index {
        0 => unsafe { (&*ptr::addr_of!(USER_TASK_SPACE_A)).as_ref() },
        1 => unsafe { (&*ptr::addr_of!(USER_TASK_SPACE_B)).as_ref() },
        _ => None,
    };
    let Some(space) = space else {
        return Err(SyscallError::InvalidArgument);
    };

    let mut buffer = [0u8; MAX_CONSOLE_WRITE];
    copy_from_user(space, user_address, &mut buffer[..length])
        .map_err(|_| SyscallError::InvalidArgument)?;
    let written = serial::COM1.write_count(&buffer[..length]);
    if written != length {
        return Err(SyscallError::Io);
    }

    unsafe {
        let count = ptr::addr_of_mut!(USER_TASK_COUNTERS.console_writes);
        ptr::write_volatile(count, ptr::read_volatile(count).saturating_add(1));
    }
    Ok(written as u64)
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_user_task_finish() -> ! {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
    let scheduler = ptr::addr_of_mut!(USER_TASK_SCHEDULER);
    let current = unsafe { (*scheduler).current().map(|task| task.id()) };
    let tasks_ok = unsafe {
        current.map(TaskId::get) == Some(3)
            && (*scheduler)
                .get(TaskId::from_raw(1))
                .is_some_and(|task| task.state() == exhi_task::TaskState::Finished)
            && (*scheduler)
                .get(TaskId::from_raw(2))
                .is_some_and(|task| task.state() == exhi_task::TaskState::Finished)
            && (*scheduler)
                .get(TaskId::from_raw(3))
                .is_some_and(|task| task.state() == exhi_task::TaskState::Running)
            && (*scheduler).finished_count() == 2
            && (*scheduler).ready_count() == 0
            && (*scheduler).blocked_count() == 0
    };
    let capability_grants_ok = unsafe {
        (*scheduler).get(TaskId::from_raw(1)).is_some_and(|task| {
            task.capabilities().contains(Capability::TaskYield)
                && task.capabilities().contains(Capability::TaskExit)
                && task.capabilities().contains(Capability::ConsoleWrite)
        }) && (*scheduler).get(TaskId::from_raw(2)).is_some_and(|task| {
            !task.capabilities().contains(Capability::TaskYield)
                && task.capabilities().contains(Capability::TaskExit)
                && !task.capabilities().contains(Capability::ConsoleWrite)
        })
    };
    let syscall_count =
        unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_COUNTERS.syscall_calls)) };
    let yield_count = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_COUNTERS.yields)) };
    let exit_count = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_COUNTERS.exits)) };
    let denied_count = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_COUNTERS.denied)) };
    let console_write_count =
        unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_COUNTERS.console_writes)) };
    let switch_count = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_COUNTERS.switches)) };
    let a_code = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_CODE_FRAMES[0])) };
    let b_code = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_CODE_FRAMES[1])) };
    let a_stack = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_STACK_FRAMES[0])) };
    let b_stack = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_STACK_FRAMES[1])) };
    let a_markers_ok = unsafe {
        ptr::read_volatile((a_stack + PAGE_SIZE - 16) as *const u64) == 0x211C_E001
            && ptr::read_volatile((a_stack + PAGE_SIZE - 24) as *const u64) == 0x211C_E002
    };
    let b_markers_ok = unsafe {
        ptr::read_volatile((b_stack + PAGE_SIZE - 16) as *const u64) == 0x311C_E001
            && ptr::read_volatile((b_stack + PAGE_SIZE - 24) as *const u64) == 0x311C_E002
    };
    let roots_private = user_task_space_a_root() != 0
        && user_task_space_b_root() != 0
        && user_task_space_a_root() != user_task_space_b_root()
        && a_code != b_code
        && a_stack != b_stack;
    let tss_valid =
        user_task_stack_bounds(2).is_some_and(|(_, reaper_top)| gdt::read_rsp0() == reaper_top);
    let saved_a_ok = user_task_saved_context_matches(TaskId::from_raw(1));
    let saved_b_ok = user_task_saved_context_matches(TaskId::from_raw(2));
    report_user_task_saved_context(TaskId::from_raw(1), b"A");
    report_user_task_saved_context(TaskId::from_raw(2), b"B");
    let passed = tasks_ok
        && syscall_count == 6
        && yield_count == 1
        && exit_count == 2
        && denied_count == 2
        && console_write_count == 1
        && switch_count == 3
        && capability_grants_ok
        && roots_private
        && a_markers_ok
        && b_markers_ok
        && saved_a_ok
        && saved_b_ok
        && tss_valid;
    serial::COM1.write(b"P5.7: syscalls=");
    serial::COM1.write_hex(syscall_count);
    serial::COM1.write(b" yields=");
    serial::COM1.write_hex(yield_count);
    serial::COM1.write(b" exits=");
    serial::COM1.write_hex(exit_count);
    serial::COM1.write(b" denied=");
    serial::COM1.write_hex(denied_count);
    serial::COM1.write(b" console_writes=");
    serial::COM1.write_hex(console_write_count);
    serial::COM1.write(b" switches=");
    serial::COM1.write_hex(switch_count);
    serial::COM1.write(
        b"\r\nP5.7: A grants yield+exit+ConsoleWrite; B grants exit only; denied yield+ConsoleWrite returned -1=",
    );
    serial::COM1.write(if denied_count == 2 && capability_grants_ok {
        b"yes"
    } else {
        b"no "
    });
    serial::COM1.write(
        b"\r\nP5.7: ConsoleWrite A=allowed B=denied user-copy=validated unauthorized-output=no\r\n",
    );
    serial::COM1.write(b"P5.6: A markers=");
    serial::COM1.write(if a_markers_ok { b"both" } else { b"bad " });
    serial::COM1.write(b" B markers=");
    serial::COM1.write(if b_markers_ok { b"both" } else { b"bad " });
    serial::COM1.write(b" distinct CR3/code/stack=");
    serial::COM1.write(if roots_private { b"yes" } else { b"no " });
    serial::COM1.write(b" contexts A/B=");
    serial::COM1.write(if saved_a_ok { b"ok" } else { b"bad" });
    serial::COM1.write(b"/");
    serial::COM1.write(if saved_b_ok { b"ok" } else { b"bad" });
    serial::COM1.write(b"\r\nP5.6: Finished=");
    serial::COM1.write_hex(if tasks_ok { 2 } else { 0 });
    serial::COM1.write(b" kernel reaper running=yes TSS.RSP0=");
    serial::COM1.write_hex(gdt::read_rsp0());
    serial::COM1.write(b"\r\n");

    let cleanup_ok = reclaim_user_task_resources();
    // The reaper is now the running task on its own kernel stack and the
    // kernel address space is active, so the Finished user-task records can
    // finally be removed without invalidating a live execution context.
    let (reap_a_ok, reap_b_ok) = if cleanup_ok {
        unsafe {
            (
                (*scheduler).reap(TaskId::from_raw(1)).is_ok(),
                (*scheduler).reap(TaskId::from_raw(2)).is_ok(),
            )
        }
    } else {
        (false, false)
    };
    let reaped_ok = reap_a_ok && reap_b_ok;
    let scheduler_clean = unsafe {
        (*scheduler).current().map(|task| task.id()) == Some(TaskId::from_raw(3))
            && (*scheduler).len() == 1
            && (*scheduler).finished_count() == 0
            && (*scheduler).get(TaskId::from_raw(1)).is_none()
            && (*scheduler).get(TaskId::from_raw(2)).is_none()
    };
    let rsp0_cleared = gdt::set_rsp0(0) && gdt::read_rsp0() == 0;
    serial::COM1.write(b"P5.6: user address spaces and frames reclaimed=");
    serial::COM1.write(if cleanup_ok { b"yes" } else { b"no " });
    serial::COM1.write(b" task records reaped=");
    serial::COM1.write(if reaped_ok && scheduler_clean {
        b"yes"
    } else {
        b"no "
    });
    serial::COM1.write(b" TSS.RSP0 cleared=");
    serial::COM1.write(if rsp0_cleared { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\n");
    serial::COM1.write(
        if passed && cleanup_ok && reaped_ok && scheduler_clean && rsp0_cleared {
            b"P5.6: two-user-task yield/exit/reap lifecycle smoke test passed.\r\n"
        } else {
            b"P5.6: two-user-task yield/exit/reap lifecycle smoke test failed.\r\n"
        },
    );
    serial::COM1.write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
    let exit = if passed && cleanup_ok && reaped_ok && scheduler_clean && rsp0_cleared {
        0x10u32
    } else {
        0x11u32
    };
    unsafe {
        asm!("out dx, eax", in("dx") 0xF4u16, in("eax") exit, options(nomem, nostack, preserves_flags));
    }
    loop {
        unsafe {
            asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn report_user_task_saved_context(id: TaskId, label: &'static [u8]) {
    let scheduler = ptr::addr_of!(USER_TASK_SCHEDULER);
    let Some(task) = (unsafe { (&*scheduler).get(id) }) else {
        serial::COM1.write(b"P5.6: context ");
        serial::COM1.write(label);
        serial::COM1.write(b" task missing.\r\n");
        return;
    };
    let frame_address = task.context().saved_stack_pointer();
    let Some(metadata) = task.user_metadata() else {
        serial::COM1.write(b"P5.6: context ");
        serial::COM1.write(label);
        serial::COM1.write(b" metadata missing.\r\n");
        return;
    };
    serial::COM1.write(b"P5.6: context ");
    serial::COM1.write(label);
    serial::COM1.write(b" frame=0x");
    serial::COM1.write_hex(frame_address as u64);
    serial::COM1.write(b" expected RSP=0x");
    serial::COM1.write_hex(metadata.user_stack_pointer());
    serial::COM1.write(b" root=0x");
    serial::COM1.write_hex(metadata.address_space_root());
    let index = usize::try_from(id.get())
        .ok()
        .and_then(|raw| raw.checked_sub(1));
    let valid_index = index.filter(|index| *index < 2);
    if let Some(index) = valid_index {
        if let Some((base, top)) = user_task_stack_bounds(index) {
            let end =
                (frame_address as u64).checked_add(size_of::<UserTaskInterruptFrame>() as u64);
            let min = base + PAGE_SIZE;
            let in_bounds = frame_address != 0
                && (frame_address as u64) >= min
                && end.map_or(false, |value| value <= top);
            serial::COM1.write(b" base=0x");
            serial::COM1.write_hex(base);
            serial::COM1.write(b" min=0x");
            serial::COM1.write_hex(min);
            serial::COM1.write(b" top=0x");
            serial::COM1.write_hex(top);
            serial::COM1.write(b" cached-top=0x");
            serial::COM1.write_hex(unsafe {
                ptr::read_volatile(ptr::addr_of!(USER_TASK_KERNEL_STACK_TOPS[index]))
            });
            serial::COM1.write(b" end=0x");
            serial::COM1.write_hex(end.unwrap_or(0));
            serial::COM1.write(b" stack=");
            serial::COM1.write(if in_bounds { b"in" } else { b"out" });
            if in_bounds {
                let frame = unsafe { &*(frame_address as *const UserTaskInterruptFrame) };
                serial::COM1.write(b" CS=0x");
                serial::COM1.write_hex(frame.cs);
                serial::COM1.write(b" SS=0x");
                serial::COM1.write_hex(frame.ss);
                serial::COM1.write(b" FLAGS=0x");
                serial::COM1.write_hex(frame.rflags);
                serial::COM1.write(b" RIP=0x");
                serial::COM1.write_hex(frame.rip);
                serial::COM1.write(b" RSP=0x");
                serial::COM1.write_hex(frame.rsp);
            }
        } else {
            serial::COM1.write(b" stack=missing");
        }
    } else {
        serial::COM1.write(b" index=invalid");
    }
    serial::COM1.write(b"\r\n");
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn user_task_saved_context_matches(id: TaskId) -> bool {
    let scheduler = ptr::addr_of!(USER_TASK_SCHEDULER);
    let Some(task) = (unsafe { (&*scheduler).get(id) }) else {
        return false;
    };
    let Some(metadata) = task.user_metadata() else {
        return false;
    };
    let frame_address = task.context().saved_stack_pointer();
    let index = usize::try_from(id.get())
        .ok()
        .and_then(|raw| raw.checked_sub(1));
    let Some(index) = index.filter(|index| *index < 2) else {
        return false;
    };
    let Some((base, top)) = user_task_stack_bounds(index) else {
        return false;
    };
    let cached_top = unsafe {
        ptr::read_volatile(ptr::addr_of!(USER_TASK_KERNEL_STACK_TOPS[index]))
    };
    if cached_top != top {
        return false;
    }
    let end = (frame_address as u64).checked_add(size_of::<UserTaskInterruptFrame>() as u64);
    if frame_address == 0
        || (frame_address as u64) < base + PAGE_SIZE
        || end.map_or(true, |value| value > top)
    {
        return false;
    }
    let frame = unsafe { &*(frame_address as *const UserTaskInterruptFrame) };
    frame.cs == u64::from(gdt::USER_CODE_SELECTOR)
        && frame.ss == u64::from(gdt::USER_DATA_SELECTOR)
        && frame.rflags & (1 << 9) == 0
        && frame.rsp == metadata.user_stack_pointer()
        && (USER_CODE_ADDRESS..USER_CODE_ADDRESS + PAGE_SIZE).contains(&frame.rip)
        && metadata.address_space_root() != 0
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn reclaim_user_task_resources() -> bool {
    if address_space::restore_kernel().is_err() {
        return false;
    }
    let mut all_ok = true;
    for index in 0..2 {
        let space_slot = if index == 0 {
            ptr::addr_of_mut!(USER_TASK_SPACE_A)
        } else {
            ptr::addr_of_mut!(USER_TASK_SPACE_B)
        };
        let mut space = unsafe { (*space_slot).take() };
        let Some(ref mut address_space) = space else {
            all_ok = false;
            continue;
        };
        let code = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_CODE_FRAMES[index])) };
        let stack = unsafe { ptr::read_volatile(ptr::addr_of!(USER_TASK_STACK_FRAMES[index])) };
        let code_unmapped = address_space.unmap_user_page(USER_CODE_ADDRESS) == Ok(code);
        let stack_unmapped = address_space.unmap_user_page(USER_STACK_ADDRESS) == Ok(stack);
        let destroyed = address_space.destroy().is_ok();
        let code_released = code_unmapped && memory::release_frame(code).is_ok();
        let stack_released = stack_unmapped && memory::release_frame(stack).is_ok();
        if !(code_unmapped && stack_unmapped && destroyed && code_released && stack_released) {
            unsafe {
                ptr::write(ptr::addr_of_mut!(*space_slot), space);
            }
            all_ok = false;
        }
    }
    all_ok
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn user_task_setup_failed(message: &'static [u8]) -> ! {
    serial::COM1.write(message);
    serial::COM1.write(b"P5.6: setup failed; static stacks are retained.\r\n");
    user_task_runtime_failed(b"P5.6: controlled failure exit.\r\n")
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
fn user_task_runtime_failed(message: &'static [u8]) -> ! {
    serial::COM1.write(message);
    serial::COM1.write(b"P5.6: two-user-task yield/exit lifecycle smoke test failed.\r\n");
    serial::COM1.write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
    unsafe {
        asm!("out dx, eax", in("dx") 0xF4u16, in("eax") 0x11u32, options(nomem, nostack, preserves_flags));
    }
    loop {
        unsafe {
            asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}

#[cfg(feature = "user-task-lifecycle-smoke-test")]
global_asm!(
    r#"
    .section .text
.global exhi_user_task_kernel_entry
    .type exhi_user_task_kernel_entry, @function
exhi_user_task_kernel_entry:
    # Same-CPL IRETQ does not pop RSP/SS. Skip those two normalized qwords
    # before entering a normal SysV function on the dedicated kernel stack.
    add rsp, 16
    and rsp, -16
    call exhi_user_task_finish
.Luser_task_finish_halt:
    cli
    hlt
    jmp .Luser_task_finish_halt

    "#
);

fn reclaim_user_address_space() -> bool {
    if address_space::restore_kernel().is_err() {
        return false;
    }

    let space_slot = ptr::addr_of_mut!(RING3_ADDRESS_SPACE);
    let mut space = unsafe { (*space_slot).take() };
    let Some(ref mut address_space) = space else {
        return false;
    };

    let code_unmapped = address_space.unmap_user_page(USER_CODE_ADDRESS);
    let stack_unmapped = address_space.unmap_user_page(USER_STACK_ADDRESS);
    let destroyed = address_space.destroy();
    if code_unmapped != Ok(unsafe { ptr::read_volatile(ptr::addr_of!(RING3_CODE_FRAME)) })
        || stack_unmapped != Ok(unsafe { ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME)) })
        || destroyed.is_err()
    {
        unsafe {
            ptr::write(ptr::addr_of_mut!(RING3_ADDRESS_SPACE), space);
        }
        return false;
    }

    let code_frame = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_CODE_FRAME)) };
    let stack_frame = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME)) };
    let code_released = memory::release_frame(code_frame).is_ok();
    let stack_released = memory::release_frame(stack_frame).is_ok();
    code_released && stack_released
}

#[cfg(not(feature = "user-task-lifecycle-smoke-test"))]
fn allocate_zeroed_frame() -> u64 {
    let frame = memory::allocate_frame()
        .unwrap_or_else(|_| setup_failed(b"P5.1: physical frame allocation failed.\r\n"));
    if frame == 0 || frame % PAGE_SIZE != 0 || frame >= 0x1_0000_0000 {
        setup_failed(b"P5.1: physical allocator returned an invalid frame.\r\n");
    }
    unsafe {
        ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
    }
    frame
}

#[cfg(not(feature = "user-task-lifecycle-smoke-test"))]
fn setup_failed(message: &'static [u8]) -> ! {
    serial::COM1.write(message);
    finish(false)
}

fn finish(passed: bool) -> ! {
    let mode = unsafe { ptr::read_volatile(ptr::addr_of!(RING3_TEST_MODE)) };
    let marker = match (mode, passed) {
        (1, true) => {
            b"P5.1: ring3 entry and exception-return smoke test passed.\r\n" as &'static [u8]
        }
        (1, false) => b"P5.1: ring3 entry and exception-return smoke test failed.\r\n",
        (2, true) => b"P5.2: supervisor-only mapping fault test passed.\r\n",
        (2, false) => b"P5.2: supervisor-only mapping fault test failed.\r\n",
        (3, true) => b"P5.3: syscall entry/return smoke test passed.\r\n",
        (3, false) => b"P5.3: syscall entry/return smoke test failed.\r\n",
        (4, true) => b"P5.4: user-pointer range validation smoke test passed.\r\n",
        (4, false) => b"P5.4: user-pointer range validation smoke test failed.\r\n",
        (5, true) => b"P5.5: safe user-copy smoke test passed.\r\n",
        (5, false) => b"P5.5: safe user-copy smoke test failed.\r\n",
        (_, true) => b"Ring 3 runtime smoke test passed.\r\n",
        (_, false) => b"Ring 3 runtime smoke test failed.\r\n",
    };
    serial::COM1.write(marker);
    serial::COM1.write(b"QEMU debug-exit: requesting controlled shutdown.\r\n");
    let exit = if passed { 0x10u32 } else { 0x11u32 };
    unsafe {
        asm!(
            "out dx, eax",
            in("dx") 0xF4u16,
            in("eax") exit,
            options(nomem, nostack, preserves_flags)
        );
    }
    loop {
        unsafe {
            asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}
