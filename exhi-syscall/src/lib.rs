#![no_std]

//! Capability-gated syscall dispatch policy.
//!
//! This crate owns the syscall-to-capability mapping and generic service
//! dispatch. Architecture code supplies the current-task and service adapter;
//! it remains responsible for validating the interrupt frame and safely
//! copying user memory before touching a service buffer.

use exhi_abi::{SyscallError, SyscallNumber, SyscallRequest};
use exhi_task::{Capability, CapabilitySet};

/// Result of dispatching a syscall. A switch target is opaque to this crate
/// and is interpreted only by the architecture-specific context-switch code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchOutcome<T> {
    /// Resume the current task with this value in RAX.
    Return(u64),
    /// The service selected a different task; restore its saved context.
    Switch(T),
}

/// Kernel services required by the stable syscall ABI.
///
/// Implementations must not assume that a syscall has been authorized until
/// the dispatcher invokes the matching method. The dispatcher always checks
/// the capability before calling a service method.
pub trait SyscallServices {
    type SwitchTarget;

    /// Returns the effective grants of the currently running process.
    /// A kernel task or missing current task must return CapabilitySet::EMPTY.
    fn current_capabilities(&self) -> CapabilitySet;

    /// Yields the current task. None means the scheduler kept it running.
    fn yield_current(&mut self) -> Result<Option<Self::SwitchTarget>, SyscallError>;

    /// Finishes the current task and selects the next kernel/user context.
    fn exit_current(&mut self) -> Result<Self::SwitchTarget, SyscallError>;

    /// Writes a user-owned byte range to the kernel console.
    ///
    /// The adapter must validate the complete user range and copy it into
    /// kernel-owned memory before performing device I/O.
    fn console_write(&mut self, user_address: u64, length: u64) -> Result<u64, SyscallError>;

    /// Reads available console bytes into a user-owned writable range.
    ///
    /// The operation is non-blocking: zero means no input is currently ready.
    /// The adapter must validate the entire destination before consuming input.
    fn console_read(&mut self, user_address: u64, length: u64) -> Result<u64, SyscallError>;

    /// Reads a file into a user-owned buffer after validating both user ranges.
    fn file_read(
        &mut self,
        path_address: u64,
        path_length: u64,
        offset: u64,
        user_buffer: u64,
        buffer_length: u64,
    ) -> Result<u64, SyscallError>;
}

/// Dispatches one ABI request with default-deny capability enforcement.
///
/// Unknown syscall numbers return NotImplemented. Known calls are authorized
/// before their service methods run, so denied calls cannot cause service
/// side effects or access user pointers.
pub fn dispatch<S: SyscallServices>(
    request: SyscallRequest,
    services: &mut S,
) -> DispatchOutcome<S::SwitchTarget> {
    let syscall = match SyscallNumber::try_from(request.number) {
        Ok(syscall) => syscall,
        Err(_) => {
            return DispatchOutcome::Return(SyscallError::NotImplemented.as_return_value());
        }
    };

    let required_capability = match syscall {
        SyscallNumber::Yield => Capability::TaskYield,
        SyscallNumber::Exit => Capability::TaskExit,
        SyscallNumber::ConsoleWrite => Capability::ConsoleWrite,
        SyscallNumber::ConsoleRead => Capability::ConsoleRead,
        SyscallNumber::FileRead => Capability::FileRead,
    };
    if services
        .current_capabilities()
        .authorize(required_capability)
        .is_err()
    {
        return DispatchOutcome::Return(SyscallError::AccessDenied.as_return_value());
    }

    match syscall {
        SyscallNumber::Yield => match services.yield_current() {
            Ok(Some(target)) => DispatchOutcome::Switch(target),
            Ok(None) => DispatchOutcome::Return(0),
            Err(error) => DispatchOutcome::Return(error.as_return_value()),
        },
        SyscallNumber::Exit => match services.exit_current() {
            Ok(target) => DispatchOutcome::Switch(target),
            Err(error) => DispatchOutcome::Return(error.as_return_value()),
        },
        SyscallNumber::ConsoleWrite => {
            let [address, length, ..] = request.arguments;
            match services.console_write(address, length) {
                Ok(written) => DispatchOutcome::Return(written),
                Err(error) => DispatchOutcome::Return(error.as_return_value()),
            }
        }
        SyscallNumber::ConsoleRead => {
            let [address, length, ..] = request.arguments;
            match services.console_read(address, length) {
                Ok(read) => DispatchOutcome::Return(read),
                Err(error) => DispatchOutcome::Return(error.as_return_value()),
            }
        }
        SyscallNumber::FileRead => {
            let [path, path_length, offset, buffer, buffer_length, ..] = request.arguments;
            match services.file_read(path, path_length, offset, buffer, buffer_length) {
                Ok(read) => DispatchOutcome::Return(read),
                Err(error) => DispatchOutcome::Return(error.as_return_value()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct MockServices {
        capabilities: exhi_task::CapabilitySet,
        yield_target: Option<u64>,
        exit_target: Option<u64>,
        console_result: Result<u64, SyscallError>,
        console_read_result: Result<u64, SyscallError>,
        file_read_result: Result<u64, SyscallError>,
        yield_calls: u8,
        exit_calls: u8,
        console_calls: u8,
        console_read_calls: u8,
        file_read_calls: u8,
        last_console_range: Option<(u64, u64)>,
        last_console_read_range: Option<(u64, u64)>,
        last_file_read: Option<(u64, u64, u64, u64, u64)>,
    }

    impl MockServices {
        const fn new(capabilities: exhi_task::CapabilitySet) -> Self {
            Self {
                capabilities,
                yield_target: None,
                exit_target: Some(3),
                console_result: Ok(0),
                console_read_result: Ok(0),
                file_read_result: Ok(0),
                yield_calls: 0,
                exit_calls: 0,
                console_calls: 0,
                console_read_calls: 0,
                file_read_calls: 0,
                last_console_range: None,
                last_console_read_range: None,
                last_file_read: None,
            }
        }
    }

    impl SyscallServices for MockServices {
        type SwitchTarget = u64;

        fn current_capabilities(&self) -> CapabilitySet {
            self.capabilities
        }

        fn yield_current(&mut self) -> Result<Option<Self::SwitchTarget>, SyscallError> {
            self.yield_calls += 1;
            Ok(self.yield_target)
        }

        fn exit_current(&mut self) -> Result<Self::SwitchTarget, SyscallError> {
            self.exit_calls += 1;
            self.exit_target.ok_or(SyscallError::Io)
        }

        fn console_write(&mut self, user_address: u64, length: u64) -> Result<u64, SyscallError> {
            self.console_calls += 1;
            self.last_console_range = Some((user_address, length));
            self.console_result
        }

        fn console_read(&mut self, user_address: u64, length: u64) -> Result<u64, SyscallError> {
            self.console_read_calls += 1;
            self.last_console_read_range = Some((user_address, length));
            self.console_read_result
        }

        fn file_read(
            &mut self,
            path_address: u64,
            path_length: u64,
            offset: u64,
            user_buffer: u64,
            buffer_length: u64,
        ) -> Result<u64, SyscallError> {
            self.file_read_calls += 1;
            self.last_file_read = Some((
                path_address,
                path_length,
                offset,
                user_buffer,
                buffer_length,
            ));
            self.file_read_result
        }
    }

    #[test]
    fn denied_console_write_never_enters_the_service() {
        let mut services = MockServices::new(exhi_task::CapabilitySet::empty());
        let request =
            SyscallRequest::new(SyscallNumber::ConsoleWrite as u64, [0x2000, 12, 0, 0, 0, 0]);

        assert_eq!(
            dispatch(request, &mut services),
            DispatchOutcome::Return(SyscallError::AccessDenied.as_return_value())
        );
        assert_eq!(services.console_calls, 0);
        assert_eq!(services.last_console_range, None);
    }

    #[test]
    fn kernel_default_empty_grants_deny_every_user_service() {
        let cases = [
            SyscallNumber::Yield,
            SyscallNumber::Exit,
            SyscallNumber::ConsoleWrite,
            SyscallNumber::ConsoleRead,
            SyscallNumber::FileRead,
        ];

        for syscall in cases {
            let mut services = MockServices::new(exhi_task::CapabilitySet::EMPTY);
            let request = SyscallRequest::new(syscall as u64, [0; 6]);
            assert_eq!(
                dispatch(request, &mut services),
                DispatchOutcome::Return(SyscallError::AccessDenied.as_return_value())
            );
            assert_eq!(services.yield_calls, 0);
            assert_eq!(services.exit_calls, 0);
            assert_eq!(services.console_calls, 0);
            assert_eq!(services.console_read_calls, 0);
            assert_eq!(services.file_read_calls, 0);
        }
    }

    #[test]
    fn denied_console_read_never_enters_the_service() {
        let mut services = MockServices::new(CapabilitySet::EMPTY);
        let request = SyscallRequest::new(
            SyscallNumber::ConsoleRead as u64,
            [0x0000_0002_0000_0080, 32, 0, 0, 0, 0],
        );
        assert_eq!(
            dispatch(request, &mut services),
            DispatchOutcome::Return(SyscallError::AccessDenied.as_return_value())
        );
        assert_eq!(services.console_read_calls, 0);
        assert_eq!(services.last_console_read_range, None);
    }

    #[test]
    fn authorized_console_read_receives_the_user_buffer_and_returns_available_bytes() {
        let capabilities = CapabilitySet::EMPTY.with(Capability::ConsoleRead);
        let mut services = MockServices::new(capabilities);
        services.console_read_result = Ok(3);
        let request = SyscallRequest::new(
            SyscallNumber::ConsoleRead as u64,
            [0x0000_0002_0000_0080, 32, 0, 0, 0, 0],
        );
        assert_eq!(dispatch(request, &mut services), DispatchOutcome::Return(3));
        assert_eq!(services.console_read_calls, 1);
        assert_eq!(
            services.last_console_read_range,
            Some((0x0000_0002_0000_0080, 32))
        );
    }

    #[test]
    fn authorized_console_write_receives_the_abi_range_and_returns_byte_count() {
        let capabilities = exhi_task::CapabilitySet::empty().with(Capability::ConsoleWrite);
        let mut services = MockServices::new(capabilities);
        services.console_result = Ok(12);
        let request = SyscallRequest::new(
            SyscallNumber::ConsoleWrite as u64,
            [0x0000_0002_0000_0080, 12, 0x33, 0x44, 0x55, 0x66],
        );

        assert_eq!(
            dispatch(request, &mut services),
            DispatchOutcome::Return(12)
        );
        assert_eq!(services.console_calls, 1);
        assert_eq!(
            services.last_console_range,
            Some((0x0000_0002_0000_0080, 12))
        );
    }

    #[test]
    fn denied_file_read_never_enters_the_service() {
        let mut services = MockServices::new(CapabilitySet::EMPTY);
        let request = SyscallRequest::new(
            SyscallNumber::FileRead as u64,
            [0x2000, 13, 0, 0x3000, 64, 0],
        );
        assert_eq!(
            dispatch(request, &mut services),
            DispatchOutcome::Return(SyscallError::AccessDenied.as_return_value())
        );
        assert_eq!(services.file_read_calls, 0);
        assert_eq!(services.last_file_read, None);
    }

    #[test]
    fn authorized_file_read_receives_all_abi_ranges() {
        let capabilities = CapabilitySet::EMPTY.with(Capability::FileRead);
        let mut services = MockServices::new(capabilities);
        services.file_read_result = Ok(17);
        let request = SyscallRequest::new(
            SyscallNumber::FileRead as u64,
            [0x2000, 13, 4, 0x3000, 64, 0],
        );
        assert_eq!(dispatch(request, &mut services), DispatchOutcome::Return(17));
        assert_eq!(services.file_read_calls, 1);
        assert_eq!(services.last_file_read, Some((0x2000, 13, 4, 0x3000, 64)));
    }

    #[test]
    fn yield_returns_zero_when_scheduler_keeps_the_current_task() {
        let capabilities = exhi_task::CapabilitySet::empty().with(Capability::TaskYield);
        let mut services = MockServices::new(capabilities);
        services.yield_target = None;

        assert_eq!(
            dispatch(
                SyscallRequest::new(SyscallNumber::Yield as u64, [0; 6]),
                &mut services
            ),
            DispatchOutcome::Return(0)
        );
        assert_eq!(services.yield_calls, 1);
    }

    #[test]
    fn yield_and_exit_return_opaque_switch_targets() {
        let capabilities = exhi_task::CapabilitySet::empty()
            .with(Capability::TaskYield)
            .with(Capability::TaskExit);
        let mut services = MockServices::new(capabilities);
        services.yield_target = Some(2);
        services.exit_target = Some(3);

        assert_eq!(
            dispatch(
                SyscallRequest::new(SyscallNumber::Yield as u64, [0; 6]),
                &mut services
            ),
            DispatchOutcome::Switch(2)
        );
        assert_eq!(
            dispatch(
                SyscallRequest::new(SyscallNumber::Exit as u64, [0; 6]),
                &mut services
            ),
            DispatchOutcome::Switch(3)
        );
        assert_eq!(services.yield_calls, 1);
        assert_eq!(services.exit_calls, 1);
    }

    #[test]
    fn unknown_syscall_is_not_implemented_and_has_no_service_side_effects() {
        let mut services = MockServices::new(exhi_task::CapabilitySet::EMPTY);

        assert_eq!(
            dispatch(SyscallRequest::new(0xFFFF, [0; 6]), &mut services),
            DispatchOutcome::Return(SyscallError::NotImplemented.as_return_value())
        );
        assert_eq!(services.yield_calls, 0);
        assert_eq!(services.exit_calls, 0);
        assert_eq!(services.console_calls, 0);
        assert_eq!(services.console_read_calls, 0);
    }

    #[test]
    fn service_errors_are_returned_without_a_context_switch() {
        let capabilities = exhi_task::CapabilitySet::empty().with(Capability::ConsoleWrite);
        let mut services = MockServices::new(capabilities);
        services.console_result = Err(SyscallError::InvalidArgument);

        assert_eq!(
            dispatch(
                SyscallRequest::new(SyscallNumber::ConsoleWrite as u64, [0x2000, 4, 0, 0, 0, 0]),
                &mut services
            ),
            DispatchOutcome::Return(SyscallError::InvalidArgument.as_return_value())
        );
    }
}
