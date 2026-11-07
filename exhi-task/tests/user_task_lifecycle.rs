use exhi_task::{ScheduleDecision, Scheduler, TaskContext, TaskState, UserTaskMetadata};

fn user_metadata(root: u64, stack: u64, kernel_stack: u64, fx: usize) -> UserTaskMetadata {
    UserTaskMetadata::new(root, stack, kernel_stack, fx)
}

#[test]
fn user_tasks_finish_into_idle_then_reaper_runs_and_reclaims_them() {
    let mut scheduler = Scheduler::<3>::new();
    let user_a = scheduler
        .spawn_user(
            "user-A",
            TaskContext::new(0x3000),
            user_metadata(0x2000, 0x2FF8, 0x4000, 0x5000),
        )
        .unwrap();
    let user_b = scheduler
        .spawn_user(
            "user-B",
            TaskContext::new(0x6000),
            user_metadata(0x7000, 0x7FF8, 0x8000, 0x9000),
        )
        .unwrap();
    let reaper = scheduler
        .spawn("kernel-reaper", TaskContext::new(0xC000))
        .unwrap();

    scheduler.block_ready(reaper).unwrap();
    assert_eq!(scheduler.start(), Ok(user_a));
    assert_eq!(
        scheduler.yield_current(),
        Ok(ScheduleDecision::Switch {
            from: user_a,
            to: user_b,
        })
    );
    assert_eq!(
        scheduler.yield_current(),
        Ok(ScheduleDecision::Switch {
            from: user_b,
            to: user_a,
        })
    );

    // The first exit hands off to the other user task. The second exit must
    // not resume either Finished task; it enters idle while the reaper sleeps.
    assert_eq!(scheduler.finish_current(), Ok(Some(user_b)));
    assert_eq!(scheduler.finish_current(), Ok(None));
    assert!(scheduler.is_idle());
    assert_eq!(scheduler.current(), None);
    assert_eq!(scheduler.finished_count(), 2);
    assert_eq!(scheduler.blocked_count(), 1);
    assert_eq!(scheduler.ready_count(), 0);

    scheduler.wake(reaper).unwrap();
    assert_eq!(scheduler.schedule_from_idle(), Ok(Some(reaper)));
    assert_eq!(scheduler.current().map(|task| task.id()), Some(reaper));
    assert_eq!(scheduler.get(user_a).map(|task| task.state()), Some(TaskState::Finished));
    assert_eq!(scheduler.get(user_b).map(|task| task.state()), Some(TaskState::Finished));

    // Reaping is legal only after execution has switched to the reaper.
    scheduler.reap(user_a).unwrap();
    scheduler.reap(user_b).unwrap();
    assert_eq!(scheduler.len(), 1);
    assert_eq!(scheduler.finished_count(), 0);
    assert_eq!(scheduler.current().map(|task| task.id()), Some(reaper));
}

#[test]
fn scheduler_never_reaps_a_running_task_or_a_non_finished_task() {
    let mut scheduler = Scheduler::<2>::new();
    let first = scheduler.spawn("first", TaskContext::new(0x1000)).unwrap();
    let second = scheduler.spawn("second", TaskContext::new(0x2000)).unwrap();

    assert!(scheduler.reap(first).is_err());
    assert_eq!(scheduler.len(), 2);
    assert_eq!(scheduler.start(), Ok(first));
    assert!(scheduler.reap(first).is_err());
    assert_eq!(scheduler.yield_current(), Ok(ScheduleDecision::Switch {
        from: first,
        to: second,
    }));
    assert_eq!(scheduler.finish_current(), Ok(Some(first)));
    assert!(scheduler.reap(second).is_err());
    scheduler.reap(second).unwrap_err();
    assert_eq!(scheduler.get(first).map(|task| task.state()), Some(TaskState::Finished));
    assert_eq!(scheduler.get(second), None);
}
