//! Stable x86-64 syscall numbers, register convention and return statuses.
//!
//! The syscall number is passed in RAX. Up to six arguments are passed in
//! RDI, RSI, RDX, R10, R8 and R9, in that order. RCX and R11 are clobbered by
//! the SYSCALL instruction and are not argument registers.

/// Number of argument registers in the ExhiOS syscall ABI.
pub const SYSCALL_ARGUMENT_COUNT: usize = 6;

/// Register names in the order used by SyscallRequest::arguments.
pub const SYSCALL_ARGUMENT_REGISTERS: [&str; SYSCALL_ARGUMENT_COUNT] =
    ["RDI", "RSI", "RDX", "R10", "R8", "R9"];

/// System-call numbers are stable ABI values. Unassigned values are rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum SyscallNumber {
    Yield = 1,
    Exit = 2,
    ConsoleWrite = 3,
    ConsoleRead = 4,
    FileRead = 5,
    SpawnWorker = 6,
}

impl core::convert::TryFrom<u64> for SyscallNumber {
    type Error = UnknownSyscallNumber;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            value if value == Self::Yield as u64 => Ok(Self::Yield),
            value if value == Self::Exit as u64 => Ok(Self::Exit),
            value if value == Self::ConsoleWrite as u64 => Ok(Self::ConsoleWrite),
            value if value == Self::ConsoleRead as u64 => Ok(Self::ConsoleRead),
            value if value == Self::FileRead as u64 => Ok(Self::FileRead),
            value if value == Self::SpawnWorker as u64 => Ok(Self::SpawnWorker),
            other => Err(UnknownSyscallNumber(other)),
        }
    }
}

/// A raw syscall request captured from the defined register convention.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SyscallRequest {
    pub number: u64,
    pub arguments: [u64; SYSCALL_ARGUMENT_COUNT],
}

impl SyscallRequest {
    pub const fn new(number: u64, arguments: [u64; SYSCALL_ARGUMENT_COUNT]) -> Self {
        Self { number, arguments }
    }

    /// Captures the stable x86-64 syscall argument register order.
    pub const fn from_registers(
        number: u64,
        rdi: u64,
        rsi: u64,
        rdx: u64,
        r10: u64,
        r8: u64,
        r9: u64,
    ) -> Self {
        Self {
            number,
            arguments: [rdi, rsi, rdx, r10, r8, r9],
        }
    }
}

/// Error values are returned in RAX using sign-extended negative integers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub enum SyscallError {
    AccessDenied = -1,
    InvalidArgument = -2,
    NotImplemented = -3,
    Io = -4,
}

impl SyscallError {
    pub const fn as_return_value(self) -> u64 {
        self as i64 as u64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownSyscallNumber(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syscall_numbers_are_stable_and_unknown_values_are_rejected() {
        assert_eq!(SyscallNumber::Yield as u64, 1);
        assert_eq!(SyscallNumber::Exit as u64, 2);
        assert_eq!(SyscallNumber::ConsoleWrite as u64, 3);
        assert_eq!(SyscallNumber::ConsoleRead as u64, 4);
        assert_eq!(SyscallNumber::FileRead as u64, 5);
        assert_eq!(SyscallNumber::SpawnWorker as u64, 6);
        assert_eq!(SyscallNumber::try_from(1), Ok(SyscallNumber::Yield));
        assert_eq!(SyscallNumber::try_from(2), Ok(SyscallNumber::Exit));
        assert_eq!(SyscallNumber::try_from(3), Ok(SyscallNumber::ConsoleWrite));
        assert_eq!(SyscallNumber::try_from(4), Ok(SyscallNumber::ConsoleRead));
        assert_eq!(SyscallNumber::try_from(5), Ok(SyscallNumber::FileRead));
        assert_eq!(SyscallNumber::try_from(6), Ok(SyscallNumber::SpawnWorker));
        assert_eq!(
            SyscallNumber::try_from(0xFFFF),
            Err(UnknownSyscallNumber(0xFFFF))
        );
    }

    #[test]
    fn syscall_request_has_a_fixed_register_snapshot_layout() {
        assert_eq!(core::mem::size_of::<SyscallRequest>(), 7 * 8);
        assert_eq!(core::mem::align_of::<SyscallRequest>(), 8);
        let request = SyscallRequest::from_registers(3, 1, 2, 3, 4, 5, 6);
        assert_eq!(request.number, SyscallNumber::ConsoleWrite as u64);
        assert_eq!(request.arguments, [1, 2, 3, 4, 5, 6]);
        assert_eq!(
            SYSCALL_ARGUMENT_REGISTERS,
            ["RDI", "RSI", "RDX", "R10", "R8", "R9"]
        );
    }

    #[test]
    fn errors_use_sign_extended_negative_return_values() {
        assert_eq!(SyscallError::AccessDenied.as_return_value(), u64::MAX);
        assert_eq!(
            SyscallError::InvalidArgument.as_return_value(),
            u64::MAX - 1
        );
        assert_eq!(SyscallError::NotImplemented.as_return_value(), u64::MAX - 2);
        assert_eq!(SyscallError::Io.as_return_value(), u64::MAX - 3);
    }
}
