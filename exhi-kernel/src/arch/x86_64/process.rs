//! Fixed-address ELF64 user-process loader.
//!
//! The loader owns every data frame it maps. A partially loaded process is
//! rolled back in reverse order, and the address-space page tables are destroyed
//! only after all user mappings have been removed and their frames returned.
//! The first implementation is single-CPU and requires IF=0 for all operations.

use core::ptr;

use exhi_elf::{ElfError, ElfFile, ProgramHeader, PT_LOAD};
use exhi_memory::MemoryError;

use crate::memory;

use super::address_space::{self, AddressSpace, AddressSpaceError, PageMapping, UserPageFlags};

const PAGE_SIZE: u64 = 4096;
const PAGE_MASK: u64 = PAGE_SIZE - 1;
const USER_BASE: u64 = 0x0000_0002_0000_0000;
const USER_LIMIT: u64 = 0x0000_0080_0000_0000;
const USER_STACK_TOP: u64 = 0x0000_0002_1000_0000;
const USER_STACK_PAGES: usize = 4;
const USER_STACK_SIZE: u64 = USER_STACK_PAGES as u64 * PAGE_SIZE;
const USER_STACK_BASE: u64 = USER_STACK_TOP - USER_STACK_SIZE;
const USER_STACK_GUARD: u64 = USER_STACK_BASE - PAGE_SIZE;
const MAX_IMAGE_PAGES: usize = 256;
const MAX_OWNED_PAGES: usize = MAX_IMAGE_PAGES + USER_STACK_PAGES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessLoadError {
    Elf(ElfError),
    AddressSpace(AddressSpaceError),
    Memory(MemoryError),
    InvalidSegmentRange,
    SegmentOutsideUserRange,
    SegmentOverlapsUserStack,
    TooManyImagePages,
    TooManyOwnedPages,
    InvalidPhysicalFrame,
    FileRangeOutOfBounds,
    MappingMismatch,
    CleanupFailed,
}

impl From<ElfError> for ProcessLoadError {
    fn from(error: ElfError) -> Self {
        Self::Elf(error)
    }
}

impl From<AddressSpaceError> for ProcessLoadError {
    fn from(error: AddressSpaceError) -> Self {
        Self::AddressSpace(error)
    }
}

impl From<MemoryError> for ProcessLoadError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnedPage {
    virtual_page: u64,
    physical_frame: u64,
    mapped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessLoadReport {
    pub entry_point: u64,
    pub user_stack_pointer: u64,
    pub image_pages: usize,
    pub stack_pages: usize,
}

/// An ELF process and all resources it owns.
///
/// Call destroy before dropping this object. Drop intentionally does not
/// manipulate CR3 or the physical allocator because those operations require
/// a serialized kernel context with interrupts disabled.
pub struct LoadedProcess {
    address_space: Option<AddressSpace>,
    pages: [Option<OwnedPage>; MAX_OWNED_PAGES],
    page_count: usize,
    image_page_count: usize,
    entry_point: u64,
    user_stack_pointer: u64,
}

impl LoadedProcess {
    /// Parses and loads an ET_EXEC image into a private address space.
    ///
    /// PT_LOAD segments are mapped with their ELF permissions, each frame is
    /// zeroed before file bytes are copied, and the stack receives a separate
    /// unmapped guard page below four RW+NX pages.
    pub fn load(image: &[u8]) -> Result<Self, ProcessLoadError> {
        let elf = ElfFile::parse(image)?;
        let image_page_count = preflight_image(&elf)?;
        let address_space = AddressSpace::create()?;
        let mut process = Self {
            address_space: Some(address_space),
            pages: [None; MAX_OWNED_PAGES],
            page_count: 0,
            image_page_count: 0,
            entry_point: elf.entry_point(),
            user_stack_pointer: USER_STACK_TOP - 8,
        };

        let result = process.load_segments(image, &elf).and_then(|()| {
            process.image_page_count = process.page_count;
            process.load_user_stack()?;
            if process.image_page_count != image_page_count {
                return Err(ProcessLoadError::MappingMismatch);
            }
            Ok(())
        });

        if let Err(error) = result {
            process
                .destroy()
                .map_err(|_| ProcessLoadError::CleanupFailed)?;
            return Err(error);
        }

        Ok(process)
    }

    pub const fn entry_point(&self) -> u64 {
        self.entry_point
    }

    pub const fn user_stack_pointer(&self) -> u64 {
        self.user_stack_pointer
    }

    pub const fn image_page_count(&self) -> usize {
        self.image_page_count
    }

    pub const fn total_page_count(&self) -> usize {
        self.page_count
    }

    pub fn root_address(&self) -> Result<u64, ProcessLoadError> {
        self.address_space
            .as_ref()
            .map(AddressSpace::root_address)
            .ok_or(ProcessLoadError::AddressSpace(AddressSpaceError::Destroyed))
    }

    pub fn translate(&self, address: u64) -> Result<PageMapping, ProcessLoadError> {
        self.address_space
            .as_ref()
            .ok_or(ProcessLoadError::AddressSpace(AddressSpaceError::Destroyed))?
            .translate(address)
            .map_err(ProcessLoadError::AddressSpace)
    }

    pub fn activate(&self) -> Result<(), ProcessLoadError> {
        self.address_space
            .as_ref()
            .ok_or(ProcessLoadError::AddressSpace(AddressSpaceError::Destroyed))?
            .activate()
            .map_err(ProcessLoadError::AddressSpace)
    }

    /// Returns all owned user frames and private page-table structures.
    /// The kernel address space is restored first if this process is active.
    pub fn destroy(&mut self) -> Result<(), ProcessLoadError> {
        let Some(space) = self.address_space.as_mut() else {
            return Ok(());
        };

        if space.is_active()? {
            address_space::restore_kernel()?;
        }

        for index in (0..self.page_count).rev() {
            let Some(mut page) = self.pages[index] else {
                continue;
            };

            if page.mapped {
                let unmapped = space.unmap_user_page(page.virtual_page)?;
                if unmapped != page.physical_frame {
                    return Err(ProcessLoadError::MappingMismatch);
                }
                page.mapped = false;
                self.pages[index] = Some(page);
            }

            memory::release_frame(page.physical_frame)?;
            self.pages[index] = None;
        }

        space.destroy()?;
        self.address_space = None;
        self.page_count = 0;
        self.image_page_count = 0;
        Ok(())
    }

    fn load_segments(&mut self, image: &[u8], elf: &ElfFile<'_>) -> Result<(), ProcessLoadError> {
        for header in elf.program_headers() {
            if header.kind != PT_LOAD {
                continue;
            }
            self.load_segment(image, header)?;
        }
        Ok(())
    }

    fn load_segment(
        &mut self,
        image: &[u8],
        header: ProgramHeader,
    ) -> Result<(), ProcessLoadError> {
        let memory_end = header
            .virtual_address
            .checked_add(header.memory_size)
            .ok_or(ProcessLoadError::InvalidSegmentRange)?;
        let file_end = header
            .virtual_address
            .checked_add(header.file_size)
            .ok_or(ProcessLoadError::InvalidSegmentRange)?;
        let page_start = align_down(header.virtual_address);
        let page_end = align_up(memory_end).ok_or(ProcessLoadError::InvalidSegmentRange)?;
        let source_start =
            usize::try_from(header.offset).map_err(|_| ProcessLoadError::FileRangeOutOfBounds)?;
        let source_size = usize::try_from(header.file_size)
            .map_err(|_| ProcessLoadError::FileRangeOutOfBounds)?;
        let source_end = source_start
            .checked_add(source_size)
            .ok_or(ProcessLoadError::FileRangeOutOfBounds)?;
        if source_end > image.len() {
            return Err(ProcessLoadError::FileRangeOutOfBounds);
        }

        let flags = UserPageFlags {
            writable: header.is_writable(),
            executable: header.is_executable(),
        };
        let mut page = page_start;
        while page < page_end {
            let frame = self.allocate_and_map_page(page, flags)?;

            let copy_start = page.max(header.virtual_address);
            let copy_end = (page + PAGE_SIZE).min(file_end);
            if copy_start < copy_end {
                let source_offset = source_start
                    .checked_add(
                        usize::try_from(copy_start - header.virtual_address)
                            .map_err(|_| ProcessLoadError::FileRangeOutOfBounds)?,
                    )
                    .ok_or(ProcessLoadError::FileRangeOutOfBounds)?;
                let copy_length = usize::try_from(copy_end - copy_start)
                    .map_err(|_| ProcessLoadError::FileRangeOutOfBounds)?;
                let source_end = source_offset
                    .checked_add(copy_length)
                    .ok_or(ProcessLoadError::FileRangeOutOfBounds)?;
                if source_end > image.len() {
                    return Err(ProcessLoadError::FileRangeOutOfBounds);
                }
                let destination_offset = usize::try_from(copy_start - page)
                    .map_err(|_| ProcessLoadError::InvalidSegmentRange)?;
                // SAFETY: The frame is allocator-owned, identity-mapped in the
                // current kernel root, and zeroed before any image data is copied.
                unsafe {
                    ptr::copy_nonoverlapping(
                        image.as_ptr().add(source_offset),
                        (frame + destination_offset as u64) as *mut u8,
                        copy_length,
                    );
                }
            }

            page = page
                .checked_add(PAGE_SIZE)
                .ok_or(ProcessLoadError::InvalidSegmentRange)?;
        }
        Ok(())
    }

    fn load_user_stack(&mut self) -> Result<(), ProcessLoadError> {
        for page in 0..USER_STACK_PAGES {
            let address = USER_STACK_BASE + page as u64 * PAGE_SIZE;
            self.allocate_and_map_page(address, UserPageFlags::read_write_nx())?;
        }
        Ok(())
    }

    fn allocate_and_map_page(
        &mut self,
        virtual_page: u64,
        flags: UserPageFlags,
    ) -> Result<u64, ProcessLoadError> {
        if self.page_count >= MAX_OWNED_PAGES {
            return Err(ProcessLoadError::TooManyOwnedPages);
        }

        let frame = memory::allocate_frame()?;
        if frame == 0 || frame >= 0x1_0000_0000 || frame & PAGE_MASK != 0 {
            let _ = memory::release_frame(frame);
            return Err(ProcessLoadError::InvalidPhysicalFrame);
        }

        // Register ownership before mapping so every later error path can
        // release this frame, whether the PTE was installed or not.
        let index = self.page_count;
        self.pages[index] = Some(OwnedPage {
            virtual_page,
            physical_frame: frame,
            mapped: false,
        });
        self.page_count += 1;

        // Physical frames below 4 GiB remain identity mapped in the shared
        // kernel address space. No user mapping is active during loading.
        unsafe {
            ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
        }

        let space = self
            .address_space
            .as_mut()
            .ok_or(ProcessLoadError::AddressSpace(AddressSpaceError::Destroyed))?;
        // SAFETY: This process owns the allocated frame and retains it until
        // destroy removes the matching user mapping and releases the frame.
        unsafe {
            space.map_user_page(virtual_page, frame, flags)?;
        }
        self.pages[index]
            .as_mut()
            .ok_or(ProcessLoadError::MappingMismatch)?
            .mapped = true;
        Ok(frame)
    }
}

fn preflight_image(elf: &ElfFile<'_>) -> Result<usize, ProcessLoadError> {
    let mut page_count = 0usize;
    for header in elf.program_headers() {
        if header.kind != PT_LOAD {
            continue;
        }

        let end = header
            .virtual_address
            .checked_add(header.memory_size)
            .ok_or(ProcessLoadError::InvalidSegmentRange)?;
        let page_start = align_down(header.virtual_address);
        let page_end = align_up(end).ok_or(ProcessLoadError::InvalidSegmentRange)?;
        if page_start < USER_BASE || end > USER_LIMIT || page_end <= page_start {
            return Err(ProcessLoadError::SegmentOutsideUserRange);
        }
        if page_start < USER_STACK_TOP && USER_STACK_GUARD < page_end {
            return Err(ProcessLoadError::SegmentOverlapsUserStack);
        }

        let segment_pages = usize::try_from((page_end - page_start) / PAGE_SIZE)
            .map_err(|_| ProcessLoadError::TooManyImagePages)?;
        page_count = page_count
            .checked_add(segment_pages)
            .ok_or(ProcessLoadError::TooManyImagePages)?;
        if page_count > MAX_IMAGE_PAGES {
            return Err(ProcessLoadError::TooManyImagePages);
        }
    }

    if elf.entry_point() < USER_BASE || elf.entry_point() >= USER_LIMIT {
        return Err(ProcessLoadError::SegmentOutsideUserRange);
    }
    Ok(page_count)
}

const fn align_down(address: u64) -> u64 {
    address & !PAGE_MASK
}

fn align_up(address: u64) -> Option<u64> {
    address.checked_add(PAGE_MASK).map(align_down)
}

/// Exercises the loader against the embedded first user executable and then
/// destroys it again. This is a loader test, not a process scheduling test.
pub fn smoke_test(image: &[u8]) -> Result<ProcessLoadReport, ProcessLoadError> {
    let mut process = LoadedProcess::load(image)?;
    let report = ProcessLoadReport {
        entry_point: process.entry_point(),
        user_stack_pointer: process.user_stack_pointer(),
        image_pages: process.image_page_count(),
        stack_pages: USER_STACK_PAGES,
    };

    let elf = ElfFile::parse(image)?;
    let validation = validate_loaded_process(&process, image, &elf);
    let cleanup = process.destroy();
    validation?;
    cleanup?;
    if process.translate(report.entry_point).is_ok() {
        return Err(ProcessLoadError::MappingMismatch);
    }
    Ok(report)
}

fn validate_loaded_process(
    process: &LoadedProcess,
    image: &[u8],
    elf: &ElfFile<'_>,
) -> Result<(), ProcessLoadError> {
    let entry = process.translate(process.entry_point())?;
    if !entry.user || !entry.executable || entry.writable {
        return Err(ProcessLoadError::MappingMismatch);
    }

    let entry_header = elf
        .program_headers()
        .find(|header| {
            header.kind == PT_LOAD
                && header.is_executable()
                && process.entry_point() >= header.virtual_address
                && process.entry_point() < header.virtual_address + header.memory_size
        })
        .ok_or(ProcessLoadError::MappingMismatch)?;
    let entry_file_offset = entry_header
        .offset
        .checked_add(process.entry_point() - entry_header.virtual_address)
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or(ProcessLoadError::FileRangeOutOfBounds)?;
    let expected_opcode = *image
        .get(entry_file_offset)
        .ok_or(ProcessLoadError::FileRangeOutOfBounds)?;
    let entry_offset = process.entry_point() & PAGE_MASK;
    let loaded_opcode = unsafe {
        ptr::read_volatile((entry.physical_page + entry_offset) as *const u8)
    };
    if loaded_opcode != expected_opcode {
        return Err(ProcessLoadError::MappingMismatch);
    }

    for header in elf.program_headers() {
        if header.kind != PT_LOAD {
            continue;
        }
        let first = process.translate(header.virtual_address)?;
        let last_address = header.virtual_address + header.memory_size - 1;
        let last = process.translate(last_address)?;
        let expected_writable = header.is_writable();
        let expected_executable = header.is_executable();
        if !first.user
            || !last.user
            || first.writable != expected_writable
            || last.writable != expected_writable
            || first.executable != expected_executable
            || last.executable != expected_executable
        {
            return Err(ProcessLoadError::MappingMismatch);
        }

        if header.file_size < header.memory_size {
            let bss_start = header
                .virtual_address
                .checked_add(header.file_size)
                .ok_or(ProcessLoadError::InvalidSegmentRange)?;
            let bss = process.translate(bss_start)?;
            let bss_offset = bss_start & PAGE_MASK;
            let bss_byte = unsafe {
                ptr::read_volatile((bss.physical_page + bss_offset) as *const u8)
            };
            if bss_byte != 0 {
                return Err(ProcessLoadError::MappingMismatch);
            }
        }
    }

    for stack_index in [0, USER_STACK_PAGES - 1] {
        let stack_address = USER_STACK_BASE + stack_index as u64 * PAGE_SIZE;
        let stack = process.translate(stack_address)?;
        if !stack.user || !stack.writable || stack.executable {
            return Err(ProcessLoadError::MappingMismatch);
        }
    }

    if process.translate(USER_STACK_GUARD).is_ok() {
        return Err(ProcessLoadError::MappingMismatch);
    }

    Ok(())
}
