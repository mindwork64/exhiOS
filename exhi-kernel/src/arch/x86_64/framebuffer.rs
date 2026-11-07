use core::convert::TryFrom;
use core::slice;

use exhi_abi::FramebufferInfo;
use exhi_console::{ConsoleError, FramebufferConsole};

const IDENTITY_MAP_LIMIT: u64 = 1 << 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramebufferInitError {
    Missing,
    InvalidAlignment,
    InvalidDimensions,
    InvalidStride,
    InvalidBufferSize,
    OutsideIdentityMap,
    AddressSpaceTooSmall,
    Console(ConsoleError),
}

impl FramebufferInitError {
    pub const fn message(self) -> &'static [u8] {
        match self {
            Self::Missing => b"GOP framebuffer is unavailable",
            Self::InvalidAlignment => b"framebuffer base is not 32-bit aligned",
            Self::InvalidDimensions => b"framebuffer dimensions are invalid",
            Self::InvalidStride => b"framebuffer stride is smaller than width",
            Self::InvalidBufferSize => b"framebuffer buffer is too small",
            Self::OutsideIdentityMap => b"framebuffer is outside the 4 GiB identity map",
            Self::AddressSpaceTooSmall => b"framebuffer length does not fit usize",
            Self::Console(error) => error.message(),
        }
    }
}

/// # Safety
/// The framebuffer must remain writable through the current identity mapping
/// for the returned lifetime and must have no competing mutable writer.
pub unsafe fn initialize(
    info: &FramebufferInfo,
) -> Result<FramebufferConsole<'static>, FramebufferInitError> {
    if info.physical_address == 0 || info.buffer_size == 0 {
        return Err(FramebufferInitError::Missing);
    }
    if info.physical_address & 3 != 0 {
        return Err(FramebufferInitError::InvalidAlignment);
    }
    if info.width == 0 || info.height == 0 {
        return Err(FramebufferInitError::InvalidDimensions);
    }
    if info.pixels_per_scan_line < info.width {
        return Err(FramebufferInitError::InvalidStride);
    }

    let required_pixels = u64::from(info.pixels_per_scan_line)
        .checked_mul(u64::from(info.height))
        .ok_or(FramebufferInitError::InvalidBufferSize)?;
    let required_bytes = required_pixels
        .checked_mul(core::mem::size_of::<u32>() as u64)
        .ok_or(FramebufferInitError::InvalidBufferSize)?;
    if info.buffer_size < required_bytes {
        return Err(FramebufferInitError::InvalidBufferSize);
    }
    let end = info
        .physical_address
        .checked_add(required_bytes)
        .ok_or(FramebufferInitError::OutsideIdentityMap)?;
    if end > IDENTITY_MAP_LIMIT {
        return Err(FramebufferInitError::OutsideIdentityMap);
    }

    let len =
        usize::try_from(required_pixels).map_err(|_| FramebufferInitError::AddressSpaceTooSmall)?;
    let pointer = info.physical_address as *mut u32;
    // SAFETY: the caller guarantees framebuffer lifetime, identity mapping and exclusive access.
    let pixels = unsafe { slice::from_raw_parts_mut(pointer, len) };

    FramebufferConsole::new(
        pixels,
        info.width,
        info.height,
        info.pixels_per_scan_line,
        info.pixel_format,
    )
    .map_err(FramebufferInitError::Console)
}
