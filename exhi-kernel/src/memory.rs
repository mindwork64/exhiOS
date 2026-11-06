use core::slice;

use exhi_abi::BootInfo;
use exhi_memory::{
    FrameAllocator, MemoryError, MemoryStats, MAX_PHYSICAL_ADDRESS, PAGE_SIZE,
};

const MAX_FRAME_COUNT: usize = (MAX_PHYSICAL_ADDRESS / PAGE_SIZE) as usize;
const BITMAP_WORD_COUNT: usize = MAX_FRAME_COUNT / 64;

static mut PHYSICAL_ALLOCATOR: FrameAllocator<MAX_FRAME_COUNT, BITMAP_WORD_COUNT> =
    FrameAllocator::new();

/// Initializes from the retained UEFI memory map passed in BootInfo.
/// The initial kernel still uses UEFI's identity mapping at this stage.
pub fn initialize(boot_info: &BootInfo) -> Result<MemoryStats, MemoryError> {
    let map_address = boot_info.memory_map.physical_address;
    let map_size = usize::try_from(boot_info.memory_map.map_size)
        .map_err(|_| MemoryError::InvalidMemoryMapSize)?;
    let descriptor_size = usize::try_from(boot_info.memory_map.descriptor_size)
        .map_err(|_| MemoryError::InvalidDescriptorSize)?;

    if map_address == 0 || map_size == 0 || map_size > isize::MAX as usize {
        return Err(MemoryError::InvalidMemoryMapSize);
    }

    map_address
        .checked_add(map_size as u64)
        .ok_or(MemoryError::AddressOverflow)?;

    // SAFETY: BootInfo refers to the final UEFI memory map buffer retained by
    // the bootloader. Current firmware page tables identity-map loader data.
    let map = unsafe { slice::from_raw_parts(map_address as *const u8, map_size) };

    let allocator = core::ptr::addr_of_mut!(PHYSICAL_ALLOCATOR);
    // SAFETY: Kernel startup is single-threaded and IF remains clear.
    unsafe {
        (&mut *allocator).initialize_from_uefi(
            map,
            descriptor_size,
            boot_info.memory_map.descriptor_version,
        )
    }
}

/// Tests metadata allocation, free and reuse without dereferencing a physical
/// page. Mapping arbitrary physical pages belongs to the page-table milestone.
pub fn allocation_reuse_smoke_test() -> Result<u64, MemoryError> {
    let allocator = core::ptr::addr_of_mut!(PHYSICAL_ALLOCATOR);

    // SAFETY: Called once during single-threaded kernel startup while IF is
    // clear. The allocator bitmap itself resides in the kernel image.
    unsafe {
        let allocator = &mut *allocator;
        let first = allocator.allocate_page()?;
        allocator.free_page(first)?;
        let reused = allocator.allocate_page()?;

        if reused != first {
            let _ = allocator.free_page(reused);
            return Err(MemoryError::PageAlreadyFree);
        }

        allocator.free_page(reused)?;
        Ok(reused)
    }
}
