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
static mut RING3_KERNEL_STACK_TOP: u64 = 0;
static mut RING3_ADDRESS_SPACE: Option<AddressSpace> = None;
static mut RING3_CODE_FRAME: u64 = 0;
static mut RING3_STACK_FRAME: u64 = 0;

unsafe extern "sysv64" {
    fn exhi_interrupt_context_start(initial_rsp: usize, fx_state: usize) -> !;
}

pub fn ring3_smoke_test() -> ! {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
        ptr::write(ptr::addr_of_mut!(RING3_ADDRESS_SPACE), None);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_CODE_FRAME), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_STACK_FRAME), 0);
        ptr::write_volatile(ptr::addr_of_mut!(RING3_KERNEL_STACK_TOP), 0);
    }

    serial::COM1.write(b"P5.1: preparing isolated Ring 3 address space.\r\n");

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
    const USER_CODE: [u8; USER_CODE_LENGTH as usize] = [
        // mov qword ptr [rsp - 8], 0x5A6B7C8D; int3
        0x48, 0xC7, 0x44, 0x24, 0xF8, 0x8D, 0x7C, 0x6B, 0x5A, 0xCC,
    ];
    unsafe {
        ptr::copy_nonoverlapping(USER_CODE.as_ptr(), code_frame as *mut u8, USER_CODE.len());
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

    serial::COM1.write(b"P5.1: user code mapped RX; user stack mapped RW+NX.\r\n");
    serial::COM1.write(b"P5.1: IRETQ will enter CPL3; INT3 must return through TSS.RSP0.\r\n");

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
    serial::COM1.write(b"P5.1: exception returned from user mode; vector=");
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

    let passed = vector == 3
        && error_code == 0
        && code_selector == u64::from(gdt::USER_CODE_SELECTOR)
        && code_selector & 3 == 3
        && instruction_pointer == USER_CODE_ADDRESS + USER_CODE_LENGTH
        && flags & (1 << 9) != 0
        && frame_in_bounds
        && stack_shape_valid
        && saved_user_rsp == USER_STACK_POINTER
        && saved_user_ss == u64::from(gdt::USER_DATA_SELECTOR)
        && mapping_checks
        && unsafe {
            ptr::read_volatile(
                (ptr::read_volatile(ptr::addr_of!(RING3_STACK_FRAME)) + PAGE_SIZE - 16)
                    as *const u64,
            ) == USER_STACK_SENTINEL
        };

    serial::COM1.write(b"P5.1: saved user RIP=0x");
    serial::COM1.write_hex(instruction_pointer);
    serial::COM1.write(b" RSP=0x");
    serial::COM1.write_hex(saved_user_rsp);
    serial::COM1.write(b" SS=0x");
    serial::COM1.write_hex(saved_user_ss);
    serial::COM1.write(b"\r\nP5.1: user code RX, guard unmapped, stack RW+NX and CPL3 stack write verified.\r\n");

    let cleanup_ok = reclaim_user_address_space();
    let rsp0_cleared = gdt::set_rsp0(0) && gdt::read_rsp0() == 0;
    finish(passed && cleanup_ok && rsp0_cleared)
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
    serial::COM1.write(if passed {
        b"P5.1: ring3 entry and exception-return smoke test passed.\r\n"
    } else {
        b"P5.1: ring3 entry and exception-return smoke test failed.\r\n"
    });
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
