//! Minimal Ring 3 transition and CPL3 exception-frame smoke test.
//!
//! This milestone proves privilege-level entry and a processor-originated
//! exception return to a TSS RSP0 stack. It does not implement syscalls,
//! process scheduling, or a userspace ABI.

use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::ptr;

use super::address_space::{self, AddressSpace, UserPageFlags};
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
const USER_CODE_LENGTH: u64 = 10;

#[repr(u64)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Ring3TestMode {
    Breakpoint = 1,
    SupervisorFault = 2,
    Syscall = 3,
    PointerValidation = 4,
}

static RING3_SUPERVISOR_PROBE: u64 = 0x4558_4849_4F53_5046;
static mut RING3_TEST_MODE: u64 = 0;
static mut RING3_EXPECTED_VECTOR: u64 = 3;
static mut RING3_EXPECTED_ERROR_MASK: u64 = u64::MAX;
static mut RING3_EXPECTED_ERROR_VALUE: u64 = 0;
static mut RING3_EXPECTED_RIP: u64 = 0;
static mut RING3_EXPECTED_FAULT_ADDRESS: u64 = 0;
#[no_mangle]
static mut RING3_USER_RSP: u64 = 0;
#[no_mangle]
static mut RING3_FX_STATE: usize = 0;
static mut RING3_SYSCALL_CALLS: u64 = 0;
static mut RING3_SYSCALL_VALID: u64 = 0;

#[repr(C, align(4096))]
struct KernelStack([u8; STACK_TOTAL_SIZE]);

#[repr(C, align(16))]
struct UserEntryFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rbx: u64,
    rdx: u64,
    rcx: u64,
    rax: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

const _: [(); 20 * size_of::<u64>()] = [(); size_of::<UserEntryFrame>()];

static mut RING3_KERNEL_STACK: KernelStack = KernelStack([0; STACK_TOTAL_SIZE]);
#[no_mangle]
static mut RING3_KERNEL_STACK_TOP: u64 = 0;
static mut RING3_ADDRESS_SPACE: Option<AddressSpace> = None;
static mut RING3_CODE_FRAME: u64 = 0;
static mut RING3_STACK_FRAME: u64 = 0;

unsafe extern "sysv64" {
    fn exhi_interrupt_context_start(initial_rsp: usize, fx_state: usize) -> !;
    fn exhi_syscall_entry();
}

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

#[cfg(feature = "ring3-supervisor-fault-smoke-test")]
pub fn ring3_supervisor_fault_smoke_test() -> ! {
    run_ring3_test(Ring3TestMode::SupervisorFault)
}

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
        ptr::write_volatile(ptr::addr_of_mut!(RING3_USER_RSP), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_FX_STATE), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_SYSCALL_CALLS), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_SYSCALL_VALID), 0);
    }

    serial::COM1.write(match mode {
        Ring3TestMode::Breakpoint => b"P5.1: preparing isolated Ring 3 address space.\r\n",
        Ring3TestMode::SupervisorFault => b"P5.2: preparing Ring 3 supervisor-isolation test.\r\n",
        Ring3TestMode::Syscall => b"P5.3: preparing Ring 3 SYSCALL/SYSRET test.\r\n",
        Ring3TestMode::PointerValidation => b"P5.4: preparing user-pointer range validation test.\r\n",
    });

    let Some(fx_state) = task::initialize_fx_state_for_user_mode() else {
        setup_failed(b"P5.1: FXSR/SSE context initialization failed.\r\n");
    };

    let kernel_stack_base = unsafe { ptr::addr_of_mut!(RING3_KERNEL_STACK) as u64 };
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
        ptr::write_volatile(
            ptr::addr_of_mut!(RING3_KERNEL_STACK_TOP),
            kernel_stack_top,
        );
    }

    let code_frame = allocate_zeroed_frame();
    let stack_frame = allocate_zeroed_frame();
    let (user_code, user_code_length, expected_vector, expected_error_mask, expected_error_value, expected_rip, expected_fault_address) =
        build_user_code(mode);
    unsafe {
        ptr::copy_nonoverlapping(
            user_code.as_ptr(),
            code_frame as *mut u8,
            user_code_length,
        );
        ptr::write_volatile(
            ptr::addr_of_mut!(RING3_EXPECTED_VECTOR),
            expected_vector,
        );
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
        ptr::write_volatile(
            ptr::addr_of_mut!(RING3_CODE_FRAME),
            code_frame,
        );
        ptr::write_volatile(
            ptr::addr_of_mut!(RING3_STACK_FRAME),
            stack_frame,
        );
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
    let permissions_valid = code_mapping.is_ok_and(|mapping| {
        mapping.user && mapping.executable && !mapping.writable
    }) && guard_mapping.is_err()
        && stack_mapping.is_ok_and(|mapping| {
            mapping.user && !mapping.executable && mapping.writable
        });
    if !permissions_valid {
        setup_failed(b"P5.1: user W^X or user/supervisor permissions are incorrect.\r\n");
    }

    unsafe {
        ptr::write(ptr::addr_of_mut!(RING3_ADDRESS_SPACE), Some(space));
    }

    serial::COM1.write(match mode {
        Ring3TestMode::Breakpoint => b"P5.1: user code mapped RX; user stack mapped RW+NX.\r\n",
        Ring3TestMode::SupervisorFault => b"P5.2: user RX code and RW+NX stack mapped; kernel probe is supervisor-only.\r\n",
        Ring3TestMode::Syscall => b"P5.3: user RX code and RW+NX stack mapped; syscall MSRs will be verified.\r\n",
        Ring3TestMode::PointerValidation => b"P5.4: user RX code, guard and RW+NX stack pages prepared.\r\n",
    });
    serial::COM1.write(b"Ring 3: IRETQ enters CPL3; the expected CPU exception returns through TSS.RSP0.\r\n");

    if mode == Ring3TestMode::PointerValidation {
        let space_slot = unsafe { &*ptr::addr_of!(RING3_ADDRESS_SPACE) };
        let pointers_valid = space_slot
            .as_ref()
            .is_some_and(validate_user_pointer_ranges);
        let cleanup_ok = reclaim_user_address_space();
        let rsp0_cleared = gdt::set_rsp0(0) && gdt::read_rsp0() == 0;
        serial::COM1.write(if pointers_valid {
            b"P5.4: user/read/write/execute/guard/overflow range checks passed.\r\n"
        } else {
            b"P5.4: user-pointer range validation failed.\r\n"
        });
        finish(pointers_valid && cleanup_ok && rsp0_cleared);
    }

    #[cfg(feature = "ring3-syscall-smoke-test")]
    if mode == Ring3TestMode::Syscall {
        unsafe { ptr::write_volatile(ptr::addr_of_mut!(RING3_FX_STATE), fx_state); }
        if !initialize_syscall_msrs() {
            setup_failed(b"P5.3: syscall MSR configuration or read-back failed.\r\n");
        }
    }

    let mut frame = UserEntryFrame {
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
        rflags: 0x202,
        rsp: USER_STACK_POINTER,
        ss: u64::from(gdt::USER_DATA_SELECTOR),
    };
    let frame_rsp = ptr::addr_of_mut!(frame) as usize;

    let space = unsafe { &*ptr::addr_of!(RING3_ADDRESS_SPACE) };
    if space.as_ref().map_or(true, |space| space.activate().is_err()) {
        setup_failed(b"P5.1: CR3 activation for Ring 3 failed.\r\n");
    }

    unsafe {
        exhi_interrupt_context_start(frame_rsp, fx_state);
    }
}


fn build_user_code(
    mode: Ring3TestMode,
) -> ([u8; 64], usize, u64, u64, u64, u64, u64) {
    // Each scenario first writes a sentinel through the actual CPL3 user
    // stack. P5.1 executes INT3, P5.2 faults on a supervisor-only read,
    // and P5.3 executes SYSCALL/SYSRET before the final INT3.
    let mut code = [0u8; 64];
    code[..10].copy_from_slice(&[
        0x48, 0xC7, 0x44, 0x24, 0xF8, 0x8D, 0x7C, 0x6B, 0x5A, 0x90,
    ]);
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
        Ring3TestMode::PointerValidation => {
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
    let mode = match unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_TEST_MODE))
    } {
        2 => Ring3TestMode::SupervisorFault,
        3 => Ring3TestMode::Syscall,
        _ => Ring3TestMode::Breakpoint,
    };
    serial::COM1.write(match mode {
        Ring3TestMode::Breakpoint => b"P5.1: exception returned from user mode; vector=",
        Ring3TestMode::SupervisorFault => b"P5.2: exception returned from user mode; vector=",
        Ring3TestMode::Syscall => b"P5.3: exception returned from user mode after SYSCALL; vector=",
        Ring3TestMode::PointerValidation => b"P5.4: unexpected exception during pointer validation; vector=",
    });
    serial::COM1.write_hex(vector);
    serial::COM1.write(b" CPL=");
    serial::COM1.write_hex(code_selector & 3);
    serial::COM1.write(b"\r\n");

    let kernel_stack_base = unsafe { ptr::addr_of!(RING3_KERNEL_STACK) as u64 };
    let kernel_stack_top = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_KERNEL_STACK_TOP))
    };
    let frame_bytes = 7 * size_of::<u64>();
    let frame_end = (exception_stack_pointer as u64).checked_add(frame_bytes as u64);
    let frame_in_bounds = exception_stack_pointer != 0
        && (exception_stack_pointer as u64) >= kernel_stack_base + PAGE_SIZE
        && frame_end.is_some_and(|end| end <= kernel_stack_top);

    let frame = if frame_in_bounds {
        unsafe {
            core::slice::from_raw_parts(
                exception_stack_pointer as *const u64,
                7,
            )
        }
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
        code.is_ok_and(|mapping| {
            mapping.user && mapping.executable && !mapping.writable
        }) && guard.is_err() && stack.is_ok_and(|mapping| {
            mapping.user && !mapping.executable && mapping.writable
        })
    });

    let expected_vector = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_VECTOR))
    };
    let expected_error_mask = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_ERROR_MASK))
    };
    let expected_error_value = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_ERROR_VALUE))
    };
    let expected_rip = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_RIP))
    };
    let expected_fault_address = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_EXPECTED_FAULT_ADDRESS))
    };
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
            (ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME)) + PAGE_SIZE - 16)
                as *const u64,
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
        Ring3TestMode::SupervisorFault => b"P5.2: user supervisor-access fault RIP/RSP/SS verified.\r\n",
        Ring3TestMode::Syscall => b"P5.3: syscall returned to CPL3; return value and user frame verified.\r\n",
        Ring3TestMode::PointerValidation => b"P5.4: pointer-range verification completed.\r\n",
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


fn initialize_syscall_msrs() -> bool {
    const EFER: u32 = 0xC000_0080;
    const STAR: u32 = 0xC000_0081;
    const LSTAR: u32 = 0xC000_0082;
    const FMASK: u32 = 0xC000_0084;
    const EFER_SCE: u64 = 1;
    const KERNEL_CS: u64 = gdt::KERNEL_CODE_SELECTOR as u64;
    // SYSRETQ computes SS = STAR[63:48] + 8 and CS = STAR[63:48] + 16.
    // The base value itself intentionally denotes no descriptor.
    const USER_STAR_BASE: u64 = 0x23;
    const FMASK_BITS: u64 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 18);

    let max_extended = unsafe { core::arch::x86_64::__cpuid(0x8000_0000) };
    if max_extended.eax < 0x8000_0001 {
        return false;
    }
    let features = unsafe { core::arch::x86_64::__cpuid(0x8000_0001) };
    if features.edx & (1 << 11) == 0 {
        return false;
    }

    let mut efer = unsafe { read_msr(EFER) };
    efer |= EFER_SCE;
    let star = (USER_STAR_BASE << 48) | (KERNEL_CS << 32);
    let lstar = exhi_syscall_entry as *const () as u64;
    unsafe {
        write_msr(EFER, efer);
        write_msr(STAR, star);
        write_msr(LSTAR, lstar);
        write_msr(FMASK, FMASK_BITS);
    }

    unsafe {
        read_msr(EFER) & EFER_SCE != 0
            && read_msr(STAR) == star
            && read_msr(LSTAR) == lstar
            && read_msr(FMASK) == FMASK_BITS
    }
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

#[no_mangle]
extern "sysv64" fn exhi_syscall_handler(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> u64 {
    let valid = number == 1
        && arg1 == 0x11
        && arg2 == 0x22
        && arg3 == 0x33
        && arg4 == 0x44
        && arg5 == 0x55;
    unsafe {
        let calls = ptr::addr_of_mut!(RING3_SYSCALL_CALLS);
        ptr::write_volatile(calls, ptr::read_volatile(calls).saturating_add(1));
        ptr::write_volatile(ptr::addr_of_mut!(RING3_SYSCALL_VALID), u64::from(valid));
    }
    if valid { 0xC0DE_CAFE_F00D_BEEFu64 } else { 0xBAD0_BAD0_BAD0_BAD0u64 }
}

global_asm!(
    r#"
    .section .text
    .global exhi_syscall_entry
    .type exhi_syscall_entry, @function
exhi_syscall_entry:
    # SYSCALL leaves RSP on the untrusted user stack. Switch immediately to
    # the dedicated kernel stack; interrupts are disabled by IA32_FMASK.
    mov qword ptr [rip + RING3_USER_RSP], rsp
    mov rsp, qword ptr [rip + RING3_KERNEL_STACK_TOP]

    # Ten pushes keep the stack 16-byte aligned before calling the Rust ABI.
    push qword ptr [rip + RING3_USER_RSP]
    push r11
    push rcx
    push r9
    push r8
    push r10
    push rdx
    push rsi
    push rdi
    push rax

    # Preserve user x87/SSE state around the kernel Rust handler.
    mov rax, qword ptr [rip + RING3_FX_STATE]
    test rax, rax
    jz .Lsyscall_fatal
    fxsave64 [rax]

    # Stack now: number, arg1, arg2, arg3, arg4(R10), arg5(R8),
    # saved user R9, RCX, R11, user RSP.
    mov rdi, qword ptr [rsp]
    mov rsi, qword ptr [rsp + 8]
    mov rdx, qword ptr [rsp + 16]
    mov rcx, qword ptr [rsp + 24]
    mov r8,  qword ptr [rsp + 32]
    mov r9,  qword ptr [rsp + 40]
    cld
    call exhi_syscall_handler

    mov rcx, qword ptr [rip + RING3_FX_STATE]
    fxrstor64 [rcx]

    # Discard saved syscall number, preserve RAX as the return value.
    add rsp, 8
    pop rdi
    pop rsi
    pop rdx
    pop r10
    pop r8
    pop r9
    pop rcx
    pop r11
    pop rsp
    sysretq

.Lsyscall_fatal:
    mov dx, 0xF4
    mov eax, 0x11
    out dx, eax
    cli
.Lsyscall_halt:
    hlt
    jmp .Lsyscall_halt
"#
);


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserAccess {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserRangeError {
    AddressOverflow,
    RangeTooLarge,
    NotUserMemory,
    PermissionDenied,
    AddressSpace(address_space::AddressSpaceError),
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
            UserAccess::Execute => mapping.executable,
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

fn validate_user_pointer_ranges(space: &AddressSpace) -> bool {
    let code_read = validate_user_range(space, USER_CODE_ADDRESS, 10, UserAccess::Read).is_ok();
    let code_execute = validate_user_range(space, USER_CODE_ADDRESS, 10, UserAccess::Execute).is_ok();
    let code_write = validate_user_range(space, USER_CODE_ADDRESS, 1, UserAccess::Write)
        == Err(UserRangeError::PermissionDenied);
    let stack_read = validate_user_range(space, USER_STACK_ADDRESS, 32, UserAccess::Read).is_ok();
    let stack_write = validate_user_range(space, USER_STACK_TOP - 32, 32, UserAccess::Write).is_ok();
    let stack_execute = validate_user_range(space, USER_STACK_ADDRESS, 1, UserAccess::Execute)
        == Err(UserRangeError::PermissionDenied);
    let guard_rejected = matches!(
        validate_user_range(
            space,
            USER_CODE_ADDRESS + PAGE_SIZE,
            1,
            UserAccess::Read,
        ),
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
    let overflow = validate_user_range(
        space,
        u64::MAX - 3,
        8,
        UserAccess::Read,
    ) == Err(UserRangeError::AddressOverflow);
    let too_large = validate_user_range(
        space,
        USER_STACK_ADDRESS,
        (MAX_USER_POINTER_RANGE + 1) as usize,
        UserAccess::Read,
    ) == Err(UserRangeError::RangeTooLarge);

    code_read && code_execute && code_write
        && stack_read && stack_write && stack_execute
        && guard_rejected && crossing_guard_rejected
        && supervisor_rejected && zero_length && overflow && too_large
}

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
    if code_unmapped != Ok(unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_CODE_FRAME))
    }) || stack_unmapped != Ok(unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME))
    }) || destroyed.is_err()
    {
        unsafe {
            ptr::write(ptr::addr_of_mut!(RING3_ADDRESS_SPACE), space);
        }
        return false;
    }

    let code_frame = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_CODE_FRAME))
    };
    let stack_frame = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME))
    };
    let code_released = memory::release_frame(code_frame).is_ok();
    let stack_released = memory::release_frame(stack_frame).is_ok();
    code_released && stack_released
}

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

fn setup_failed(message: &'static [u8]) -> ! {
    serial::COM1.write(message);
    finish(false)
}

fn finish(passed: bool) -> ! {
    let mode = unsafe {
        ptr::read_volatile(ptr::addr_of!(RING3_TEST_MODE))
    };
    let marker = match (mode, passed) {
        (1, true) => b"P5.1: ring3 entry and exception-return smoke test passed.\r\n" as &'static [u8],
        (1, false) => b"P5.1: ring3 entry and exception-return smoke test failed.\r\n",
        (2, true) => b"P5.2: supervisor-only mapping fault test passed.\r\n",
        (2, false) => b"P5.2: supervisor-only mapping fault test failed.\r\n",
        (3, true) => b"P5.3: syscall entry/return smoke test passed.\r\n",
        (3, false) => b"P5.3: syscall entry/return smoke test failed.\r\n",
        (4, true) => b"P5.4: user-pointer range validation smoke test passed.\r\n",
        (4, false) => b"P5.4: user-pointer range validation smoke test failed.\r\n",
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
        unsafe { asm!("cli", "hlt", options(nomem, nostack)); }
    }
}
