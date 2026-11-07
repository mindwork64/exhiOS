#![no_std]

//! Conservative physical frame allocator for the first ExhiOS kernel.
//!
//! Only UEFI EfiConventionalMemory descriptors are made allocatable. Firmware,
//! loader, runtime, ACPI, MMIO and other ranges remain unavailable. The first
//! MiB is kept reserved and this initial bitmap manages physical addresses
//! below 4 GiB.

pub const PAGE_SIZE: u64 = 4096;
pub const UEFI_DESCRIPTOR_VERSION_1: u32 = 1;
pub const UEFI_CONVENTIONAL_MEMORY_TYPE: u32 = 7;
pub const UEFI_DESCRIPTOR_V1_MIN_SIZE: usize = 40;
pub const MIN_ALLOCATABLE_FRAME: u64 = 256; // 1 MiB
pub const MAX_PHYSICAL_ADDRESS: u64 = 0x1_0000_0000; // 4 GiB

const TYPE_OFFSET: usize = 0;
const PHYSICAL_START_OFFSET: usize = 8;
const PAGE_COUNT_OFFSET: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    AlreadyInitialized,
    NotInitialized,
    InvalidBitmapCapacity,
    UnsupportedDescriptorVersion,
    InvalidDescriptorSize,
    InvalidMemoryMapSize,
    InvalidDescriptor,
    AddressOverflow,
    OverlappingUsableRanges,
    NoUsableMemory,
    OutOfMemory,
    UnalignedAddress,
    AddressOutOfRange,
    PageNotManaged,
    PageAlreadyFree,
}

impl MemoryError {
    pub const fn message(self) -> &'static [u8] {
        match self {
            Self::AlreadyInitialized => b"allocator is already initialized",
            Self::NotInitialized => b"allocator is not initialized",
            Self::InvalidBitmapCapacity => b"bitmap capacity is too small",
            Self::UnsupportedDescriptorVersion => b"unsupported UEFI descriptor version",
            Self::InvalidDescriptorSize => b"invalid UEFI descriptor size",
            Self::InvalidMemoryMapSize => b"invalid UEFI memory map size",
            Self::InvalidDescriptor => b"malformed UEFI memory descriptor",
            Self::AddressOverflow => b"physical memory range overflow",
            Self::OverlappingUsableRanges => b"overlapping conventional memory ranges",
            Self::NoUsableMemory => b"no usable conventional memory below 4 GiB",
            Self::OutOfMemory => b"no free physical pages",
            Self::UnalignedAddress => b"physical page address is not aligned",
            Self::AddressOutOfRange => b"physical page address is outside allocator range",
            Self::PageNotManaged => b"physical page is not managed",
            Self::PageAlreadyFree => b"physical page is already free",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryStats {
    pub descriptor_count: u64,
    pub managed_pages: u64,
    pub free_pages: u64,
    pub ignored_low_pages: u64,
    pub ignored_above_limit_pages: u64,
    pub highest_managed_address: u64,
}

pub struct FrameAllocator<const FRAME_COUNT: usize, const WORD_COUNT: usize> {
    managed: [u64; WORD_COUNT],
    allocated: [u64; WORD_COUNT],
    stats: MemoryStats,
    initialized: bool,
}

impl<const FRAME_COUNT: usize, const WORD_COUNT: usize> FrameAllocator<FRAME_COUNT, WORD_COUNT> {
    pub const fn new() -> Self {
        Self {
            managed: [0; WORD_COUNT],
            allocated: [0; WORD_COUNT],
            stats: MemoryStats {
                descriptor_count: 0,
                managed_pages: 0,
                free_pages: 0,
                ignored_low_pages: 0,
                ignored_above_limit_pages: 0,
                highest_managed_address: 0,
            },
            initialized: false,
        }
    }

    /// Initializes from a UEFI memory map using the firmware-reported stride.
    /// UEFI descriptor version 1 has a 40-byte structure, while firmware
    /// commonly reports a 48-byte stride.
    pub fn initialize_from_uefi(
        &mut self,
        map: &[u8],
        descriptor_size: usize,
        descriptor_version: u32,
    ) -> Result<MemoryStats, MemoryError> {
        if self.initialized {
            return Err(MemoryError::AlreadyInitialized);
        }

        let required_words = FRAME_COUNT
            .checked_add(63)
            .ok_or(MemoryError::InvalidBitmapCapacity)?
            / 64;

        if FRAME_COUNT == 0 || WORD_COUNT < required_words {
            return Err(MemoryError::InvalidBitmapCapacity);
        }

        if descriptor_version != UEFI_DESCRIPTOR_VERSION_1 {
            return Err(MemoryError::UnsupportedDescriptorVersion);
        }

        if descriptor_size < UEFI_DESCRIPTOR_V1_MIN_SIZE || descriptor_size % 8 != 0 {
            return Err(MemoryError::InvalidDescriptorSize);
        }

        if map.is_empty() || map.len() % descriptor_size != 0 {
            return Err(MemoryError::InvalidMemoryMapSize);
        }

        // Validate all entries before making any physical page allocatable.
        for descriptor in map.chunks_exact(descriptor_size) {
            let physical_start = read_u64(descriptor, PHYSICAL_START_OFFSET)?;
            let page_count = read_u64(descriptor, PAGE_COUNT_OFFSET)?;

            if physical_start % PAGE_SIZE != 0 || page_count == 0 {
                return Err(MemoryError::InvalidDescriptor);
            }

            let byte_count = page_count
                .checked_mul(PAGE_SIZE)
                .ok_or(MemoryError::AddressOverflow)?;
            physical_start
                .checked_add(byte_count)
                .ok_or(MemoryError::AddressOverflow)?;
        }

        self.clear();

        let mut managed_pages = 0u64;
        let mut ignored_low_pages = 0u64;
        let mut ignored_above_limit_pages = 0u64;
        let mut highest_managed_frame = 0u64;

        for descriptor in map.chunks_exact(descriptor_size) {
            let memory_type = read_u32(descriptor, TYPE_OFFSET)?;
            if memory_type != UEFI_CONVENTIONAL_MEMORY_TYPE {
                continue;
            }

            let physical_start = read_u64(descriptor, PHYSICAL_START_OFFSET)?;
            let page_count = read_u64(descriptor, PAGE_COUNT_OFFSET)?;
            let first_frame = physical_start / PAGE_SIZE;
            let end_frame = first_frame
                .checked_add(page_count)
                .ok_or(MemoryError::AddressOverflow)?;

            let low_end = end_frame.min(MIN_ALLOCATABLE_FRAME);
            if first_frame < low_end {
                ignored_low_pages = ignored_low_pages
                    .checked_add(low_end - first_frame)
                    .ok_or(MemoryError::AddressOverflow)?;
            }

            let max_frames = FRAME_COUNT as u64;
            if end_frame > max_frames {
                let ignored_start = first_frame.max(max_frames);
                if end_frame > ignored_start {
                    ignored_above_limit_pages = ignored_above_limit_pages
                        .checked_add(end_frame - ignored_start)
                        .ok_or(MemoryError::AddressOverflow)?;
                }
            }

            let usable_start = first_frame.max(MIN_ALLOCATABLE_FRAME);
            let usable_end = end_frame.min(max_frames);

            if usable_start >= usable_end {
                continue;
            }

            for frame in usable_start..usable_end {
                let index =
                    usize::try_from(frame).map_err(|_| MemoryError::InvalidBitmapCapacity)?;
                let word = index / 64;
                let mask = 1u64 << (index % 64);

                if self.managed[word] & mask != 0 {
                    self.clear();
                    return Err(MemoryError::OverlappingUsableRanges);
                }

                self.managed[word] |= mask;
                managed_pages = managed_pages
                    .checked_add(1)
                    .ok_or(MemoryError::AddressOverflow)?;
                highest_managed_frame = highest_managed_frame.max(frame + 1);
            }
        }

        if managed_pages == 0 {
            self.clear();
            return Err(MemoryError::NoUsableMemory);
        }

        self.stats = MemoryStats {
            descriptor_count: (map.len() / descriptor_size) as u64,
            managed_pages,
            free_pages: managed_pages,
            ignored_low_pages,
            ignored_above_limit_pages,
            highest_managed_address: highest_managed_frame
                .checked_mul(PAGE_SIZE)
                .ok_or(MemoryError::AddressOverflow)?,
        };
        self.initialized = true;

        Ok(self.stats)
    }

    /// Allocates one 4 KiB physical page and returns its physical address.
    pub fn allocate_page(&mut self) -> Result<u64, MemoryError> {
        if !self.initialized {
            return Err(MemoryError::NotInitialized);
        }

        for word_index in 0..WORD_COUNT {
            let available = self.managed[word_index] & !self.allocated[word_index];
            if available == 0 {
                continue;
            }

            let bit = available.trailing_zeros() as usize;
            let frame = word_index
                .checked_mul(64)
                .and_then(|base| base.checked_add(bit))
                .ok_or(MemoryError::AddressOverflow)?;

            if frame >= FRAME_COUNT {
                continue;
            }

            self.allocated[word_index] |= 1u64 << bit;
            self.stats.free_pages -= 1;

            return (frame as u64)
                .checked_mul(PAGE_SIZE)
                .ok_or(MemoryError::AddressOverflow);
        }

        Err(MemoryError::OutOfMemory)
    }

    /// Permanently reserves a page for a kernel/firmware object.
    ///
    /// A page outside Conventional Memory is already unavailable to this
    /// allocator, so reserving it is an idempotent no-op. A managed free page
    /// is marked allocated and removed from the free-page count.
    pub fn reserve_page(&mut self, physical_address: u64) -> Result<(), MemoryError> {
        if !self.initialized {
            return Err(MemoryError::NotInitialized);
        }
        if physical_address % PAGE_SIZE != 0 {
            return Err(MemoryError::UnalignedAddress);
        }

        let frame_u64 = physical_address / PAGE_SIZE;
        let frame = usize::try_from(frame_u64).map_err(|_| MemoryError::AddressOutOfRange)?;
        if frame >= FRAME_COUNT {
            return Err(MemoryError::AddressOutOfRange);
        }

        let word = frame / 64;
        let mask = 1u64 << (frame % 64);
        if self.managed[word] & mask == 0 || self.allocated[word] & mask != 0 {
            return Ok(());
        }

        self.allocated[word] |= mask;
        self.stats.free_pages -= 1;
        Ok(())
    }

    /// Returns a page previously allocated by this allocator.
    pub fn free_page(&mut self, physical_address: u64) -> Result<(), MemoryError> {
        if !self.initialized {
            return Err(MemoryError::NotInitialized);
        }

        if physical_address % PAGE_SIZE != 0 {
            return Err(MemoryError::UnalignedAddress);
        }

        let frame_u64 = physical_address / PAGE_SIZE;
        let frame = usize::try_from(frame_u64).map_err(|_| MemoryError::AddressOutOfRange)?;
        if frame >= FRAME_COUNT {
            return Err(MemoryError::AddressOutOfRange);
        }

        let word = frame / 64;
        let mask = 1u64 << (frame % 64);

        if self.managed[word] & mask == 0 {
            return Err(MemoryError::PageNotManaged);
        }

        if self.allocated[word] & mask == 0 {
            return Err(MemoryError::PageAlreadyFree);
        }

        self.allocated[word] &= !mask;
        self.stats.free_pages += 1;
        Ok(())
    }

    pub const fn stats(&self) -> MemoryStats {
        self.stats
    }

    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn clear(&mut self) {
        self.managed.fill(0);
        self.allocated.fill(0);
        self.stats = MemoryStats {
            descriptor_count: 0,
            managed_pages: 0,
            free_pages: 0,
            ignored_low_pages: 0,
            ignored_above_limit_pages: 0,
            highest_managed_address: 0,
        };
        self.initialized = false;
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, MemoryError> {
    let end = offset
        .checked_add(4)
        .ok_or(MemoryError::InvalidDescriptorSize)?;
    let value = bytes
        .get(offset..end)
        .ok_or(MemoryError::InvalidDescriptorSize)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, MemoryError> {
    let end = offset
        .checked_add(8)
        .ok_or(MemoryError::InvalidDescriptorSize)?;
    let value = bytes
        .get(offset..end)
        .ok_or(MemoryError::InvalidDescriptorSize)?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    const TEST_DESCRIPTOR_SIZE: usize = 48;
    type TestAllocator = FrameAllocator<4096, 64>;

    fn descriptor(memory_type: u32, physical_start: u64, page_count: u64) -> Vec<u8> {
        let mut bytes = vec![0u8; TEST_DESCRIPTOR_SIZE];
        bytes[TYPE_OFFSET..TYPE_OFFSET + 4].copy_from_slice(&memory_type.to_le_bytes());
        bytes[PHYSICAL_START_OFFSET..PHYSICAL_START_OFFSET + 8]
            .copy_from_slice(&physical_start.to_le_bytes());
        bytes[PAGE_COUNT_OFFSET..PAGE_COUNT_OFFSET + 8].copy_from_slice(&page_count.to_le_bytes());
        bytes
    }

    fn map(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut result = Vec::new();
        for entry in entries {
            result.extend_from_slice(entry);
        }
        result
    }

    #[test]
    fn parses_forty_eight_byte_stride_and_only_conventional_memory() {
        let bytes = map(&[
            descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x1000, 10),
            descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_0000, 8),
            descriptor(4, 0x20_0000, 16),
            descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x30_0000, 4),
        ]);

        let mut allocator = TestAllocator::new();
        let stats = allocator
            .initialize_from_uefi(&bytes, TEST_DESCRIPTOR_SIZE, 1)
            .unwrap();

        assert_eq!(stats.descriptor_count, 4);
        assert_eq!(stats.managed_pages, 12);
        assert_eq!(stats.free_pages, 12);
        assert_eq!(stats.ignored_low_pages, 10);
        assert_eq!(stats.ignored_above_limit_pages, 0);
        assert_eq!(stats.highest_managed_address, 0x30_4000);
    }

    #[test]
    fn allocates_frees_and_reuses_a_physical_page() {
        let bytes = map(&[descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_0000, 2)]);
        let mut allocator = TestAllocator::new();
        allocator
            .initialize_from_uefi(&bytes, TEST_DESCRIPTOR_SIZE, 1)
            .unwrap();

        let first = allocator.allocate_page().unwrap();
        let second = allocator.allocate_page().unwrap();
        assert_eq!(first, 0x10_0000);
        assert_eq!(second, 0x10_1000);
        assert_eq!(allocator.stats().free_pages, 0);
        assert_eq!(allocator.allocate_page(), Err(MemoryError::OutOfMemory));

        allocator.free_page(first).unwrap();
        assert_eq!(allocator.stats().free_pages, 1);
        assert_eq!(
            allocator.free_page(first),
            Err(MemoryError::PageAlreadyFree)
        );

        let reused = allocator.allocate_page().unwrap();
        assert_eq!(reused, first);
        allocator.free_page(reused).unwrap();
        allocator.free_page(second).unwrap();
        assert_eq!(allocator.stats().free_pages, 2);
    }

    #[test]
    fn reserves_managed_pages_and_ignores_firmware_reserved_pages() {
        let bytes = map(&[descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_0000, 2)]);
        let mut allocator = TestAllocator::new();
        allocator
            .initialize_from_uefi(&bytes, TEST_DESCRIPTOR_SIZE, 1)
            .unwrap();

        assert_eq!(allocator.stats().free_pages, 2);
        allocator.reserve_page(0x10_0000).unwrap();
        assert_eq!(allocator.stats().free_pages, 1);
        // Reserving a page twice is harmless and does not decrement twice.
        allocator.reserve_page(0x10_0000).unwrap();
        assert_eq!(allocator.stats().free_pages, 1);
        // A firmware-reserved page is outside the managed bitmap already.
        allocator.reserve_page(0x20_0000).unwrap();
        assert_eq!(allocator.stats().free_pages, 1);
        assert_eq!(allocator.allocate_page().unwrap(), 0x10_1000);
        assert_eq!(allocator.allocate_page(), Err(MemoryError::OutOfMemory));
    }

    #[test]
    fn reserve_page_rejects_unaligned_and_out_of_range_addresses() {
        let bytes = map(&[descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_0000, 1)]);
        let mut allocator = TestAllocator::new();
        allocator
            .initialize_from_uefi(&bytes, TEST_DESCRIPTOR_SIZE, 1)
            .unwrap();

        assert_eq!(
            allocator.reserve_page(0x10_0001),
            Err(MemoryError::UnalignedAddress)
        );
        assert_eq!(
            allocator.reserve_page(MAX_PHYSICAL_ADDRESS),
            Err(MemoryError::AddressOutOfRange)
        );
    }

    #[test]
    fn refuses_to_free_unmanaged_or_unaligned_addresses() {
        let bytes = map(&[descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_0000, 1)]);
        let mut allocator = TestAllocator::new();
        allocator
            .initialize_from_uefi(&bytes, TEST_DESCRIPTOR_SIZE, 1)
            .unwrap();

        assert_eq!(
            allocator.free_page(0x10_0001),
            Err(MemoryError::UnalignedAddress)
        );
        assert_eq!(
            allocator.free_page(0x10_0000_0000),
            Err(MemoryError::AddressOutOfRange)
        );
        assert_eq!(
            allocator.free_page(0x2000),
            Err(MemoryError::PageNotManaged)
        );
    }

    #[test]
    fn rejects_invalid_map_metadata() {
        let bytes = map(&[descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_0000, 1)]);
        let mut allocator = TestAllocator::new();
        assert_eq!(
            allocator.initialize_from_uefi(&bytes, 48, 2),
            Err(MemoryError::UnsupportedDescriptorVersion)
        );
        assert_eq!(
            allocator.initialize_from_uefi(&bytes, 39, 1),
            Err(MemoryError::InvalidDescriptorSize)
        );
        assert_eq!(
            allocator.initialize_from_uefi(&bytes[..47], 48, 1),
            Err(MemoryError::InvalidMemoryMapSize)
        );
    }

    #[test]
    fn rejects_zero_page_descriptors_and_overflowing_ranges() {
        let zero = map(&[descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_0000, 0)]);
        let mut allocator = TestAllocator::new();
        assert_eq!(
            allocator.initialize_from_uefi(&zero, TEST_DESCRIPTOR_SIZE, 1),
            Err(MemoryError::InvalidDescriptor)
        );

        let overflow = map(&[descriptor(
            UEFI_CONVENTIONAL_MEMORY_TYPE,
            u64::MAX & !(PAGE_SIZE - 1),
            2,
        )]);
        let mut allocator = TestAllocator::new();
        assert_eq!(
            allocator.initialize_from_uefi(&overflow, TEST_DESCRIPTOR_SIZE, 1),
            Err(MemoryError::AddressOverflow)
        );
    }

    #[test]
    fn rejects_overlapping_conventional_descriptors() {
        let bytes = map(&[
            descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_0000, 4),
            descriptor(UEFI_CONVENTIONAL_MEMORY_TYPE, 0x10_2000, 4),
        ]);
        let mut allocator = TestAllocator::new();
        assert_eq!(
            allocator.initialize_from_uefi(&bytes, TEST_DESCRIPTOR_SIZE, 1),
            Err(MemoryError::OverlappingUsableRanges)
        );
        assert!(!allocator.is_initialized());
    }

    #[test]
    fn clips_memory_above_the_allocator_limit() {
        let allocator_limit = 4096 * PAGE_SIZE;
        let bytes = map(&[descriptor(
            UEFI_CONVENTIONAL_MEMORY_TYPE,
            allocator_limit - PAGE_SIZE,
            3,
        )]);
        let mut allocator = TestAllocator::new();
        let stats = allocator
            .initialize_from_uefi(&bytes, TEST_DESCRIPTOR_SIZE, 1)
            .unwrap();

        assert_eq!(stats.managed_pages, 1);
        assert_eq!(stats.ignored_above_limit_pages, 2);
        assert_eq!(
            allocator.allocate_page().unwrap(),
            allocator_limit - PAGE_SIZE
        );
    }
}
