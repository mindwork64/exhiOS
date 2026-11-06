//! x86-64 cooperative context switching and a two-task QEMU smoke test.
//!
//! The first implementation uses static kernel stacks with one unmapped guard
//! page each. Switching is cooperative and single-CPU; IRQ0 never enters the
//! scheduler. Tasks remain in Ring 0 and share the kernel address space.

use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

use exhi_task::{ScheduleDecision, Scheduler, TaskContext, TaskId};

use super::{gdt, paging, serial, timer, vmm};

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
    // SAFETY: The PIT is programmed, IRQ0 has a valid IDT gate and the PIC
    // unmasks only IRQ0. The ISR only updates the atomic tick count and EOI.
    unsafe {
        asm!("sti", options(nomem, nostack, preserves_flags));
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
    if !account_current_runtime() {
        task_fatal(b"P4.3: could not account current task runtime.\r\n");
    }
    yield_current();
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
