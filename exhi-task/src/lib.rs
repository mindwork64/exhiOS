#![no_std]

/// Monotonically assigned identifier within one Scheduler instance.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct TaskId(u64);

impl TaskId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Architecture-owned context. The scheduler policy treats RSP as opaque.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct TaskContext {
    saved_stack_pointer: usize,
}

impl TaskContext {
    pub const fn new(saved_stack_pointer: usize) -> Self {
        Self { saved_stack_pointer }
    }

    pub const fn saved_stack_pointer(self) -> usize {
        self.saved_stack_pointer
    }

    pub fn set_saved_stack_pointer(&mut self, saved_stack_pointer: usize) {
        self.saved_stack_pointer = saved_stack_pointer;
    }

    /// Returns the address of the saved RSP slot for an architecture context
    /// switch routine. The pointer is valid while this context remains inside
    /// its owning Scheduler and must not be retained across task-table moves.
    pub fn saved_stack_pointer_mut_ptr(&mut self) -> *mut usize {
        core::ptr::addr_of_mut!(self.saved_stack_pointer)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Finished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Task {
    id: TaskId,
    name: &'static str,
    state: TaskState,
    context: TaskContext,
    runtime_ticks: u64,
}

impl Task {
    pub const fn id(&self) -> TaskId { self.id }
    pub const fn name(&self) -> &'static str { self.name }
    pub const fn state(&self) -> TaskState { self.state }
    pub const fn context(&self) -> TaskContext { self.context }
    pub fn context_mut(&mut self) -> &mut TaskContext { &mut self.context }
    pub const fn runtime_ticks(&self) -> u64 { self.runtime_ticks }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    Full,
    InvalidContext,
    AlreadyStarted,
    NotStarted,
    NoRunnableTask,
    NotFound,
    InvalidStateTransition,
    IdExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleDecision {
    Switch { from: TaskId, to: TaskId },
    Continue { current: TaskId },
}

/// Fixed-capacity, allocation-free cooperative round-robin policy.
///
/// It manages metadata only: no kernel stacks are allocated and no CPU
/// registers are switched here. Callers serialize access; the first version
/// assumes a single CPU and Ring 0 tasks.
pub struct Scheduler<const CAPACITY: usize> {
    slots: [Option<Task>; CAPACITY],
    task_count: usize,
    next_id: u64,
    current_index: Option<usize>,
    idle_active: bool,
}

impl<const CAPACITY: usize> Scheduler<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            slots: [None; CAPACITY],
            task_count: 0,
            next_id: 1,
            current_index: None,
            idle_active: false,
        }
    }

    /// Adds a Ready task with a context prepared by the architecture layer.
    pub fn spawn(
        &mut self,
        name: &'static str,
        context: TaskContext,
    ) -> Result<TaskId, SchedulerError> {
        if context.saved_stack_pointer() == 0 {
            return Err(SchedulerError::InvalidContext);
        }
        let index = self.slots.iter().position(Option::is_none)
            .ok_or(SchedulerError::Full)?;
        if self.next_id == 0 {
            return Err(SchedulerError::IdExhausted);
        }

        let raw_id = self.next_id;
        self.next_id = raw_id.checked_add(1).unwrap_or(0);
        let id = TaskId(raw_id);
        self.slots[index] = Some(Task {
            id,
            name,
            state: TaskState::Ready,
            context,
            runtime_ticks: 0,
        });
        self.task_count += 1;
        Ok(id)
    }

    /// Starts the first Ready task.
    pub fn start(&mut self) -> Result<TaskId, SchedulerError> {
        if self.current_index.is_some() || self.idle_active {
            return Err(SchedulerError::AlreadyStarted);
        }
        let index = self.find_ready_from(0).ok_or(SchedulerError::NoRunnableTask)?;
        let task = self.slots[index].as_mut().expect("ready slot exists");
        task.state = TaskState::Running;
        let id = task.id;
        self.current_index = Some(index);
        Ok(id)
    }

    /// Performs a cooperative round-robin yield.
    pub fn yield_current(&mut self) -> Result<ScheduleDecision, SchedulerError> {
        self.reschedule_current()
    }

    /// Reschedules the running task from an asynchronous timer interrupt.
    /// The architecture layer saves the interrupted register frame first.
    pub fn preempt_current(&mut self) -> Result<ScheduleDecision, SchedulerError> {
        self.reschedule_current()
    }

    fn reschedule_current(&mut self) -> Result<ScheduleDecision, SchedulerError> {
        let current = self.current_index.ok_or(SchedulerError::NotStarted)?;
        let running = self.slots[current].as_ref().ok_or(SchedulerError::NotFound)?;
        if running.state != TaskState::Running {
            return Err(SchedulerError::InvalidStateTransition);
        }
        let from = running.id;
        let Some(next) = self.find_next_ready(current) else {
            return Ok(ScheduleDecision::Continue { current: from });
        };
        let to = self.slots[next].as_ref().expect("ready slot exists").id;
        self.slots[current].as_mut().expect("running slot exists").state = TaskState::Ready;
        self.slots[next].as_mut().expect("ready slot exists").state = TaskState::Running;
        self.current_index = Some(next);
        Ok(ScheduleDecision::Switch { from, to })
    }

    /// Finishes the current task and selects another Ready task, if any.
    pub fn finish_current(&mut self) -> Result<Option<TaskId>, SchedulerError> {
        let current = self.current_index.ok_or(SchedulerError::NotStarted)?;
        let running = self.slots[current].as_ref().ok_or(SchedulerError::NotFound)?;
        if running.state != TaskState::Running {
            return Err(SchedulerError::InvalidStateTransition);
        }

        let next = self.find_next_ready(current);
        self.slots[current].as_mut().expect("running slot exists").state = TaskState::Finished;

        if let Some(index) = next {
            let task = self.slots[index].as_mut().expect("ready slot exists");
            task.state = TaskState::Running;
            let id = task.id;
            self.current_index = Some(index);
            Ok(Some(id))
        } else {
            self.current_index = None;
            Ok(None)
        }
    }

    /// Blocks the current task and switches to a Ready peer.
    /// Failure leaves the current task Running when there is no peer.
    pub fn block_current(&mut self) -> Result<TaskId, SchedulerError> {
        let current = self.current_index.ok_or(SchedulerError::NotStarted)?;
        let running = self.slots[current].as_ref().ok_or(SchedulerError::NotFound)?;
        if running.state != TaskState::Running {
            return Err(SchedulerError::InvalidStateTransition);
        }
        let next = self.find_next_ready(current).ok_or(SchedulerError::NoRunnableTask)?;
        let next_id = self.slots[next].as_ref().expect("ready slot exists").id;

        self.slots[current].as_mut().expect("running slot exists").state = TaskState::Blocked;
        self.slots[next].as_mut().expect("ready slot exists").state = TaskState::Running;
        self.current_index = Some(next);
        Ok(next_id)
    }

    /// Blocks the current task and enters the architecture-owned idle state
    /// when no Ready peer exists. A None result means the caller must switch
    /// to its separate idle CPU context rather than restore a task.
    pub fn block_current_or_idle(&mut self) -> Result<Option<TaskId>, SchedulerError> {
        let current = self.current_index.ok_or(SchedulerError::NotStarted)?;
        let running = self.slots[current].as_ref().ok_or(SchedulerError::NotFound)?;
        if running.state != TaskState::Running {
            return Err(SchedulerError::InvalidStateTransition);
        }

        let next = self.find_next_ready(current);
        self.slots[current].as_mut().expect("running slot exists").state = TaskState::Blocked;
        if let Some(index) = next {
            let task = self.slots[index].as_mut().expect("ready slot exists");
            task.state = TaskState::Running;
            let id = task.id;
            self.current_index = Some(index);
            self.idle_active = false;
            Ok(Some(id))
        } else {
            self.current_index = None;
            self.idle_active = true;
            Ok(None)
        }
    }

    /// Enters idle after a running task finishes while peers remain Blocked.
    /// There must be no current or Ready task; at least one task must be Blocked.
    pub fn enter_idle(&mut self) -> Result<(), SchedulerError> {
        if self.current_index.is_some()
            || self.idle_active
            || self.ready_count() != 0
            || self.blocked_count() == 0
        {
            return Err(SchedulerError::InvalidStateTransition);
        }
        self.idle_active = true;
        Ok(())
    }

    /// Selects the next Ready task after an interrupt wakes the CPU from idle.
    /// Returns Ok(None) when no task is Ready and the CPU must remain idle.
    pub fn schedule_from_idle(&mut self) -> Result<Option<TaskId>, SchedulerError> {
        if !self.idle_active || self.current_index.is_some() {
            return Err(SchedulerError::InvalidStateTransition);
        }
        let Some(index) = self.find_ready_from(0) else {
            return Ok(None);
        };
        let task = self.slots[index].as_mut().expect("ready slot exists");
        task.state = TaskState::Running;
        let id = task.id;
        self.current_index = Some(index);
        self.idle_active = false;
        Ok(Some(id))
    }

    /// True while the CPU is executing the architecture-owned idle context.
    pub const fn is_idle(&self) -> bool {
        self.idle_active
    }

    /// Wakes a Blocked task, making it Ready.
    pub fn wake(&mut self, id: TaskId) -> Result<(), SchedulerError> {
        let index = self.find_index(id).ok_or(SchedulerError::NotFound)?;
        let task = self.slots[index].as_mut().expect("located slot exists");
        if task.state != TaskState::Blocked {
            return Err(SchedulerError::InvalidStateTransition);
        }
        task.state = TaskState::Ready;
        Ok(())
    }

    /// Reclaims a Finished task slot. Its stack must be released by the
    /// architecture layer only after execution has switched away from it.
    pub fn reap(&mut self, id: TaskId) -> Result<(), SchedulerError> {
        let index = self.find_index(id).ok_or(SchedulerError::NotFound)?;
        let task = self.slots[index].as_ref().expect("located slot exists");
        if self.current_index == Some(index) || task.state != TaskState::Finished {
            return Err(SchedulerError::InvalidStateTransition);
        }
        self.slots[index] = None;
        self.task_count -= 1;
        Ok(())
    }

    /// Adds elapsed PIT ticks to the currently running task.
    ///
    /// The architecture layer calls this only at a safe cooperative
    /// scheduling boundary; the IRQ handler itself never accesses Scheduler.
    pub fn account_current_ticks(&mut self, elapsed_ticks: u64) -> Result<u64, SchedulerError> {
        let current = self.current_index.ok_or(SchedulerError::NotStarted)?;
        let task = self.slots[current].as_mut().ok_or(SchedulerError::NotFound)?;
        if task.state != TaskState::Running {
            return Err(SchedulerError::InvalidStateTransition);
        }
        task.runtime_ticks = task.runtime_ticks.saturating_add(elapsed_ticks);
        Ok(task.runtime_ticks)
    }

    pub fn current(&self) -> Option<&Task> {
        self.current_index.and_then(|index| self.slots[index].as_ref())
    }

    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.find_index(id).and_then(|index| self.slots[index].as_ref())
    }

    pub fn get_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        let index = self.find_index(id)?;
        self.slots[index].as_mut()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.slots.iter().filter_map(Option::as_ref)
    }

    pub const fn len(&self) -> usize { self.task_count }
    pub const fn capacity(&self) -> usize { CAPACITY }
    pub const fn is_empty(&self) -> bool { self.task_count == 0 }

    pub fn ready_count(&self) -> usize { self.count_state(TaskState::Ready) }
    pub fn blocked_count(&self) -> usize { self.count_state(TaskState::Blocked) }
    pub fn finished_count(&self) -> usize { self.count_state(TaskState::Finished) }

    fn count_state(&self, state: TaskState) -> usize {
        self.iter().filter(|task| task.state == state).count()
    }

    fn find_index(&self, id: TaskId) -> Option<usize> {
        self.slots.iter().position(|slot| slot.as_ref().is_some_and(|task| task.id == id))
    }

    fn find_ready_from(&self, start: usize) -> Option<usize> {
        if CAPACITY == 0 { return None; }
        (0..CAPACITY)
            .map(|distance| (start + distance) % CAPACITY)
            .find(|&index| self.slots[index].as_ref().is_some_and(|task| task.state == TaskState::Ready))
    }

    fn find_next_ready(&self, current: usize) -> Option<usize> {
        if CAPACITY < 2 { return None; }
        (1..CAPACITY)
            .map(|distance| (current + distance) % CAPACITY)
            .find(|&index| self.slots[index].as_ref().is_some_and(|task| task.state == TaskState::Ready))
    }
}

impl<const CAPACITY: usize> Default for Scheduler<CAPACITY> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(pointer: usize) -> TaskContext { TaskContext::new(pointer) }

    #[test]
    fn starts_first_ready_task() {
        let mut scheduler = Scheduler::<3>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        scheduler.spawn("second", context(0x2000)).unwrap();
        assert_eq!(scheduler.start(), Ok(first));
        assert_eq!(scheduler.current().map(Task::id), Some(first));
        assert_eq!(scheduler.get(first).map(Task::state), Some(TaskState::Running));
    }

    #[test]
    fn round_robins_ready_tasks() {
        let mut scheduler = Scheduler::<3>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        let second = scheduler.spawn("second", context(0x2000)).unwrap();
        let third = scheduler.spawn("third", context(0x3000)).unwrap();
        assert_eq!(scheduler.start(), Ok(first));
        assert_eq!(scheduler.yield_current(), Ok(ScheduleDecision::Switch { from: first, to: second }));
        assert_eq!(scheduler.yield_current(), Ok(ScheduleDecision::Switch { from: second, to: third }));
        assert_eq!(scheduler.yield_current(), Ok(ScheduleDecision::Switch { from: third, to: first }));
    }

    #[test]
    fn a_single_task_keeps_running_when_it_yields() {
        let mut scheduler = Scheduler::<1>::new();
        let only = scheduler.spawn("only", context(0x1000)).unwrap();
        scheduler.start().unwrap();
        assert_eq!(scheduler.yield_current(), Ok(ScheduleDecision::Continue { current: only }));
        assert_eq!(scheduler.current().map(Task::state), Some(TaskState::Running));
    }

    #[test]
    fn finishing_tasks_never_reschedules_them() {
        let mut scheduler = Scheduler::<2>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        let second = scheduler.spawn("second", context(0x2000)).unwrap();
        scheduler.start().unwrap();
        assert_eq!(scheduler.finish_current(), Ok(Some(second)));
        assert_eq!(scheduler.get(first).map(Task::state), Some(TaskState::Finished));
        assert_eq!(scheduler.finish_current(), Ok(None));
        assert_eq!(scheduler.current(), None);
        assert_eq!(scheduler.finished_count(), 2);
    }

    #[test]
    fn blocks_and_wakes_tasks() {
        let mut scheduler = Scheduler::<2>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        let second = scheduler.spawn("second", context(0x2000)).unwrap();
        scheduler.start().unwrap();
        assert_eq!(scheduler.block_current(), Ok(second));
        assert_eq!(scheduler.get(first).map(Task::state), Some(TaskState::Blocked));
        assert_eq!(scheduler.wake(first), Ok(()));
        assert_eq!(scheduler.yield_current(), Ok(ScheduleDecision::Switch { from: second, to: first }));
    }

    #[test]
    fn cannot_block_the_only_runnable_task() {
        let mut scheduler = Scheduler::<1>::new();
        let only = scheduler.spawn("only", context(0x1000)).unwrap();
        scheduler.start().unwrap();
        assert_eq!(scheduler.block_current(), Err(SchedulerError::NoRunnableTask));
        assert_eq!(scheduler.current().map(Task::id), Some(only));
    }

    #[test]
    fn rejects_unknown_and_invalid_wake_transitions() {
        let mut scheduler = Scheduler::<2>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        assert_eq!(scheduler.wake(TaskId(999)), Err(SchedulerError::NotFound));
        assert_eq!(scheduler.wake(first), Err(SchedulerError::InvalidStateTransition));
        scheduler.spawn("second", context(0x2000)).unwrap();
        scheduler.start().unwrap();
        scheduler.block_current().unwrap();
        scheduler.finish_current().unwrap();
        assert_eq!(scheduler.wake(first), Ok(()));
    }

    #[test]
    fn last_runnable_task_blocks_into_idle_and_resumes_after_wake() {
        let mut scheduler = Scheduler::<1>::new();
        let only = scheduler.spawn("only", context(0x1000)).unwrap();
        assert_eq!(scheduler.start(), Ok(only));
        assert_eq!(scheduler.block_current_or_idle(), Ok(None));
        assert!(scheduler.is_idle());
        assert_eq!(scheduler.current(), None);
        assert_eq!(scheduler.blocked_count(), 1);
        assert_eq!(scheduler.schedule_from_idle(), Ok(None));
        assert_eq!(scheduler.wake(only), Ok(()));
        assert_eq!(scheduler.schedule_from_idle(), Ok(Some(only)));
        assert!(!scheduler.is_idle());
        assert_eq!(scheduler.current().map(Task::id), Some(only));
        assert_eq!(scheduler.get(only).map(Task::state), Some(TaskState::Running));
    }

    #[test]
    fn finishing_last_running_task_can_enter_idle_for_blocked_peer() {
        let mut scheduler = Scheduler::<2>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        let second = scheduler.spawn("second", context(0x2000)).unwrap();
        assert_eq!(scheduler.start(), Ok(first));
        assert_eq!(scheduler.block_current_or_idle(), Ok(Some(second)));
        assert_eq!(scheduler.finish_current(), Ok(None));
        assert_eq!(scheduler.current(), None);
        assert_eq!(scheduler.blocked_count(), 1);
        assert_eq!(scheduler.enter_idle(), Ok(()));
        assert!(scheduler.is_idle());
        assert_eq!(scheduler.wake(second), Err(SchedulerError::InvalidStateTransition));
        assert_eq!(scheduler.wake(first), Ok(()));
        assert_eq!(scheduler.schedule_from_idle(), Ok(Some(first)));
        assert_eq!(scheduler.current().map(Task::id), Some(first));
    }

    #[test]
    fn idle_state_rejects_invalid_entry_and_dispatch() {
        let mut scheduler = Scheduler::<2>::new();
        assert_eq!(scheduler.enter_idle(), Err(SchedulerError::InvalidStateTransition));
        assert_eq!(scheduler.schedule_from_idle(), Err(SchedulerError::InvalidStateTransition));
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        scheduler.start().unwrap();
        assert_eq!(scheduler.enter_idle(), Err(SchedulerError::InvalidStateTransition));
        assert_eq!(scheduler.block_current_or_idle(), Ok(None));
        assert_eq!(scheduler.block_current_or_idle(), Err(SchedulerError::NotStarted));
        assert_eq!(scheduler.enter_idle(), Err(SchedulerError::InvalidStateTransition));
        assert_eq!(scheduler.wake(first), Ok(()));
        assert_eq!(scheduler.schedule_from_idle(), Ok(Some(first)));
    }

    #[test]
    fn finished_slots_can_be_reaped_without_reusing_ids() {
        let mut scheduler = Scheduler::<1>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        scheduler.start().unwrap();
        assert_eq!(scheduler.finish_current(), Ok(None));
        scheduler.reap(first).unwrap();
        let second = scheduler.spawn("second", context(0x2000)).unwrap();
        assert_ne!(first, second);
        assert_eq!(scheduler.len(), 1);
    }

    #[test]
    fn enforces_capacity_and_rejects_zero_stack_pointer() {
        let mut scheduler = Scheduler::<1>::new();
        assert_eq!(scheduler.spawn("invalid", context(0)), Err(SchedulerError::InvalidContext));
        scheduler.spawn("first", context(0x1000)).unwrap();
        assert_eq!(scheduler.spawn("second", context(0x2000)), Err(SchedulerError::Full));
    }

    #[test]
    fn handles_zero_capacity() {
        let mut scheduler = Scheduler::<0>::new();
        assert_eq!(scheduler.start(), Err(SchedulerError::NoRunnableTask));
        assert_eq!(scheduler.capacity(), 0);
    }

    #[test]
    fn reports_lifecycle_counts() {
        let mut scheduler = Scheduler::<3>::new();
        let first = scheduler.spawn("one", context(0x1000)).unwrap();
        scheduler.spawn("two", context(0x2000)).unwrap();
        scheduler.spawn("three", context(0x3000)).unwrap();
        scheduler.start().unwrap();
        scheduler.block_current().unwrap();
        assert_eq!(scheduler.ready_count(), 1);
        assert_eq!(scheduler.blocked_count(), 1);
        assert_eq!(scheduler.finished_count(), 0);
        assert!(scheduler.get(first).is_some());
    }

    #[test]
    fn timer_preemption_round_robins_without_a_cooperative_yield() {
        let mut scheduler = Scheduler::<2>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        let second = scheduler.spawn("second", context(0x2000)).unwrap();
        assert_eq!(scheduler.start(), Ok(first));
        assert_eq!(
            scheduler.preempt_current(),
            Ok(ScheduleDecision::Switch { from: first, to: second })
        );
        assert_eq!(scheduler.current().map(Task::id), Some(second));
        assert_eq!(scheduler.get(first).map(Task::state), Some(TaskState::Ready));
        assert_eq!(scheduler.get(second).map(Task::state), Some(TaskState::Running));
        assert_eq!(
            scheduler.preempt_current(),
            Ok(ScheduleDecision::Switch { from: second, to: first })
        );
        assert_eq!(scheduler.current().map(Task::id), Some(first));
    }

    #[test]
    fn accounts_runtime_ticks_to_each_task_independently() {
        let mut scheduler = Scheduler::<2>::new();
        let first = scheduler.spawn("first", context(0x1000)).unwrap();
        let second = scheduler.spawn("second", context(0x2000)).unwrap();
        assert_eq!(scheduler.start(), Ok(first));
        assert_eq!(scheduler.account_current_ticks(7), Ok(7));
        assert_eq!(scheduler.yield_current(), Ok(ScheduleDecision::Switch {
            from: first,
            to: second,
        }));
        assert_eq!(scheduler.account_current_ticks(3), Ok(3));
        assert_eq!(scheduler.get(first).map(Task::runtime_ticks), Some(7));
        assert_eq!(scheduler.get(second).map(Task::runtime_ticks), Some(3));
    }

    #[test]
    fn runtime_accounting_saturates_instead_of_wrapping() {
        let mut scheduler = Scheduler::<1>::new();
        let only = scheduler.spawn("only", context(0x1000)).unwrap();
        scheduler.start().unwrap();
        assert_eq!(scheduler.account_current_ticks(u64::MAX - 1), Ok(u64::MAX - 1));
        assert_eq!(scheduler.account_current_ticks(10), Ok(u64::MAX));
        assert_eq!(scheduler.get(only).map(Task::runtime_ticks), Some(u64::MAX));
    }

    #[test]
    fn runtime_accounting_requires_a_running_task() {
        let mut scheduler = Scheduler::<1>::new();
        assert_eq!(scheduler.account_current_ticks(1), Err(SchedulerError::NotStarted));
        let only = scheduler.spawn("only", context(0x1000)).unwrap();
        scheduler.start().unwrap();
        assert_eq!(scheduler.finish_current(), Ok(None));
        assert_eq!(scheduler.account_current_ticks(1), Err(SchedulerError::NotStarted));
        assert_eq!(scheduler.get(only).map(Task::runtime_ticks), Some(0));
    }
}
