//! x86-64 task contexts, cooperative switching and timer-preemption tests.
//!
//! All tasks use static guarded Ring 0 stacks and one shared kernel address
//! space. The ordinary PIT handler remains minimal; the opt-in P4.4 test
//! installs a separate full-frame IRQ0 path for single-CPU preemption.

use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

use exhi_task::{ScheduleDecision, Scheduler, TaskContext, TaskId};

use super::{gdt, paging, serial, vmm};
#[cfg(any(feature = "task-accounting-smoke-test", feature = "timer-preemption-smoke-test"))]
use super::timer;

const TASK_COUNT: usize = 2;
const PAGE_SIZE: usize = 4096;
const STACK_USABLE_SIZE: usize = 16 * 1024;
const STACK_TOTAL_SIZE: usize = PAGE_SIZE + STACK_USABLE_SIZE;
const INITIAL_CONTEXT_WORDS: usize = 8;
const INITIAL_CONTEXT_SIZE: usize = INITIAL_CONTEXT_WORDS * size_of::<usize>();

#[repr(C, align(4096))]
struct TaskStack([u8; STACK_TOTAL_SIZE]);

static mut TASK_STACKS: [TaskStack; TASK_COUNT] = [
    TaskStack([0; STACK_TOTAL_SIZE]),
    TaskStack([0; STACK_TOTAL_SIZE]),
];
static mut TASK_STACK_TOPS: [u64; TASK_COUNT] = [0; TASK_COUNT];
#[cfg(feature = "task-accounting-smoke-test")]
static mut TASK_ACCOUNTING_START_TICKS: [u64; TASK_COUNT] = [0; TASK_COUNT];

#[cfg(feature = "timer-preemption-smoke-test")]
static mut TASK_PREEMPTION_START_TICKS: [u64; TASK_COUNT] = [0; TASK_COUNT];
#[cfg(feature = "timer-preemption-smoke-test")]
static mut PREEMPTION_TASK_A_ID: Option<TaskId> = None;
#[cfg(feature = "timer-preemption-smoke-test")]
static mut PREEMPTION_TASK_B_ID: Option<TaskId> = None;

#[cfg(feature = "timer-preemption-smoke-test")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct FxSaveArea([u8; 512]);
#[cfg(feature = "timer-preemption-smoke-test")]
static mut TASK_FX_STATES: [FxSaveArea; TASK_COUNT] = [FxSaveArea([0; 512]); TASK_COUNT];
#[cfg(feature = "timer-preemption-smoke-test")]
static mut FX_SAVE_TEMPLATE: FxSaveArea = FxSaveArea([0; 512]);
#[cfg(feature = "timer-preemption-smoke-test")]
static DEFAULT_MXCSR: u32 = 0x1F80;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_ACTIVE_FX_STATE: usize = 0;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_P4_4_SWITCHES: u64 = 0;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_P4_4_FAILURES: u64 = 0;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_P4_4_A_PROGRESS: u64 = 0;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_P4_4_B_PROGRESS: u64 = 0;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_P4_4_A_XMM_OK: u64 = 0;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_P4_4_B_XMM_OK: u64 = 0;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_P4_4_A_LOCAL: usize = 0;
#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
static mut EXHIOS_P4_4_B_LOCAL: usize = 0;

#[cfg(feature = "timer-preemption-smoke-test")]
#[repr(C)]
struct InterruptContext {
    r15: u64, r14: u64, r13: u64, r12: u64,
    r11: u64, r10: u64, r9: u64, r8: u64,
    rdi: u64, rsi: u64, rbp: u64, rbx: u64,
    rdx: u64, rcx: u64, rax: u64,
    rip: u64, cs: u64, rflags: u64,
    // In 64-bit mode the CPU always saves and IRETQ always restores RSP:SS,
    // including interrupts and returns that remain at CPL0.
    rsp: u64,
    ss: u64,
}
#[cfg(feature = "timer-preemption-smoke-test")]
const _: [(); 20 * size_of::<u64>()] = [(); size_of::<InterruptContext>()];

static mut SCHEDULER: Scheduler<TASK_COUNT> = Scheduler::new();
static mut BOOT_STACK_POINTER: usize = 0;
static mut FINISHED_STACK_POINTER: usize = 0;

static TASK_A_PROGRESS: AtomicUsize = AtomicUsize::new(0);
static TASK_B_PROGRESS: AtomicUsize = AtomicUsize::new(0);
static TASK_FAILURES: AtomicUsize = AtomicUsize::new(0);
static TASK_A_LOCAL_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static TASK_B_LOCAL_ADDRESS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "sysv64" {
    fn exhi_context_switch(old_rsp: *mut usize, new_rsp: usize);
    fn exhi_task_bootstrap() -> !;
}

#[cfg(feature = "timer-preemption-smoke-test")]
unsafe extern "sysv64" {
    fn exhi_irq0_preempt_stub();
    fn exhi_interrupt_context_start(initial_rsp: usize, fx_state: usize) -> !;
    fn exhi_preemptive_task_bootstrap() -> !;
    fn exhi_preemptive_task_a() -> !;
    fn exhi_preemptive_task_b() -> !;
    fn exhi_preemptive_task_exit_bootstrap() -> !;
    fn exhi_preemptive_task_exit_a();
    fn exhi_preemptive_task_exit_b();
}

/// Runs two tasks that repeatedly yield to each other, then returns to the
/// original boot stack after both task entry functions finish.
pub fn context_switch_smoke_test() -> bool {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
        ptr::write(ptr::addr_of_mut!(SCHEDULER), Scheduler::new());
        ptr::write(ptr::addr_of_mut!(BOOT_STACK_POINTER), 0);
        ptr::write(ptr::addr_of_mut!(FINISHED_STACK_POINTER), 0);
        ptr::write(ptr::addr_of_mut!(TASK_STACK_TOPS), [0; TASK_COUNT]);
    }
    TASK_A_PROGRESS.store(0, Ordering::Relaxed);
    TASK_B_PROGRESS.store(0, Ordering::Relaxed);
    TASK_FAILURES.store(0, Ordering::Relaxed);
    TASK_A_LOCAL_ADDRESS.store(0, Ordering::Relaxed);
    TASK_B_LOCAL_ADDRESS.store(0, Ordering::Relaxed);

    serial::COM1.write(b"P4.2: preparing two guarded kernel task stacks.\r\n");
    let Some((a_initial_rsp, a_top)) = (unsafe { prepare_stack(0, task_a) }) else {
        serial::COM1.write(b"P4.2: task A stack setup failed.\r\n");
        return false;
    };
    let Some((b_initial_rsp, b_top)) = (unsafe { prepare_stack(1, task_b) }) else {
        serial::COM1.write(b"P4.2: task B stack setup failed.\r\n");
        return false;
    };

    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let first = unsafe {
        match (*scheduler).spawn("task-A", TaskContext::new(a_initial_rsp)) {
            Ok(id) => id,
            Err(_) => {
                serial::COM1.write(b"P4.2: could not register task A.\r\n");
                return false;
            }
        }
    };
    let second = unsafe {
        match (*scheduler).spawn("task-B", TaskContext::new(b_initial_rsp)) {
            Ok(id) => id,
            Err(_) => {
                serial::COM1.write(b"P4.2: could not register task B.\r\n");
                return false;
            }
        }
    };
    if first.get() != 1 || second.get() != 2 {
        serial::COM1.write(b"P4.2: unexpected task identifiers.\r\n");
        return false;
    }

    let selected = unsafe { (*scheduler).start() };
    if selected != Ok(first) {
        serial::COM1.write(b"P4.2: scheduler did not select task A first.\r\n");
        return false;
    }
    if !gdt::set_rsp0(a_top) {
        serial::COM1.write(b"P4.2: failed to install task A TSS.RSP0.\r\n");
        return false;
    }

    let first_rsp = unsafe {
        (*scheduler)
            .get(first)
            .map(|task| task.context().saved_stack_pointer())
            .unwrap_or(0)
    };
    if first_rsp == 0 {
        serial::COM1.write(b"P4.2: task A initial RSP is null.\r\n");
        return false;
    }

    serial::COM1.write(b"P4.2: switching from boot stack to task A.\r\n");
    // SAFETY: Both stacks lie in the identity-mapped kernel image. The boot
    // RSP slot is static and the task context points at its prepared frame.
    unsafe {
        exhi_context_switch(ptr::addr_of_mut!(BOOT_STACK_POINTER), first_rsp);
    }

    let rsp0_restored = gdt::set_rsp0(0) && gdt::read_rsp0() == 0;
    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let scheduler_finished = unsafe {
        (*scheduler).current().is_none()
            && (*scheduler).len() == TASK_COUNT
            && (*scheduler).ready_count() == 0
            && (*scheduler).blocked_count() == 0
            && (*scheduler).finished_count() == TASK_COUNT
    };

    let a_local = TASK_A_LOCAL_ADDRESS.load(Ordering::Relaxed) as u64;
    let b_local = TASK_B_LOCAL_ADDRESS.load(Ordering::Relaxed) as u64;
    let a_stack_base = unsafe { ptr::addr_of!(TASK_STACKS[0]) as u64 };
    let b_stack_base = unsafe { ptr::addr_of!(TASK_STACKS[1]) as u64 };
    let a_local_valid = a_local >= a_stack_base + PAGE_SIZE as u64 && a_local < a_top;
    let b_local_valid = b_local >= b_stack_base + PAGE_SIZE as u64 && b_local < b_top;

    let contexts_saved = unsafe {
        (*scheduler)
            .get(first)
            .is_some_and(|task| task.context().saved_stack_pointer() != a_initial_rsp)
            && (*scheduler)
                .get(second)
                .is_some_and(|task| task.context().saved_stack_pointer() != b_initial_rsp)
    };

    let passed = rsp0_restored
        && scheduler_finished
        && contexts_saved
        && a_local_valid
        && b_local_valid
        && a_local != b_local
        && TASK_A_PROGRESS.load(Ordering::Relaxed) == 3
        && TASK_B_PROGRESS.load(Ordering::Relaxed) == 3
        && TASK_FAILURES.load(Ordering::Relaxed) == 0
        && unsafe { ptr::read_volatile(ptr::addr_of!(BOOT_STACK_POINTER)) != 0 };

    serial::COM1.write(b"P4.2: task A progress=");
    serial::COM1.write_hex(TASK_A_PROGRESS.load(Ordering::Relaxed) as u64);
    serial::COM1.write(b" task B progress=");
    serial::COM1.write_hex(TASK_B_PROGRESS.load(Ordering::Relaxed) as u64);
    serial::COM1.write(b"\r\nP4.2: task-local stack addresses A=0x");
    serial::COM1.write_hex(a_local);
    serial::COM1.write(b" B=0x");
    serial::COM1.write_hex(b_local);
    serial::COM1.write(b"\r\nP4.2: scheduler states Finished=");
    serial::COM1.write_hex(unsafe { (*scheduler).finished_count() as u64 });
    serial::COM1.write(b" TSS.RSP0 restored=");
    serial::COM1.write(if rsp0_restored { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\n");

    passed
}


/// Runs the cooperative scheduler with IRQ0 enabled and accounts elapsed PIT
/// ticks to each task only at explicit scheduling boundaries.
#[cfg(feature = "task-accounting-smoke-test")]
pub fn timer_accounting_smoke_test() -> bool {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
        ptr::write(ptr::addr_of_mut!(SCHEDULER), Scheduler::new());
        ptr::write(ptr::addr_of_mut!(BOOT_STACK_POINTER), 0);
        ptr::write(ptr::addr_of_mut!(FINISHED_STACK_POINTER), 0);
        ptr::write(ptr::addr_of_mut!(TASK_STACK_TOPS), [0; TASK_COUNT]);
        ptr::write(ptr::addr_of_mut!(TASK_ACCOUNTING_START_TICKS), [0; TASK_COUNT]);
    }
    TASK_A_PROGRESS.store(0, Ordering::Relaxed);
    TASK_B_PROGRESS.store(0, Ordering::Relaxed);
    TASK_FAILURES.store(0, Ordering::Relaxed);

    serial::COM1.write(b"P4.3: initializing PIT-backed cooperative task accounting.\r\n");
    let timer_status = timer::initialize();
    serial::COM1.write(b"P4.3: PIT requested Hz=");
    serial::COM1.write_hex(u64::from(timer_status.requested_hz));
    serial::COM1.write(b" divisor=");
    serial::COM1.write_hex(u64::from(timer_status.pit_divisor));
    serial::COM1.write(b" IRQ0 unmasked=");
    serial::COM1.write(if timer_status.irq0_unmasked { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\n");
    if !timer_status.irq0_unmasked {
        let _ = timer::mask_irq0();
        serial::COM1.write(b"P4.3: PIT IRQ0 could not be enabled.\r\n");
        return false;
    }

    let Some((a_initial_rsp, a_top)) =
        (unsafe { prepare_stack(0, task_a_accounting) })
    else {
        let _ = timer::mask_irq0();
        serial::COM1.write(b"P4.3: task A stack setup failed.\r\n");
        return false;
    };
    let Some((b_initial_rsp, b_top)) =
        (unsafe { prepare_stack(1, task_b_accounting) })
    else {
        let _ = timer::mask_irq0();
        serial::COM1.write(b"P4.3: task B stack setup failed.\r\n");
        return false;
    };

    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let first = unsafe {
        match (*scheduler).spawn("account-A", TaskContext::new(a_initial_rsp)) {
            Ok(id) => id,
            Err(_) => {
                let _ = timer::mask_irq0();
                serial::COM1.write(b"P4.3: could not register task A.\r\n");
                return false;
            }
        }
    };
    let second = unsafe {
        match (*scheduler).spawn("account-B", TaskContext::new(b_initial_rsp)) {
            Ok(id) => id,
            Err(_) => {
                let _ = timer::mask_irq0();
                serial::COM1.write(b"P4.3: could not register task B.\r\n");
                return false;
            }
        }
    };

    if unsafe { (*scheduler).start() } != Ok(first) || !gdt::set_rsp0(a_top) {
        let _ = timer::mask_irq0();
        serial::COM1.write(b"P4.3: scheduler start or initial TSS.RSP0 setup failed.\r\n");
        return false;
    }

    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(TASK_ACCOUNTING_START_TICKS[0]),
            timer::ticks(),
        );
    }

    serial::COM1.write(b"P4.3: starting two tasks with IRQ0 enabled; no IRQ-side scheduling.\r\n");
    // Keep IF clear while the assembly switch restores another task context.
    // Individual tasks enable interrupts only while waiting for PIT ticks.
    // SAFETY: The PIT is programmed, IRQ0 has a valid IDT gate and the PIC
    // unmasks only IRQ0. Its ISR only updates the atomic tick count and EOI.
    unsafe {
        exhi_context_switch(ptr::addr_of_mut!(BOOT_STACK_POINTER), a_initial_rsp);
        asm!("cli", options(nomem, nostack, preserves_flags));
    }

    let pic_status = timer::mask_irq0();
    let rsp0_restored = gdt::read_rsp0() == 0;
    let scheduler_finished = unsafe {
        (*scheduler).current().is_none()
            && (*scheduler).len() == TASK_COUNT
            && (*scheduler).ready_count() == 0
            && (*scheduler).blocked_count() == 0
            && (*scheduler).finished_count() == TASK_COUNT
    };
    let a_runtime = unsafe {
        (*scheduler).get(first).map(|task| task.runtime_ticks()).unwrap_or(0)
    };
    let b_runtime = unsafe {
        (*scheduler).get(second).map(|task| task.runtime_ticks()).unwrap_or(0)
    };
    let elapsed = timer::ticks();
    let irq0_masked = pic_status.master_mask & 0x01 != 0 && pic_status.slave_mask == 0xFF;
    let passed = scheduler_finished
        && rsp0_restored
        && irq0_masked
        && elapsed >= 12
        && a_runtime >= 6
        && b_runtime >= 6
        && TASK_A_PROGRESS.load(Ordering::Relaxed) == 3
        && TASK_B_PROGRESS.load(Ordering::Relaxed) == 3
        && TASK_FAILURES.load(Ordering::Relaxed) == 0
        && unsafe { ptr::read_volatile(ptr::addr_of!(BOOT_STACK_POINTER)) != 0 };

    serial::COM1.write(b"P4.3: elapsed PIT ticks=");
    serial::COM1.write_hex(elapsed);
    serial::COM1.write(b" task A runtime=");
    serial::COM1.write_hex(a_runtime);
    serial::COM1.write(b" task B runtime=");
    serial::COM1.write_hex(b_runtime);
    serial::COM1.write(b"\r\nP4.3: Finished=");
    serial::COM1.write_hex(unsafe { (*scheduler).finished_count() as u64 });
    serial::COM1.write(b" IRQ0 masked=");
    serial::COM1.write(if irq0_masked { b"yes" } else { b"no " });
    serial::COM1.write(b" TSS.RSP0 restored=");
    serial::COM1.write(if rsp0_restored { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\n");
    passed
}



/// Runs the preemptive task-exit lifecycle test. Task A returns first and must
/// become Finished; task B then runs alone and exits the scheduler cleanly.
#[cfg(feature = "preemptive-task-exit-smoke-test")]
pub fn preemptive_task_exit_smoke_test() -> ! {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
        ptr::write(ptr::addr_of_mut!(SCHEDULER), Scheduler::new());
        ptr::write(ptr::addr_of_mut!(TASK_STACK_TOPS), [0; TASK_COUNT]);
        ptr::write(ptr::addr_of_mut!(TASK_PREEMPTION_START_TICKS), [0; TASK_COUNT]);
        ptr::write(ptr::addr_of_mut!(PREEMPTION_TASK_A_ID), None);
        ptr::write(ptr::addr_of_mut!(PREEMPTION_TASK_B_ID), None);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_ACTIVE_FX_STATE), 0);
    }
    TASK_A_PROGRESS.store(0, Ordering::Relaxed);
    TASK_B_PROGRESS.store(0, Ordering::Relaxed);
    TASK_FAILURES.store(0, Ordering::Relaxed);
    TASK_A_LOCAL_ADDRESS.store(0, Ordering::Relaxed);
    TASK_B_LOCAL_ADDRESS.store(0, Ordering::Relaxed);
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_SWITCHES), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_A_PROGRESS), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_B_PROGRESS), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_A_XMM_OK), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_B_XMM_OK), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_A_LOCAL), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_B_LOCAL), 0);
    }

    serial::COM1.write(b"P4.5: preparing preemptive task-exit contexts.\r\n");
    if !initialize_fx_contexts() {
        preemption_setup_failed(b"P4.5: CPU FXSR/SSE state initialization failed.\r\n");
    }
    let Some((a_rsp, a_top)) =
        (unsafe { prepare_preemption_exit_stack(0, exhi_preemptive_task_exit_a) })
    else {
        preemption_setup_failed(b"P4.5: task A stack setup failed.\r\n");
    };
    let Some((b_rsp, _b_top)) =
        (unsafe { prepare_preemption_exit_stack(1, exhi_preemptive_task_exit_b) })
    else {
        preemption_setup_failed(b"P4.5: task B stack setup failed.\r\n");
    };

    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let first = unsafe {
        (*scheduler)
            .spawn("preempt-exit-A", TaskContext::new(a_rsp))
            .unwrap_or_else(|_| preemption_setup_failed(b"P4.5: task A registration failed.\r\n"))
    };
    let second = unsafe {
        (*scheduler)
            .spawn("preempt-exit-B", TaskContext::new(b_rsp))
            .unwrap_or_else(|_| preemption_setup_failed(b"P4.5: task B registration failed.\r\n"))
    };
    if first.get() != 1
        || second.get() != 2
        || unsafe { (*scheduler).start() } != Ok(first)
        || !gdt::set_rsp0(a_top)
    {
        preemption_setup_failed(b"P4.5: scheduler initialization failed.\r\n");
    }

    let timer_status = timer::initialize();
    if !timer_status.irq0_unmasked {
        let _ = timer::mask_irq0();
        preemption_setup_failed(b"P4.5: PIT IRQ0 could not be enabled.\r\n");
    }
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(PREEMPTION_TASK_A_ID), Some(first));
        ptr::write_volatile(ptr::addr_of_mut!(PREEMPTION_TASK_B_ID), Some(second));
        ptr::write_volatile(ptr::addr_of_mut!(TASK_PREEMPTION_START_TICKS[0]), timer::ticks());
        ptr::write_volatile(
            ptr::addr_of_mut!(EXHIOS_ACTIVE_FX_STATE),
            ptr::addr_of_mut!(TASK_FX_STATES[0]) as usize,
        );
    }
    serial::COM1.write(b"P4.5: PIT 100 Hz ready; tasks run without cooperative yields.\r\n");
    serial::COM1.write(b"P4.5: task A will return first; task B will finish the test.\r\n");
    // SAFETY: IRQ0 has the full-frame ISR, both guarded stacks and FX states
    // are initialized, and only IRQ0 is unmasked.
    unsafe {
        exhi_interrupt_context_start(a_rsp, ptr::addr_of!(TASK_FX_STATES[0]) as usize);
    }
}

#[cfg(feature = "preemptive-task-exit-smoke-test")]
unsafe fn prepare_preemption_exit_stack(
    index: usize,
    entry: unsafe extern "sysv64" fn(),
) -> Option<(usize, u64)> {
    if index >= TASK_COUNT {
        return None;
    }
    let base = unsafe { ptr::addr_of_mut!(TASK_STACKS[index]) as u64 };
    let top = base.checked_add(STACK_TOTAL_SIZE as u64)?;
    if base % PAGE_SIZE as u64 != 0 || top % 16 != 0 || top >= 0x1_0000_0000 {
        return None;
    }
    if vmm::unmap_page(base).ok()? != base
        || paging::translate_current(base) != Err(paging::PagingError::NotMapped)
    {
        return None;
    }
    let rsp = (top as usize).checked_sub(size_of::<InterruptContext>())?;
    if rsp % 16 != 0 {
        return None;
    }
    let frame = InterruptContext {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: entry as usize as u64,
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
        rip: exhi_preemptive_task_exit_bootstrap as *const () as usize as u64,
        cs: u64::from(gdt::KERNEL_CODE_SELECTOR),
        rflags: 0x202,
        rsp: top,
        ss: u64::from(gdt::KERNEL_DATA_SELECTOR),
    };
    unsafe {
        ptr::write(rsp as *mut InterruptContext, frame);
        ptr::write(ptr::addr_of_mut!(TASK_STACK_TOPS[index]), top);
    }
    Some((rsp, top))
}

#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_preemptive_task_finished() -> ! {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let current_id = unsafe {
        (*scheduler)
            .current()
            .map(|task| task.id())
            .unwrap_or_else(|| preemption_setup_failed(b"P4.5: task returned without a Running scheduler entry.\r\n"))
    };
    let Some(current_index) = task_stack_index(current_id) else {
        preemption_setup_failed(b"P4.5: exiting task has no kernel stack.\r\n");
    };
    let start = unsafe {
        ptr::read_volatile(ptr::addr_of!(TASK_PREEMPTION_START_TICKS[current_index]))
    };
    if unsafe {
        (*scheduler).account_current_ticks(timer::ticks().wrapping_sub(start))
    }
    .is_err()
    {
        preemption_setup_failed(b"P4.5: could not account final task runtime.\r\n");
    }

    let next = unsafe { (*scheduler).finish_current() }
        .unwrap_or_else(|_| preemption_setup_failed(b"P4.5: scheduler could not finish the task.\r\n"));
    match next {
        Some(next_id) => {
            let Some(index) = task_stack_index(next_id) else {
                preemption_setup_failed(b"P4.5: next task has no stack slot.\r\n");
            };
            let Some(next_task) = (unsafe { (*scheduler).get(next_id) }) else {
                preemption_setup_failed(b"P4.5: next task is missing from scheduler.\r\n");
            };
            let next_rsp = next_task.context().saved_stack_pointer();
            let next_top = unsafe { ptr::read_volatile(ptr::addr_of!(TASK_STACK_TOPS[index])) };
            let next_base = unsafe { ptr::addr_of!(TASK_STACKS[index]) as u64 };
            let next_end = (next_rsp as u64).checked_add(size_of::<InterruptContext>() as u64);
            if next_rsp == 0
                || (next_rsp as u64) < next_base + PAGE_SIZE as u64
                || next_end.map_or(true, |end| end > next_top)
                || !gdt::set_rsp0(next_top)
            {
                preemption_setup_failed(b"P4.5: next task interrupt frame or TSS.RSP0 is invalid.\r\n");
            }
            unsafe {
                ptr::write_volatile(
                    ptr::addr_of_mut!(TASK_PREEMPTION_START_TICKS[index]),
                    timer::ticks(),
                );
                ptr::write_volatile(
                    ptr::addr_of_mut!(EXHIOS_ACTIVE_FX_STATE),
                    ptr::addr_of_mut!(TASK_FX_STATES[index]) as usize,
                );
                exhi_interrupt_context_start(
                    next_rsp,
                    ptr::addr_of!(TASK_FX_STATES[index]) as usize,
                );
            }
        }
        None => {
            if !gdt::set_rsp0(0) {
                preemption_setup_failed(b"P4.5: failed to clear TSS.RSP0 after final task.\r\n");
            }
            unsafe {
                ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_ACTIVE_FX_STATE), 0);
            }
            finish_preemptive_exit_test(true);
        }
    }
}

#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_preemptive_task_exit_fail() -> ! {
    finish_preemptive_exit_test(false)
}

#[cfg(feature = "timer-preemption-smoke-test")]
fn finish_preemptive_exit_test(target_reached: bool) -> ! {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
    let pic_status = timer::mask_irq0();
    let elapsed = timer::ticks();
    let switches = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_SWITCHES)) };
    let failures = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_FAILURES)) };
    let a_progress = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_A_PROGRESS)) };
    let b_progress = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_B_PROGRESS)) };
    let a_local = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_A_LOCAL)) as u64 };
    let b_local = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_B_LOCAL)) as u64 };
    let a_base = unsafe { ptr::addr_of!(TASK_STACKS[0]) as u64 };
    let b_base = unsafe { ptr::addr_of!(TASK_STACKS[1]) as u64 };
    let a_top = unsafe { ptr::read_volatile(ptr::addr_of!(TASK_STACK_TOPS[0])) };
    let b_top = unsafe { ptr::read_volatile(ptr::addr_of!(TASK_STACK_TOPS[1])) };
    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let a_id = unsafe { ptr::read_volatile(ptr::addr_of!(PREEMPTION_TASK_A_ID)) };
    let b_id = unsafe { ptr::read_volatile(ptr::addr_of!(PREEMPTION_TASK_B_ID)) };
    let a_finished = unsafe {
        a_id.is_some_and(|id| {
            (*scheduler)
                .get(id)
                .is_some_and(|task| task.state() == exhi_task::TaskState::Finished)
        })
    };
    let b_finished = unsafe {
        b_id.is_some_and(|id| {
            (*scheduler)
                .get(id)
                .is_some_and(|task| task.state() == exhi_task::TaskState::Finished)
        })
    };
    let a_runtime = unsafe {
        a_id.and_then(|id| (*scheduler).get(id).map(|task| task.runtime_ticks())).unwrap_or(0)
    };
    let b_runtime = unsafe {
        b_id.and_then(|id| (*scheduler).get(id).map(|task| task.runtime_ticks())).unwrap_or(0)
    };
    let b_rsp = unsafe {
        b_id.and_then(|id| (*scheduler).get(id).map(|task| task.context().saved_stack_pointer())).unwrap_or(0)
    };
    let a_local_ok = a_local >= a_base + PAGE_SIZE as u64 && a_local < a_top;
    let b_local_ok = b_local >= b_base + PAGE_SIZE as u64 && b_local < b_top;
    let b_frame_ok = b_rsp as u64 >= b_base + PAGE_SIZE as u64
        && (b_rsp as u64).checked_add(size_of::<InterruptContext>() as u64)
            .is_some_and(|end| end <= b_top);
    let all_finished = unsafe {
        (*scheduler).current().is_none()
            && (*scheduler).ready_count() == 0
            && (*scheduler).blocked_count() == 0
            && (*scheduler).finished_count() == TASK_COUNT
    };
    let tss_cleared = gdt::read_rsp0() == 0;
    let fx_state_cleared = unsafe {
        ptr::read_volatile(ptr::addr_of!(EXHIOS_ACTIVE_FX_STATE)) == 0
    };
    let irq0_masked = pic_status.master_mask & 1 != 0 && pic_status.slave_mask == 0xFF;

    let passed = target_reached
        && elapsed >= 24
        && switches >= 12
        && a_runtime >= 6
        && b_runtime >= 6
        && a_progress > 0
        && b_progress > 0
        && failures == 0
        && a_finished
        && b_finished
        && a_local_ok
        && b_local_ok
        && a_local != b_local
        && b_frame_ok
        && unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_A_XMM_OK)) == 1 }
        && unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_B_XMM_OK)) == 1 }
        && all_finished
        && tss_cleared
        && fx_state_cleared
        && irq0_masked;

    serial::COM1.write(b"P4.5: elapsed PIT ticks=");
    serial::COM1.write_hex(elapsed);
    serial::COM1.write(b" preemption switches=");
    serial::COM1.write_hex(switches as u64);
    serial::COM1.write(b" task A runtime=");
    serial::COM1.write_hex(a_runtime);
    serial::COM1.write(b" task B runtime=");
    serial::COM1.write_hex(b_runtime);
    serial::COM1.write(b"\r\nP4.5: task A progress=");
    serial::COM1.write_hex(a_progress as u64);
    serial::COM1.write(b" task B progress=");
    serial::COM1.write_hex(b_progress as u64);
    serial::COM1.write(b"\r\nP4.5: task A finished=");
    serial::COM1.write(if a_finished { b"yes" } else { b"no " });
    serial::COM1.write(b" task B finished=");
    serial::COM1.write(if b_finished { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\nP4.5: XMM state A=");
    serial::COM1.write(if unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_A_XMM_OK)) == 1 } { b"preserved" } else { b"failed   " });
    serial::COM1.write(b" B=");
    serial::COM1.write(if unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_B_XMM_OK)) == 1 } { b"preserved" } else { b"failed   " });
    serial::COM1.write(b" stacks distinct=");
    serial::COM1.write(if a_local_ok && b_local_ok && a_local != b_local { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\nP4.5: scheduler current=none Finished=");
    serial::COM1.write_hex(unsafe { (*scheduler).finished_count() as u64 });
    serial::COM1.write(b" TSS.RSP0 cleared=");
    serial::COM1.write(if tss_cleared { b"yes" } else { b"no " });
    serial::COM1.write(b" IRQ0 masked=");
    serial::COM1.write(if irq0_masked { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\n");
    serial::COM1.write(if passed {
        b"P4.5: preemptive task-exit runtime smoke test passed.\r\n"
    } else {
        b"P4.5: preemptive task-exit runtime smoke test failed.\r\n"
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

/// Dedicated IRQ0 entry for the full interrupt-frame preemption path.
#[cfg(feature = "timer-preemption-smoke-test")]
pub fn preemption_irq0_stub_address() -> u64 {
    exhi_irq0_preempt_stub as *const () as usize as u64
}

/// Runs two tasks that never yield. IRQ0 alone must alternate their contexts.
#[cfg(feature = "timer-preemption-smoke-test")]
pub fn timer_preemption_smoke_test() -> ! {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
        ptr::write(ptr::addr_of_mut!(SCHEDULER), Scheduler::new());
        ptr::write(ptr::addr_of_mut!(TASK_STACK_TOPS), [0; TASK_COUNT]);
        ptr::write(ptr::addr_of_mut!(TASK_PREEMPTION_START_TICKS), [0; TASK_COUNT]);
        ptr::write(ptr::addr_of_mut!(PREEMPTION_TASK_A_ID), None);
        ptr::write(ptr::addr_of_mut!(PREEMPTION_TASK_B_ID), None);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_ACTIVE_FX_STATE), 0);
    }
    TASK_A_PROGRESS.store(0, Ordering::Relaxed);
    TASK_B_PROGRESS.store(0, Ordering::Relaxed);
    TASK_FAILURES.store(0, Ordering::Relaxed);
    TASK_A_LOCAL_ADDRESS.store(0, Ordering::Relaxed);
    TASK_B_LOCAL_ADDRESS.store(0, Ordering::Relaxed);
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_SWITCHES), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_A_PROGRESS), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_B_PROGRESS), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_A_XMM_OK), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_B_XMM_OK), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_A_LOCAL), 0);
        ptr::write_volatile(ptr::addr_of_mut!(EXHIOS_P4_4_B_LOCAL), 0);
    }

    serial::COM1.write(b"P4.4: preparing hardware-interrupt task contexts.\r\n");
    if !initialize_fx_contexts() {
        preemption_setup_failed(b"P4.4: CPU FXSR/SSE state initialization failed.\r\n");
    }
    let Some((a_rsp, a_top)) = (unsafe { prepare_preemption_stack(0, exhi_preemptive_task_a) }) else {
        preemption_setup_failed(b"P4.4: task A stack setup failed.\r\n");
    };
    let Some((b_rsp, _)) = (unsafe { prepare_preemption_stack(1, exhi_preemptive_task_b) }) else {
        preemption_setup_failed(b"P4.4: task B stack setup failed.\r\n");
    };

    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let first = unsafe {
        (*scheduler).spawn("preempt-A", TaskContext::new(a_rsp))
            .unwrap_or_else(|_| preemption_setup_failed(b"P4.4: task A registration failed.\r\n"))
    };
    let second = unsafe {
        (*scheduler).spawn("preempt-B", TaskContext::new(b_rsp))
            .unwrap_or_else(|_| preemption_setup_failed(b"P4.4: task B registration failed.\r\n"))
    };
    if first.get() != 1 || second.get() != 2 || unsafe { (*scheduler).start() } != Ok(first)
        || !gdt::set_rsp0(a_top)
    {
        preemption_setup_failed(b"P4.4: scheduler initialization failed.\r\n");
    }

    let timer_status = timer::initialize();
    if !timer_status.irq0_unmasked {
        let _ = timer::mask_irq0();
        preemption_setup_failed(b"P4.4: PIT IRQ0 could not be enabled.\r\n");
    }
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(PREEMPTION_TASK_A_ID), Some(first));
        ptr::write_volatile(ptr::addr_of_mut!(PREEMPTION_TASK_B_ID), Some(second));
        ptr::write_volatile(ptr::addr_of_mut!(TASK_PREEMPTION_START_TICKS[0]), timer::ticks());
        ptr::write_volatile(
            ptr::addr_of_mut!(EXHIOS_ACTIVE_FX_STATE),
            ptr::addr_of_mut!(TASK_FX_STATES[0]) as usize,
        );
    }
    serial::COM1.write(b"P4.4: PIT 100 Hz ready; IRQ0 will preempt tasks with no cooperative yields.\r\n");
    serial::COM1.write(b"P4.4: initial IRETQ enters task A with IF=1.\r\n");
    // SAFETY: IDT routes IRQ0 to the full-frame ISR; both guarded stacks and
    // FX states are initialized, and only IRQ0 is unmasked.
    unsafe {
        exhi_interrupt_context_start(a_rsp, ptr::addr_of!(TASK_FX_STATES[0]) as usize);
    }
}

#[cfg(feature = "timer-preemption-smoke-test")]
fn preemption_setup_failed(message: &'static [u8]) -> ! {
    serial::COM1.write(message);
    serial::COM1.write(b"QEMU debug-exit: reporting preemption setup failure.\r\n");
    unsafe {
        asm!(
            "out dx, eax",
            in("dx") 0xF4u16,
            in("eax") 0x11u32,
            options(nomem, nostack, preserves_flags)
        );
    }
    loop {
        unsafe { asm!("cli", "hlt", options(nomem, nostack)); }
    }
}

#[cfg(feature = "timer-preemption-smoke-test")]
fn initialize_fx_contexts() -> bool {
    let features = unsafe { core::arch::x86_64::__cpuid(1) };
    const REQUIRED: u32 = (1 << 24) | (1 << 25) | (1 << 26); // FXSR, SSE, SSE2
    if features.edx & REQUIRED != REQUIRED {
        return false;
    }
    let mut cr0: u64;
    unsafe { asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags)); }
    cr0 = (cr0 | (1 << 1) | (1 << 5)) & !((1 << 2) | (1 << 3));
    unsafe { asm!("mov cr0, {}", in(reg) cr0, options(nostack, preserves_flags)); }

    let mut cr4: u64;
    unsafe { asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags)); }
    if cr4 & (1 << 18) != 0 {
        // OSXSAVE may already be enabled by firmware. FXSAVE is still a
        // complete task-state format when XCR0 enables only x87 + SSE.
        // Reject any extended state component that this slice cannot save.
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
            return false;
        }
    }
    cr4 |= (1 << 9) | (1 << 10); // OSFXSR | OSXMMEXCPT
    unsafe {
        asm!(
            "mov cr4, {value}",
            "fninit",
            "ldmxcsr [{mxcsr}]",
            value = in(reg) cr4,
            mxcsr = in(reg) ptr::addr_of!(DEFAULT_MXCSR),
            options(nostack)
        );
        asm!(
            "fxsave64 [{}]",
            in(reg) ptr::addr_of_mut!(FX_SAVE_TEMPLATE),
            options(nostack)
        );
        for index in 0..TASK_COUNT {
            ptr::copy_nonoverlapping(
                ptr::addr_of!(FX_SAVE_TEMPLATE),
                ptr::addr_of_mut!(TASK_FX_STATES[index]),
                1,
            );
        }
    }
    true
}

#[cfg(feature = "timer-preemption-smoke-test")]
unsafe fn prepare_preemption_stack(
    index: usize,
    entry: unsafe extern "sysv64" fn() -> !,
) -> Option<(usize, u64)> {
    if index >= TASK_COUNT { return None; }
    let base = unsafe { ptr::addr_of_mut!(TASK_STACKS[index]) as u64 };
    let top = base.checked_add(STACK_TOTAL_SIZE as u64)?;
    if base % PAGE_SIZE as u64 != 0 || top % 16 != 0 || top >= 0x1_0000_0000 {
        return None;
    }
    if vmm::unmap_page(base).ok()? != base
        || paging::translate_current(base) != Err(paging::PagingError::NotMapped)
    {
        return None;
    }
    let rsp = (top as usize).checked_sub(size_of::<InterruptContext>())?;
    if rsp % 16 != 0 { return None; }
    let frame = InterruptContext {
        r15: 0, r14: 0, r13: 0, r12: entry as usize as u64,
        r11: 0, r10: 0, r9: 0, r8: 0,
        rdi: 0, rsi: 0, rbp: 0, rbx: 0,
        rdx: 0, rcx: 0, rax: 0,
        rip: exhi_preemptive_task_bootstrap as *const () as usize as u64,
        cs: u64::from(gdt::KERNEL_CODE_SELECTOR),
        rflags: 0x202,
        // IRETQ in 64-bit mode consumes all five architectural return slots.
        // The task begins at the top of its private Ring 0 stack.
        rsp: top,
        ss: u64::from(gdt::KERNEL_DATA_SELECTOR),
    };
    unsafe {
        ptr::write(rsp as *mut InterruptContext, frame);
        ptr::write(ptr::addr_of_mut!(TASK_STACK_TOPS[index]), top);
    }
    Some((rsp, top))
}

#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_preemption_handler(frame: *mut InterruptContext) -> usize {
    let frame_address = frame as usize;
    if frame_address == 0 {
        unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
        return 0;
    }
    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let Some(id) = (unsafe { (*scheduler).current().map(|task| task.id()) }) else {
        unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
        return frame_address;
    };
    let Some(index) = task_stack_index(id) else {
        unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
        return frame_address;
    };
    let base = unsafe { ptr::addr_of!(TASK_STACKS[index]) as u64 };
    let top = unsafe { ptr::read_volatile(ptr::addr_of!(TASK_STACK_TOPS[index])) };
    let end = (frame_address as u64).checked_add(size_of::<InterruptContext>() as u64);
    if (frame_address as u64) < base + PAGE_SIZE as u64 || end.map_or(true, |v| v > top) {
        unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
        return frame_address;
    }

    let start = unsafe { ptr::read_volatile(ptr::addr_of!(TASK_PREEMPTION_START_TICKS[index])) };
    if unsafe { (*scheduler).account_current_ticks(timer::ticks().wrapping_sub(start)) }.is_err() {
        unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
        return frame_address;
    }
    let Some(current) = (unsafe { (*scheduler).get_mut(id) }) else {
        unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
        return frame_address;
    };
    current.context_mut().set_saved_stack_pointer(frame_address);

    match unsafe { (*scheduler).preempt_current() } {
        Ok(ScheduleDecision::Continue { current }) => {
            if let Some(i) = task_stack_index(current) {
                unsafe {
                    ptr::write_volatile(
                        ptr::addr_of_mut!(TASK_PREEMPTION_START_TICKS[i]),
                        timer::ticks(),
                    );
                }
            }
            frame_address
        }
        Ok(ScheduleDecision::Switch { to, .. }) => {
            let Some(i) = task_stack_index(to) else {
                unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
                return 0;
            };
            let Some(next_task) = (unsafe { (*scheduler).get(to) }) else {
                unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
                return 0;
            };
            let next_rsp = next_task.context().saved_stack_pointer();
            let next_top = unsafe { ptr::read_volatile(ptr::addr_of!(TASK_STACK_TOPS[i])) };
            let next_base = unsafe { ptr::addr_of!(TASK_STACKS[i]) as u64 };
            let next_end = (next_rsp as u64).checked_add(size_of::<InterruptContext>() as u64);
            if next_rsp == 0 || (next_rsp as u64) < next_base + PAGE_SIZE as u64
                || next_end.map_or(true, |v| v > next_top) || !gdt::set_rsp0(next_top)
            {
                unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
                return 0;
            }
            unsafe {
                ptr::write_volatile(ptr::addr_of_mut!(TASK_PREEMPTION_START_TICKS[i]), timer::ticks());
                ptr::write_volatile(
                    ptr::addr_of_mut!(EXHIOS_ACTIVE_FX_STATE),
                    ptr::addr_of_mut!(TASK_FX_STATES[i]) as usize,
                );
            }
            unsafe {
                let p = ptr::addr_of_mut!(EXHIOS_P4_4_SWITCHES);
                ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
            }
            next_rsp
        }
        Err(_) => {
            unsafe {
            let p = ptr::addr_of_mut!(EXHIOS_P4_4_FAILURES);
            ptr::write_volatile(p, ptr::read_volatile(p).saturating_add(1));
        }
            frame_address
        }
    }
}

#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_preemption_test_pass() -> ! {
    finish_preemption_test(true)
}

#[cfg(feature = "timer-preemption-smoke-test")]
#[no_mangle]
extern "sysv64" fn exhi_preemption_test_fail() -> ! {
    finish_preemption_test(false)
}

#[cfg(feature = "timer-preemption-smoke-test")]
fn finish_preemption_test(target_reached: bool) -> ! {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)); }
    let pic_status = timer::mask_irq0();
    let elapsed = timer::ticks();
    let switches = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_SWITCHES)) };
    let failures = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_FAILURES)) };
    let a_progress = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_A_PROGRESS)) };
    let b_progress = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_B_PROGRESS)) };
    let a_local = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_A_LOCAL)) as u64 };
    let b_local = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_B_LOCAL)) as u64 };
    let a_base = unsafe { ptr::addr_of!(TASK_STACKS[0]) as u64 };
    let b_base = unsafe { ptr::addr_of!(TASK_STACKS[1]) as u64 };
    let a_top = unsafe { ptr::read_volatile(ptr::addr_of!(TASK_STACK_TOPS[0])) };
    let b_top = unsafe { ptr::read_volatile(ptr::addr_of!(TASK_STACK_TOPS[1])) };
    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let a_id = unsafe { ptr::read_volatile(ptr::addr_of!(PREEMPTION_TASK_A_ID)) };
    let b_id = unsafe { ptr::read_volatile(ptr::addr_of!(PREEMPTION_TASK_B_ID)) };
    let current_a = unsafe {
        a_id.is_some_and(|id| (*scheduler).current().is_some_and(|task| {
            task.id() == id && task.state() == exhi_task::TaskState::Running
        }))
    };
    let a_runtime = unsafe {
        a_id.and_then(|id| (*scheduler).get(id).map(|task| task.runtime_ticks())).unwrap_or(0)
    };
    let b_runtime = unsafe {
        b_id.and_then(|id| (*scheduler).get(id).map(|task| task.runtime_ticks())).unwrap_or(0)
    };
    let b_rsp = unsafe {
        b_id.and_then(|id| (*scheduler).get(id).map(|task| task.context().saved_stack_pointer())).unwrap_or(0)
    };
    let a_local_ok = a_local >= a_base + PAGE_SIZE as u64 && a_local < a_top;
    let b_local_ok = b_local >= b_base + PAGE_SIZE as u64 && b_local < b_top;
    let b_frame_ok = b_rsp as u64 >= b_base + PAGE_SIZE as u64
        && (b_rsp as u64).checked_add(size_of::<InterruptContext>() as u64)
            .is_some_and(|end| end <= b_top);
    let fx_a = unsafe { ptr::addr_of!(TASK_FX_STATES[0]) as usize };
    let fx_a_active = unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_ACTIVE_FX_STATE)) == fx_a };
    let tss_a = gdt::read_rsp0() == a_top;
    let irq0_masked = pic_status.master_mask & 1 != 0 && pic_status.slave_mask == 0xFF;

    let passed = target_reached && elapsed >= 16 && switches >= 12
        && a_runtime >= 6 && b_runtime >= 6
        && a_progress > 0 && b_progress > 0 && failures == 0
        && a_local_ok && b_local_ok && a_local != b_local && b_frame_ok
        && unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_A_XMM_OK)) == 1 }
        && unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_B_XMM_OK)) == 1 }
        && fx_a_active && current_a
        && unsafe { (*scheduler).ready_count() == 1 && (*scheduler).finished_count() == 0 }
        && tss_a && irq0_masked;

    serial::COM1.write(b"P4.4: elapsed PIT ticks=");
    serial::COM1.write_hex(elapsed);
    serial::COM1.write(b" preemption switches=");
    serial::COM1.write_hex(switches as u64);
    serial::COM1.write(b" task A runtime=");
    serial::COM1.write_hex(a_runtime);
    serial::COM1.write(b" task B runtime=");
    serial::COM1.write_hex(b_runtime);
    serial::COM1.write(b"\r\nP4.4: task A progress=");
    serial::COM1.write_hex(a_progress as u64);
    serial::COM1.write(b" task B progress=");
    serial::COM1.write_hex(b_progress as u64);
    serial::COM1.write(b"\r\nP4.4: XMM state A=");
    serial::COM1.write(if unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_A_XMM_OK)) == 1 } { b"preserved" } else { b"failed   " });
    serial::COM1.write(b" B=");
    serial::COM1.write(if unsafe { ptr::read_volatile(ptr::addr_of!(EXHIOS_P4_4_B_XMM_OK)) == 1 } { b"preserved" } else { b"failed   " });
    serial::COM1.write(b" stacks distinct=");
    serial::COM1.write(if a_local_ok && b_local_ok && a_local != b_local { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\nP4.4: scheduler current=A ready=1 TSS.RSP0 active=");
    serial::COM1.write(if tss_a { b"yes" } else { b"no " });
    serial::COM1.write(b" IRQ0 masked=");
    serial::COM1.write(if irq0_masked { b"yes" } else { b"no " });
    serial::COM1.write(b"\r\n");
    serial::COM1.write(if passed {
        b"P4.4: timer-driven preemption runtime smoke test passed.\r\n"
    } else {
        b"P4.4: timer-driven preemption runtime smoke test failed.\r\n"
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
    loop { unsafe { asm!("cli", "hlt", options(nomem, nostack)); } }
}

unsafe fn prepare_stack(
    index: usize,
    entry: extern "sysv64" fn(),
) -> Option<(usize, u64)> {
    if index >= TASK_COUNT {
        return None;
    }

    let stack_base = unsafe { ptr::addr_of_mut!(TASK_STACKS[index]) as u64 };
    let stack_top = stack_base.checked_add(STACK_TOTAL_SIZE as u64)?;
    if stack_base % PAGE_SIZE as u64 != 0
        || stack_top % 16 != 0
        || stack_top >= 0x1_0000_0000
    {
        return None;
    }

    // The first page of each static stack is a permanent unmapped guard page.
    let removed = vmm::unmap_page(stack_base).ok()?;
    if removed != stack_base
        || paging::translate_current(stack_base) != Err(paging::PagingError::NotMapped)
    {
        return None;
    }

    let initial_rsp = (stack_top as usize).checked_sub(INITIAL_CONTEXT_SIZE)?;
    if initial_rsp % 16 != 0 {
        return None;
    }

    // Restore order: R15, R14, R13, R12, RBX, RBP, RET.
    // R12 carries the task entry function into the assembly bootstrap.
    let frame = initial_rsp as *mut usize;
    unsafe {
        for slot in 0..INITIAL_CONTEXT_WORDS {
            ptr::write(frame.add(slot), 0);
        }
        ptr::write(frame.add(3), entry as usize);
        ptr::write(frame.add(6), exhi_task_bootstrap as usize);
        ptr::write(frame.add(7), 0xE7_1A_5A_C0_DE_C0_DE_u64 as usize);
        ptr::write(ptr::addr_of_mut!(TASK_STACK_TOPS[index]), stack_top);
    }

    Some((initial_rsp, stack_top))
}

fn switch_between(from: TaskId, to: TaskId) {
    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let (old_rsp, new_rsp, stack_top) = unsafe {
        let Some(old_task) = (*scheduler).get_mut(from) else {
            task_fatal(b"P4.2: current task is missing from scheduler.\r\n");
        };
        let old_rsp = old_task.context_mut().saved_stack_pointer_mut_ptr();
        let Some(new_task) = (*scheduler).get(to) else {
            task_fatal(b"P4.2: next task is missing from scheduler.\r\n");
        };
        let new_rsp = new_task.context().saved_stack_pointer();
        let index = task_stack_index(to).unwrap_or_else(|| {
            task_fatal(b"P4.2: next task has no kernel stack.\r\n")
        });
        let stack_top = ptr::read(ptr::addr_of!(TASK_STACK_TOPS[index]));
        (old_rsp, new_rsp, stack_top)
    };

    if new_rsp == 0 || !gdt::set_rsp0(stack_top) {
        task_fatal(b"P4.2: invalid next RSP or TSS.RSP0.\r\n");
    }

    #[cfg(feature = "task-accounting-smoke-test")]
    unsafe {
        let index = task_stack_index(to).unwrap_or_else(|| {
            task_fatal(b"P4.3: next task has no accounting slot.\r\n")
        });
        ptr::write_volatile(
            ptr::addr_of_mut!(TASK_ACCOUNTING_START_TICKS[index]),
            timer::ticks(),
        );
    }

    // SAFETY: Both pointers refer to stable static scheduler/task-stack storage.
    unsafe { exhi_context_switch(old_rsp, new_rsp) };
}

fn task_stack_index(id: TaskId) -> Option<usize> {
    let raw = usize::try_from(id.get()).ok()?;
    let index = raw.checked_sub(1)?;
    (index < TASK_COUNT).then_some(index)
}

#[cfg(feature = "task-accounting-smoke-test")]
fn account_current_runtime() -> bool {
    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let Some(current_id) = (unsafe { (*scheduler).current().map(|task| task.id()) }) else {
        return false;
    };
    let Some(index) = task_stack_index(current_id) else {
        return false;
    };
    let start = unsafe {
        ptr::read_volatile(ptr::addr_of!(TASK_ACCOUNTING_START_TICKS[index]))
    };
    let elapsed = timer::ticks().wrapping_sub(start);
    unsafe { (*scheduler).account_current_ticks(elapsed).is_ok() }
}

#[cfg(feature = "task-accounting-smoke-test")]
fn yield_current_accounted() {
    // Do not permit IRQ0 to interrupt the register/stack restore sequence.
    // The resumed task re-enables interrupts after its context switch returns.
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
    if !account_current_runtime() {
        task_fatal(b"P4.3: could not account current task runtime.\r\n");
    }
    yield_current();
    unsafe {
        asm!("sti", options(nomem, nostack, preserves_flags));
    }
}

#[cfg(feature = "task-accounting-smoke-test")]
fn wait_for_pit_ticks(minimum: u64) {
    let start = timer::ticks();
    while timer::elapsed_ticks(start) < minimum {
        // STI; HLT avoids burning a CPU while waiting for the next PIT IRQ.
        unsafe {
            asm!("sti", "hlt", options(nomem, nostack));
        }
    }
}

fn yield_current() {
    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let decision = unsafe { (*scheduler).yield_current() };
    match decision {
        Ok(ScheduleDecision::Switch { from, to }) => switch_between(from, to),
        Ok(ScheduleDecision::Continue { .. }) => {}
        Err(_) => task_fatal(b"P4.2: scheduler rejected cooperative yield.\r\n"),
    }
}

/// Called by the assembly bootstrap after a task entry function returns.
#[no_mangle]
pub extern "sysv64" fn exhi_task_finished() -> ! {
    #[cfg(feature = "task-accounting-smoke-test")]
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }

    #[cfg(feature = "task-accounting-smoke-test")]
    if !account_current_runtime() {
        task_fatal(b"P4.3: could not account final task runtime.\r\n");
    }

    let scheduler = ptr::addr_of_mut!(SCHEDULER);
    let (from, next) = unsafe {
        let from = (*scheduler)
            .current()
            .map(|task| task.id())
            .unwrap_or_else(|| task_fatal(b"P4.2: task returned without being current.\r\n"));
        let next = (*scheduler).finish_current();
        (from, next)
    };

    match next {
        Ok(Some(to)) => {
            switch_between(from, to);
            task_fatal(b"P4.2: a Finished task was unexpectedly resumed.\\r\\n");
        }
        Ok(None) => {
            if !gdt::set_rsp0(0) {
                task_fatal(b"P4.2: failed to clear TSS.RSP0 before boot return.\r\n");
            }
            let boot_rsp = unsafe { ptr::read_volatile(ptr::addr_of!(BOOT_STACK_POINTER)) };
            if boot_rsp == 0 {
                task_fatal(b"P4.2: saved boot RSP is null.\r\n");
            }
            // No task remains runnable. Save the finished task's last stack
            // pointer and restore the boot context that launched task A.
            unsafe {
                exhi_context_switch(
                    ptr::addr_of_mut!(FINISHED_STACK_POINTER),
                    boot_rsp,
                );
            }
            task_fatal(b"P4.2: a Finished task was unexpectedly resumed.\r\n");
        }
        Err(_) => task_fatal(b"P4.2: scheduler could not finish the current task.\r\n"),
    }
}


#[cfg(feature = "task-accounting-smoke-test")]
#[inline(never)]
extern "sysv64" fn task_a_accounting() {
    for step in 1..=3 {
        wait_for_pit_ticks(3);
        TASK_A_PROGRESS.store(step, Ordering::Relaxed);
        serial::COM1.write(b"P4.3: task A observed PIT interval; step ");
        serial::COM1.write_hex(step as u64);
        serial::COM1.write(b".\r\n");
        if step < 3 {
            yield_current_accounted();
        }
    }
}

#[cfg(feature = "task-accounting-smoke-test")]
#[inline(never)]
extern "sysv64" fn task_b_accounting() {
    for step in 1..=3 {
        wait_for_pit_ticks(3);
        TASK_B_PROGRESS.store(step, Ordering::Relaxed);
        serial::COM1.write(b"P4.3: task B observed PIT interval; step ");
        serial::COM1.write_hex(step as u64);
        serial::COM1.write(b".\r\n");
        if step < 3 {
            yield_current_accounted();
        }
    }
}

#[inline(never)]
extern "sysv64" fn task_a() {
    let mut local_probe = [0xA1A1_A1A1_A1A1_A1A1u64; 8];
    let local_pointer = local_probe.as_mut_ptr();
    unsafe { ptr::write_volatile(local_pointer.add(3), 0xA0A0_A0A0_A0A0_A0A0) };
    TASK_A_LOCAL_ADDRESS.store(local_pointer as usize, Ordering::Relaxed);
    TASK_A_PROGRESS.store(1, Ordering::Relaxed);
    serial::COM1.write(b"P4.2: task A entered on its private stack; step 1.\r\n");

    yield_current();
    let local_ok = unsafe { ptr::read_volatile(local_pointer.add(3)) == 0xA0A0_A0A0_A0A0_A0A0 };
    if !local_ok || TASK_B_PROGRESS.load(Ordering::Relaxed) != 1 {
        TASK_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    TASK_A_PROGRESS.store(2, Ordering::Relaxed);
    serial::COM1.write(b"P4.2: task A resumed after task B; step 2.\r\n");

    yield_current();
    let local_ok = unsafe { ptr::read_volatile(local_pointer.add(3)) == 0xA0A0_A0A0_A0A0_A0A0 };
    if !local_ok || TASK_B_PROGRESS.load(Ordering::Relaxed) != 2 {
        TASK_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    TASK_A_PROGRESS.store(3, Ordering::Relaxed);
    serial::COM1.write(b"P4.2: task A resumed again; returning.\r\n");
}

#[inline(never)]
extern "sysv64" fn task_b() {
    let mut local_probe = [0xB2B2_B2B2_B2B2_B2B2u64; 8];
    let local_pointer = local_probe.as_mut_ptr();
    unsafe { ptr::write_volatile(local_pointer.add(3), 0xB0B0_B0B0_B0B0_B0B0) };
    TASK_B_LOCAL_ADDRESS.store(local_pointer as usize, Ordering::Relaxed);
    TASK_B_PROGRESS.store(1, Ordering::Relaxed);
    serial::COM1.write(b"P4.2: task B entered on its private stack; step 1.\r\n");

    yield_current();
    let local_ok = unsafe { ptr::read_volatile(local_pointer.add(3)) == 0xB0B0_B0B0_B0B0_B0B0 };
    if !local_ok || TASK_A_PROGRESS.load(Ordering::Relaxed) != 2 {
        TASK_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    TASK_B_PROGRESS.store(2, Ordering::Relaxed);
    serial::COM1.write(b"P4.2: task B resumed after task A; step 2.\r\n");

    yield_current();
    let local_ok = unsafe { ptr::read_volatile(local_pointer.add(3)) == 0xB0B0_B0B0_B0B0_B0B0 };
    if !local_ok || TASK_A_PROGRESS.load(Ordering::Relaxed) != 3 {
        TASK_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    TASK_B_PROGRESS.store(3, Ordering::Relaxed);
    serial::COM1.write(b"P4.2: task B resumed again; returning.\r\n");
}

fn task_fatal(message: &'static [u8]) -> ! {
    serial::COM1.write(message);
    loop {
        unsafe { asm!("cli", "hlt", options(nomem, nostack)); }
    }
}

global_asm!(
    r#"
    .section .text
    .global exhi_context_switch
    .type exhi_context_switch, @function
exhi_context_switch:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15

    mov qword ptr [rdi], rsp
    mov rsp, rsi

    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret

    .global exhi_task_bootstrap
    .type exhi_task_bootstrap, @function
exhi_task_bootstrap:
    sub rsp, 8
    call r12
    call exhi_task_finished
    ud2
"#
);


#[cfg(feature = "timer-preemption-smoke-test")]
global_asm!(
    r#"
    .section .text
    .global exhi_irq0_preempt_stub
    .type exhi_irq0_preempt_stub, @function
exhi_irq0_preempt_stub:
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

    mov rax, qword ptr [rip + EXHIOS_ACTIVE_FX_STATE]
    test rax, rax
    jz .Lpreempt_irq_fatal
    fxsave64 [rax]

    lock inc qword ptr [rip + EXHIOS_PIT_TICKS]
    mov al, 0x20
    out 0x20, al

    mov rdi, rsp
    cld
    and rsp, -16
    call exhi_preemption_handler
    test rax, rax
    jz .Lpreempt_irq_fatal

    mov rcx, qword ptr [rip + EXHIOS_ACTIVE_FX_STATE]
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

.Lpreempt_irq_fatal:
    mov dx, 0xF4
    mov eax, 0x11
    out dx, eax
    cli
.Lpreempt_irq_halt:
    hlt
    jmp .Lpreempt_irq_halt

    .global exhi_interrupt_context_start
    .type exhi_interrupt_context_start, @function
exhi_interrupt_context_start:
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

    .global exhi_preemptive_task_bootstrap
    .type exhi_preemptive_task_bootstrap, @function
exhi_preemptive_task_bootstrap:
    call r12
    ud2

    .global exhi_preemptive_task_a
    .type exhi_preemptive_task_a, @function
exhi_preemptive_task_a:
    push rbp
    mov rbp, rsp
    sub rsp, 16
    movabs rax, 0xA1A1A1A1A1A1A1A1
    mov qword ptr [rbp - 8], rax
    lea rcx, [rbp - 8]
    mov qword ptr [rip + EXHIOS_P4_4_A_LOCAL], rcx
    movq xmm15, rax
.Lpreempt_a_loop:
    inc qword ptr [rip + EXHIOS_P4_4_A_PROGRESS]
    mov rdx, qword ptr [rip + EXHIOS_PIT_TICKS]
    cmp qword ptr [rip + EXHIOS_P4_4_FAILURES], 0
    jne .Lpreempt_a_fail
    cmp rdx, 80
    jae .Lpreempt_a_fail
    cmp rdx, 16
    jb .Lpreempt_a_loop
    mov rdx, qword ptr [rbp - 8]
    movabs rax, 0xA1A1A1A1A1A1A1A1
    cmp rdx, rax
    jne .Lpreempt_a_fail
    movq rdx, xmm15
    cmp rdx, rax
    jne .Lpreempt_a_fail
    mov qword ptr [rip + EXHIOS_P4_4_A_XMM_OK], 1
    cmp qword ptr [rip + EXHIOS_P4_4_SWITCHES], 12
    jb .Lpreempt_a_loop
    cmp qword ptr [rip + EXHIOS_P4_4_B_XMM_OK], 1
    jne .Lpreempt_a_loop
    call exhi_preemption_test_pass
    ud2
.Lpreempt_a_fail:
    mov qword ptr [rip + EXHIOS_P4_4_FAILURES], 1
    call exhi_preemption_test_fail
    ud2

    .global exhi_preemptive_task_b
    .type exhi_preemptive_task_b, @function
exhi_preemptive_task_b:
    push rbp
    mov rbp, rsp
    sub rsp, 16
    movabs rax, 0xB2B2B2B2B2B2B2B2
    mov qword ptr [rbp - 8], rax
    lea rcx, [rbp - 8]
    mov qword ptr [rip + EXHIOS_P4_4_B_LOCAL], rcx
    movq xmm15, rax
.Lpreempt_b_loop:
    inc qword ptr [rip + EXHIOS_P4_4_B_PROGRESS]
    mov rdx, qword ptr [rip + EXHIOS_PIT_TICKS]
    cmp rdx, 16
    jb .Lpreempt_b_loop
    mov rdx, qword ptr [rbp - 8]
    movabs rax, 0xB2B2B2B2B2B2B2B2
    cmp rdx, rax
    jne .Lpreempt_b_fail
    movq rdx, xmm15
    cmp rdx, rax
    jne .Lpreempt_b_fail
    mov qword ptr [rip + EXHIOS_P4_4_B_XMM_OK], 1
    jmp .Lpreempt_b_loop
.Lpreempt_b_fail:
    mov qword ptr [rip + EXHIOS_P4_4_FAILURES], 1
    jmp .Lpreempt_b_loop

    .global exhi_preemptive_task_exit_bootstrap
    .type exhi_preemptive_task_exit_bootstrap, @function
exhi_preemptive_task_exit_bootstrap:
    call r12
    call exhi_preemptive_task_finished
    ud2

    .global exhi_preemptive_task_exit_a
    .type exhi_preemptive_task_exit_a, @function
exhi_preemptive_task_exit_a:
    push rbp
    mov rbp, rsp
    sub rsp, 16
    movabs rax, 0xA1A1A1A1A1A1A1A1
    mov qword ptr [rbp - 8], rax
    lea rcx, [rbp - 8]
    mov qword ptr [rip + EXHIOS_P4_4_A_LOCAL], rcx
    movq xmm15, rax
.Lpreempt_exit_a_loop:
    inc qword ptr [rip + EXHIOS_P4_4_A_PROGRESS]
    mov rdx, qword ptr [rip + EXHIOS_PIT_TICKS]
    cmp qword ptr [rip + EXHIOS_P4_4_FAILURES], 0
    jne .Lpreempt_exit_a_fail
    cmp rdx, 80
    jae .Lpreempt_exit_a_fail
    cmp rdx, 16
    jb .Lpreempt_exit_a_loop
    mov rdx, qword ptr [rbp - 8]
    movabs rax, 0xA1A1A1A1A1A1A1A1
    cmp rdx, rax
    jne .Lpreempt_exit_a_fail
    movq rdx, xmm15
    cmp rdx, rax
    jne .Lpreempt_exit_a_fail
    mov qword ptr [rip + EXHIOS_P4_4_A_XMM_OK], 1
    cmp qword ptr [rip + EXHIOS_P4_4_SWITCHES], 12
    jb .Lpreempt_exit_a_loop
    leave
    ret
.Lpreempt_exit_a_fail:
    mov qword ptr [rip + EXHIOS_P4_4_FAILURES], 1
    call exhi_preemptive_task_exit_fail
    ud2

    .global exhi_preemptive_task_exit_b
    .type exhi_preemptive_task_exit_b, @function
exhi_preemptive_task_exit_b:
    push rbp
    mov rbp, rsp
    sub rsp, 16
    movabs rax, 0xB2B2B2B2B2B2B2B2
    mov qword ptr [rbp - 8], rax
    lea rcx, [rbp - 8]
    mov qword ptr [rip + EXHIOS_P4_4_B_LOCAL], rcx
    movq xmm15, rax
.Lpreempt_exit_b_loop:
    inc qword ptr [rip + EXHIOS_P4_4_B_PROGRESS]
    mov rdx, qword ptr [rip + EXHIOS_PIT_TICKS]
    cmp qword ptr [rip + EXHIOS_P4_4_FAILURES], 0
    jne .Lpreempt_exit_b_fail
    cmp rdx, 80
    jae .Lpreempt_exit_b_fail
    cmp rdx, 24
    jb .Lpreempt_exit_b_loop
    mov rdx, qword ptr [rbp - 8]
    movabs rax, 0xB2B2B2B2B2B2B2B2
    cmp rdx, rax
    jne .Lpreempt_exit_b_fail
    movq rdx, xmm15
    cmp rdx, rax
    jne .Lpreempt_exit_b_fail
    mov qword ptr [rip + EXHIOS_P4_4_B_XMM_OK], 1
    leave
    ret
.Lpreempt_exit_b_fail:
    mov qword ptr [rip + EXHIOS_P4_4_FAILURES], 1
    call exhi_preemptive_task_exit_fail
    ud2
"#
);
