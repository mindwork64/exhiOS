#![no_std]

//! Small user-mode wrapper around the stable ExhiOS syscall ABI.
//!
//! The wrappers deliberately expose only the syscalls implemented by the
//! initial kernel ABI. Pointers are borrowed for the duration of the syscall;
//! the kernel validates the complete user range before accessing it.

use core::arch::asm;
use core::hint::spin_loop;

pub use exhi_abi::syscall::{SyscallError, SyscallNumber};

/// Yields the current user task to the kernel scheduler.
#[inline]
pub fn yield_now() -> Result<(), SyscallError> {
    decode_return(raw_syscall(SyscallNumber::Yield as u64, [0; 6])).map(|_| ())
}

/// Writes all bytes to the process console using the current ConsoleWrite grant.
/// A successful call returns the number of bytes accepted by the kernel.
#[inline]
pub fn console_write(bytes: &[u8]) -> Result<usize, SyscallError> {
    let mut written = 0usize;
    while written < bytes.len() {
        let address = bytes[written..].as_ptr() as u64;
        let length = (bytes.len() - written) as u64;
        let count = decode_return(raw_syscall(
            SyscallNumber::ConsoleWrite as u64,
            [address, length, 0, 0, 0, 0],
        ))?;
        let count = usize::try_from(count).map_err(|_| SyscallError::InvalidArgument)?;
        if count == 0 || count > bytes.len() - written {
            return Err(SyscallError::Io);
        }
        written += count;
    }
    Ok(written)
}

/// Reads currently available bytes from the serial console into a user buffer.
/// Returns zero when no input is ready; callers may yield and retry.
#[inline]
pub fn console_read(bytes: &mut [u8]) -> Result<usize, SyscallError> {
    let count = decode_return(raw_syscall(
        SyscallNumber::ConsoleRead as u64,
        [bytes.as_mut_ptr() as u64, bytes.len() as u64, 0, 0, 0, 0],
    ))?;
    let count = usize::try_from(count).map_err(|_| SyscallError::InvalidArgument)?;
    if count > bytes.len() {
        return Err(SyscallError::Io);
    }
    Ok(count)
}

/// Reads bytes from a process-visible VFS file into a user-owned buffer.
///
/// Returns zero at EOF. The kernel checks the FileRead capability before
/// validating pointers or looking up the path.
#[inline]
pub fn file_read(path: &str, offset: usize, destination: &mut [u8]) -> Result<usize, SyscallError> {
    let count = decode_return(raw_syscall(
        SyscallNumber::FileRead as u64,
        [
            path.as_ptr() as u64,
            path.len() as u64,
            offset as u64,
            destination.as_mut_ptr() as u64,
            destination.len() as u64,
            0,
        ],
    ))?;
    let count = usize::try_from(count).map_err(|_| SyscallError::InvalidArgument)?;
    if count > destination.len() {
        return Err(SyscallError::Io);
    }
    Ok(count)
}

/// Starts the built-in worker process and returns its scheduler task ID.
#[inline]
pub fn spawn_worker() -> Result<u64, SyscallError> {
    decode_return(raw_syscall(SyscallNumber::SpawnWorker as u64, [0; 6]))
}

/// Terminates the current process. A correct kernel implementation does not
/// return to the caller; spin forever if an exit request is unexpectedly
/// returned so user code cannot continue after requesting termination.
#[inline]
pub fn exit() -> ! {
    let _ = raw_syscall(SyscallNumber::Exit as u64, [0; 6]);
    loop {
        spin_loop();
    }
}

#[inline]
fn decode_return(value: u64) -> Result<u64, SyscallError> {
    match value as i64 {
        value if value == SyscallError::AccessDenied as i64 => Err(SyscallError::AccessDenied),
        value if value == SyscallError::InvalidArgument as i64 => {
            Err(SyscallError::InvalidArgument)
        }
        value if value == SyscallError::NotImplemented as i64 => Err(SyscallError::NotImplemented),
        value if value == SyscallError::Io as i64 => Err(SyscallError::Io),
        _ => Ok(value),
    }
}

#[inline(always)]
fn raw_syscall(number: u64, arguments: [u64; 6]) -> u64 {
    let result: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number => result,
            in("rdi") arguments[0],
            in("rsi") arguments[1],
            in("rdx") arguments[2],
            in("r10") arguments[3],
            in("r8") arguments[4],
            in("r9") arguments[5],
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_defined_kernel_errors() {
        assert_eq!(decode_return(u64::MAX), Err(SyscallError::AccessDenied));
        assert_eq!(
            decode_return(u64::MAX - 1),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            decode_return(u64::MAX - 2),
            Err(SyscallError::NotImplemented)
        );
        assert_eq!(decode_return(u64::MAX - 3), Err(SyscallError::Io));
        assert_eq!(decode_return(0), Ok(0));
        assert_eq!(decode_return(128), Ok(128));
    }

    #[test]
    fn wrappers_use_the_stable_abi_numbers() {
        assert_eq!(SyscallNumber::Yield as u64, 1);
        assert_eq!(SyscallNumber::Exit as u64, 2);
        assert_eq!(SyscallNumber::ConsoleWrite as u64, 3);
        assert_eq!(SyscallNumber::ConsoleRead as u64, 4);
        assert_eq!(SyscallNumber::FileRead as u64, 5);
        assert_eq!(SyscallNumber::SpawnWorker as u64, 6);
    }
}
