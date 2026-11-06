#![no_std]

//! Stable, explicitly laid-out data structures shared across ExhiOS
//! privilege and image boundaries.

pub const BOOT_INFO_MAGIC: [u8; 8] = *b"EXHIOS01";
pub const BOOT_INFO_VERSION: u32 = 1;

/// Information transferred by the UEFI loader to the kernel.
///
/// Address fields contain physical addresses. The initial kernel will use
/// the firmware-provided address mapping until it installs its own page tables.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BootInfo {
    pub magic: [u8; 8],
    pub version: u32,
    pub struct_size: u32,
    pub memory_map: MemoryMapInfo,
    pub framebuffer: FramebufferInfo,
    pub acpi_rsdp_address: u64,
}

impl BootInfo {
    pub const fn empty() -> Self {
        Self {
            magic: BOOT_INFO_MAGIC,
            version: BOOT_INFO_VERSION,
            struct_size: core::mem::size_of::<Self>() as u32,
            memory_map: MemoryMapInfo::empty(),
            framebuffer: FramebufferInfo::empty(),
            acpi_rsdp_address: 0,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.magic == BOOT_INFO_MAGIC
            && self.version == BOOT_INFO_VERSION
            && self.struct_size as usize >= core::mem::size_of::<Self>()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryMapInfo {
    pub physical_address: u64,
    pub map_size: u64,
    pub descriptor_size: u32,
    pub descriptor_version: u32,
}

impl MemoryMapInfo {
    pub const fn empty() -> Self {
        Self {
            physical_address: 0,
            map_size: 0,
            descriptor_size: 0,
            descriptor_version: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FramebufferInfo {
    pub physical_address: u64,
    pub buffer_size: u64,
    pub width: u32,
    pub height: u32,
    pub pixels_per_scan_line: u32,
    /// UEFI GOP PixelFormat numeric value.
    pub pixel_format: u32,
}

impl FramebufferInfo {
    pub const fn empty() -> Self {
        Self {
            physical_address: 0,
            buffer_size: 0,
            width: 0,
            height: 0,
            pixels_per_scan_line: 0,
            pixel_format: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_info_layout_is_fixed() {
        assert_eq!(core::mem::size_of::<MemoryMapInfo>(), 24);
        assert_eq!(core::mem::size_of::<FramebufferInfo>(), 32);
        assert_eq!(core::mem::size_of::<BootInfo>(), 80);
    }

    #[test]
    fn empty_boot_info_has_valid_header() {
        let info = BootInfo::empty();
        assert!(info.is_valid());
        assert_eq!(info.magic, *b"EXHIOS01");
        assert_eq!(info.version, 1);
    }
}
