//! Fixed-address ELF64 user-process loader.
//!
//! The loader owns every data frame it maps. A partially loaded process is
//! rolled back in reverse order, and the address-space page tables are destroyed
//! only after all user mappings have been removed and their frames returned.
//! The first implementation is single-CPU and requires IF=0 for all operations.

#[cfg(feature = "process-runtime")]
use core::arch::global_asm;
use core::ptr;

#[cfg(feature = "process-runtime")]
use exhi_abi::{SyscallError, SyscallRequest};
use exhi_elf::{ElfError, ElfFile, ProgramHeader, PT_LOAD};
use exhi_memory::MemoryError;
#[cfg(feature = "process-runtime")]
use exhi_syscall::{dispatch as dispatch_syscall, DispatchOutcome, SyscallServices};
#[cfg(feature = "process-runtime")]
use exhi_task::{
    Capability, CapabilitySet, ScheduleDecision, Scheduler, TaskContext, TaskId, TaskState,
    UserTaskMetadata,
};
#[cfg(feature = "process-runtime")]
use exhi_vfs::{
    create_directory_path, create_file_path, resolve_path, FileSystem, NodeKind, RamFs,
};

use crate::memory;

#[cfg(all(
    feature = "process-load-smoke-test",
    not(feature = "process-run-smoke-test")
))]
use super::address_space::PageMapping;
use super::address_space::{self, AddressSpace, AddressSpaceError, UserPageFlags};
#[cfg(feature = "process-runtime")]
use super::{gdt, serial, stack_guard, syscall, user};
#[cfg(feature = "process-stack-guard-smoke-test")]
use super::{idt, vmm};

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
#[cfg(feature = "process-runtime")]
// One guard page plus sixteen usable pages. Worker creation loads and rolls back a
// bounded LoadedProcess while running on the parent's kernel stack.
const INIT_KERNEL_STACK_PAGES: usize = 17;
#[cfg(feature = "process-runtime")]
const INIT_KERNEL_STACK_BYTES: usize = INIT_KERNEL_STACK_PAGES * PAGE_SIZE as usize;
#[cfg(feature = "process-run-smoke-test")]
const INIT_YIELD_EXPECTED: u64 = 3;
#[cfg(feature = "process-run-smoke-test")]
const INIT_SYSCALL_EXPECTED: u64 = 6;
#[cfg(feature = "process-runtime")]
const MAX_INIT_CONSOLE_WRITE: usize = 64 * 1024;
#[cfg(feature = "process-runtime")]
const MAX_INIT_CONSOLE_READ: usize = 256;
#[cfg(feature = "process-runtime")]
const USERLAND_MAX_PATH: usize = 128;
#[cfg(feature = "process-runtime")]
const USERLAND_MAX_FILE_READ: usize = 256;
#[cfg(feature = "process-runtime")]
const USERLAND_FS_NODE_COUNT: usize = 8;
#[cfg(feature = "process-runtime")]
const USERLAND_FS_FILE_CAPACITY: usize = 256;
#[cfg(feature = "process-runtime")]
const USERLAND_CONFIG_CONTENT: &[u8] =
    b"# ExhiOS system configuration\nhostname=exhios\nprompt=exhios>\nversion=0.1\n";

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

#[cfg(all(
    feature = "process-load-smoke-test",
    not(feature = "process-run-smoke-test")
))]
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

    #[cfg(all(
        feature = "process-load-smoke-test",
        not(feature = "process-run-smoke-test")
    ))]
    pub const fn image_page_count(&self) -> usize {
        self.image_page_count
    }

    #[cfg(feature = "process-runtime")]
    pub fn root_address(&self) -> Result<u64, ProcessLoadError> {
        self.address_space
            .as_ref()
            .map(AddressSpace::root_address)
            .ok_or(ProcessLoadError::AddressSpace(AddressSpaceError::Destroyed))
    }

    #[cfg(feature = "process-runtime")]
    pub fn address_space(&self) -> Result<&AddressSpace, ProcessLoadError> {
        self.address_space
            .as_ref()
            .ok_or(ProcessLoadError::AddressSpace(AddressSpaceError::Destroyed))
    }

    #[cfg(all(
        feature = "process-load-smoke-test",
        not(feature = "process-run-smoke-test")
    ))]
    pub fn translate(&self, address: u64) -> Result<PageMapping, ProcessLoadError> {
        self.address_space
            .as_ref()
            .ok_or(ProcessLoadError::AddressSpace(AddressSpaceError::Destroyed))?
            .translate(address)
            .map_err(ProcessLoadError::AddressSpace)
    }

    #[cfg(feature = "process-runtime")]
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
#[cfg(all(
    feature = "process-load-smoke-test",
    not(feature = "process-run-smoke-test")
))]
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

#[cfg(all(
    feature = "process-load-smoke-test",
    not(feature = "process-run-smoke-test")
))]
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
    let loaded_opcode =
        unsafe { ptr::read_volatile((entry.physical_page + entry_offset) as *const u8) };
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
            let bss_byte =
                unsafe { ptr::read_volatile((bss.physical_page + bss_offset) as *const u8) };
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

#[cfg(feature = "process-runtime")]
#[repr(C, align(4096))]
struct InitKernelStack([u8; INIT_KERNEL_STACK_BYTES]);

#[cfg(feature = "process-runtime")]
static mut PROCESS_KERNEL_STACKS: [InitKernelStack; MAX_PROCESS_RECORDS] = [
    InitKernelStack([0; INIT_KERNEL_STACK_BYTES]),
    InitKernelStack([0; INIT_KERNEL_STACK_BYTES]),
];
#[cfg(feature = "process-runtime")]
static mut REAPER_KERNEL_STACK: InitKernelStack = InitKernelStack([0; INIT_KERNEL_STACK_BYTES]);
#[cfg(feature = "process-runtime")]
static mut PROCESS_FX_STATES: [user::FxState; MAX_PROCESS_RECORDS] =
    [user::FxState::zeroed(), user::FxState::zeroed()];
#[cfg(feature = "process-runtime")]
static mut REAPER_FX_STATE: user::FxState = user::FxState::zeroed();
#[cfg(feature = "process-runtime")]
static mut PROCESS_OBJECTS: [Option<LoadedProcess>; MAX_PROCESS_RECORDS] = [None, None];
#[cfg(feature = "process-runtime")]
static mut PROCESS_STACK_GUARDS: [bool; MAX_PROCESS_RECORDS] = [false; MAX_PROCESS_RECORDS];
#[cfg(feature = "process-runtime")]
static mut USERLAND_FILESYSTEM: Option<RamFs<USERLAND_FS_NODE_COUNT, USERLAND_FS_FILE_CAPACITY>> =
    None;
#[cfg(feature = "process-runtime")]
static mut PROCESS_SCHEDULER: Scheduler<3> = Scheduler::new();
#[cfg(feature = "process-runtime")]
const MAX_PROCESS_RECORDS: usize = 2;

/// Architecture-owned resources associated with one scheduler user task.
#[cfg(feature = "process-runtime")]
#[derive(Clone, Copy)]
struct ProcessRuntimeRecord {
    task_id: TaskId,
    process: *mut Option<LoadedProcess>,
    kernel_stack_bottom: u64,
    kernel_stack_top: u64,
    fx_state: usize,
}

#[cfg(feature = "process-runtime")]
static mut PROCESS_REGISTRY: [Option<ProcessRuntimeRecord>; MAX_PROCESS_RECORDS] =
    [None; MAX_PROCESS_RECORDS];
#[cfg(feature = "process-runtime")]
static mut REAPER_TASK_ID: Option<TaskId> = None;
#[cfg(feature = "process-fault-smoke-test")]
static mut INIT_FAULT_VECTOR: u64 = u64::MAX;
#[cfg(feature = "process-worker-fault-smoke-test")]
static mut PROCESS_WORKER_FAULT_VECTOR: u64 = u64::MAX;
#[cfg(feature = "process-worker-rollback-smoke-test")]
static mut PROCESS_WORKER_ROLLBACK_INJECTED: bool = false;
#[cfg(feature = "process-run-smoke-test")]
static mut INIT_YIELD_COUNT: u64 = 0;
#[cfg(feature = "process-run-smoke-test")]
static mut INIT_SYSCALL_COUNT: u64 = 0;
#[cfg(feature = "process-multi-smoke-test")]
static mut PROCESS_MULTI_SWITCH_COUNT: u64 = 0;

#[cfg(feature = "process-runtime")]
fn initialize_userland_filesystem() {
    let mut filesystem = RamFs::<USERLAND_FS_NODE_COUNT, USERLAND_FS_FILE_CAPACITY>::new()
        .unwrap_or_else(|_| init_runtime_failure(b"P7.4: could not create userland RAMFS.\r\n"));
    let root = filesystem.root();
    if create_directory_path(&mut filesystem, root, "/etc").is_err() {
        init_runtime_failure(b"P7.4: could not create /etc in userland RAMFS.\\r\n");
    }
    let config = create_file_path(&mut filesystem, root, "/etc/exhi.conf")
        .unwrap_or_else(|_| init_runtime_failure(b"P7.4: could not create /etc/exhi.conf.\\r\n"));
    if filesystem.write(config, 0, USERLAND_CONFIG_CONTENT).ok()
        != Some(USERLAND_CONFIG_CONTENT.len())
    {
        init_runtime_failure(b"P7.4: could not populate /etc/exhi.conf.\\r\n");
    }
    unsafe {
        ptr::write(ptr::addr_of_mut!(USERLAND_FILESYSTEM), Some(filesystem));
    }
}

#[cfg(feature = "process-runtime")]
fn runtime_stack_bounds(storage: u64) -> Option<(u64, u64)> {
    let bottom = storage.checked_add(PAGE_SIZE)?;
    let top = storage.checked_add(INIT_KERNEL_STACK_BYTES as u64)?;
    if bottom >= top || storage & PAGE_MASK != 0 || top & 0xF != 0 {
        return None;
    }
    Some((bottom, top))
}

#[cfg(feature = "process-runtime")]
fn process_kernel_stack_bounds(slot: usize) -> Option<(u64, u64)> {
    if slot >= MAX_PROCESS_RECORDS {
        return None;
    }
    let stacks = ptr::addr_of_mut!(PROCESS_KERNEL_STACKS);
    let storage = unsafe { (*stacks).as_mut_ptr().add(slot).cast::<u8>() as u64 };
    runtime_stack_bounds(storage)
}

#[cfg(feature = "process-runtime")]
fn process_object_pointer(slot: usize) -> *mut Option<LoadedProcess> {
    let objects = ptr::addr_of_mut!(PROCESS_OBJECTS);
    unsafe { (*objects).as_mut_ptr().add(slot) }
}

#[cfg(feature = "process-runtime")]
fn process_fx_state_pointer(slot: usize) -> *mut user::FxState {
    let states = ptr::addr_of_mut!(PROCESS_FX_STATES);
    unsafe { (*states).as_mut_ptr().add(slot) }
}

#[cfg(feature = "process-runtime")]
fn register_process_record(
    slot: usize,
    task_id: TaskId,
    process: *mut Option<LoadedProcess>,
    kernel_stack_bottom: u64,
    kernel_stack_top: u64,
    fx_state: usize,
) -> bool {
    if slot >= MAX_PROCESS_RECORDS
        || process.is_null()
        || kernel_stack_bottom == 0
        || kernel_stack_top <= kernel_stack_bottom
        || fx_state == 0
    {
        return false;
    }

    let registry = ptr::addr_of_mut!(PROCESS_REGISTRY);
    let entry = unsafe { &mut *(*registry).as_mut_ptr().add(slot) };
    if entry.is_some() {
        return false;
    }
    *entry = Some(ProcessRuntimeRecord {
        task_id,
        process,
        kernel_stack_bottom,
        kernel_stack_top,
        fx_state,
    });
    true
}

#[cfg(feature = "process-runtime")]
fn process_record_for_task(task_id: TaskId) -> Option<ProcessRuntimeRecord> {
    let registry = ptr::addr_of!(PROCESS_REGISTRY);
    for slot in 0..MAX_PROCESS_RECORDS {
        let entry = unsafe { ptr::read_volatile((*registry).as_ptr().add(slot)) };
        if let Some(record) = entry {
            if record.task_id == task_id {
                return Some(record);
            }
        }
    }
    None
}

#[cfg(feature = "process-runtime")]
fn remove_process_record(task_id: TaskId) -> bool {
    let registry = ptr::addr_of_mut!(PROCESS_REGISTRY);
    for slot in 0..MAX_PROCESS_RECORDS {
        let entry = unsafe { &mut *(*registry).as_mut_ptr().add(slot) };
        if entry.is_some_and(|record| record.task_id == task_id) {
            *entry = None;
            return true;
        }
    }
    false
}

#[cfg(feature = "process-runtime")]
fn reaper_kernel_stack_bounds() -> Option<(u64, u64)> {
    let storage = unsafe { ptr::addr_of_mut!(REAPER_KERNEL_STACK.0).cast::<u8>() as u64 };
    runtime_stack_bounds(storage)
}

#[cfg(feature = "process-stack-guard-smoke-test")]
fn verify_kernel_stack_guard(bottom: u64, top: u64) -> bool {
    let expected_usable_size = INIT_KERNEL_STACK_BYTES as u64 - PAGE_SIZE;
    let Some(guard_page) = bottom.checked_sub(PAGE_SIZE) else {
        return false;
    };
    if bottom & PAGE_MASK != 0
        || top & 0xF != 0
        || top.checked_sub(bottom) != Some(expected_usable_size)
        || vmm::query_page(guard_page) != Err(vmm::VmmError::NotMapped)
    {
        return false;
    }

    let mut page = bottom;
    while page < top {
        let Ok(mapping) = vmm::query_page(page) else {
            return false;
        };
        if mapping.virtual_page != page
            || mapping.physical_page != page
            || mapping.flags.user
            || !mapping.flags.writable
        {
            return false;
        }
        page = match page.checked_add(PAGE_SIZE) {
            Some(next) => next,
            None => return false,
        };
    }

    page == top
}

#[cfg(feature = "process-stack-guard-smoke-test")]
fn verify_all_process_kernel_stack_guards() -> bool {
    for slot in 0..MAX_PROCESS_RECORDS {
        let Some((bottom, top)) = process_kernel_stack_bounds(slot) else {
            return false;
        };
        if !verify_kernel_stack_guard(bottom, top) {
            return false;
        }
    }

    let Some((reaper_bottom, reaper_top)) = reaper_kernel_stack_bounds() else {
        return false;
    };
    verify_kernel_stack_guard(reaper_bottom, reaper_top)
}

#[cfg(feature = "process-runtime")]
fn install_runtime_stack_guard(storage: u64, bounds: (u64, u64)) -> bool {
    let (bottom, top) = bounds;
    let Some(size) = top.checked_sub(bottom) else {
        return false;
    };
    stack_guard::install(stack_guard::StackGuardReservation {
        stack_base: bottom,
        stack_size: size,
        stack_end: top,
        guard_page: storage,
    })
    .is_ok()
}

#[cfg(feature = "process-runtime")]
fn ensure_process_stack_guard(slot: usize, bounds: (u64, u64)) -> bool {
    if slot >= MAX_PROCESS_RECORDS {
        return false;
    }

    let stacks = ptr::addr_of_mut!(PROCESS_KERNEL_STACKS);
    let storage = unsafe { (*stacks).as_mut_ptr().add(slot).cast::<u8>() as u64 };
    if runtime_stack_bounds(storage) != Some(bounds) {
        return false;
    }

    let guards = ptr::addr_of_mut!(PROCESS_STACK_GUARDS);
    if unsafe { ptr::read_volatile((*guards).as_ptr().add(slot)) } {
        return true;
    }
    if !install_runtime_stack_guard(storage, bounds) {
        return false;
    }
    unsafe {
        ptr::write_volatile((*guards).as_mut_ptr().add(slot), true);
    }
    true
}

#[cfg(feature = "process-runtime")]
unsafe extern "sysv64" {
    fn exhi_process_reaper_bootstrap() -> !;
    fn exhi_process_reaper_enter(kernel_stack_top: u64) -> !;
}

#[cfg(feature = "process-runtime")]
global_asm!(
    r#"
    .section .text
    .global exhi_process_reaper_bootstrap
    .type exhi_process_reaper_bootstrap, @function
exhi_process_reaper_bootstrap:
    # A same-CPL IRETQ consumes RIP/CS/RFLAGS only. Skip the synthetic RSP/SS
    # tail, align for the SysV call, then enter the reaper on its own stack.
    add rsp, 16
    and rsp, -16
    call exhi_process_reaper_main
    ud2

    .global exhi_process_reaper_enter
    .type exhi_process_reaper_enter, @function
exhi_process_reaper_enter:
    cli
    mov rsp, rdi
    and rsp, -16
    call exhi_process_reaper_main
    ud2
    "#
);

#[cfg(feature = "process-runtime")]
struct InitServices {
    scheduler: *mut Scheduler<3>,
}

#[cfg(feature = "process-runtime")]
impl SyscallServices for InitServices {
    type SwitchTarget = TaskId;

    fn current_capabilities(&self) -> CapabilitySet {
        if self.scheduler.is_null() {
            return CapabilitySet::EMPTY;
        }
        unsafe { (*self.scheduler).current_capabilities() }
    }

    fn yield_current(&mut self) -> Result<Option<Self::SwitchTarget>, SyscallError> {
        if self.scheduler.is_null() {
            return Err(SyscallError::Io);
        }
        let decision =
            unsafe { (*self.scheduler).yield_current() }.map_err(|_| SyscallError::Io)?;
        #[cfg(feature = "process-multi-smoke-test")]
        if matches!(&decision, ScheduleDecision::Switch { .. }) {
            unsafe {
                let count = ptr::read_volatile(ptr::addr_of!(PROCESS_MULTI_SWITCH_COUNT));
                ptr::write_volatile(
                    ptr::addr_of_mut!(PROCESS_MULTI_SWITCH_COUNT),
                    count.saturating_add(1),
                );
            }
        }
        #[cfg(feature = "process-run-smoke-test")]
        unsafe {
            let count = ptr::read_volatile(ptr::addr_of!(INIT_YIELD_COUNT));
            ptr::write_volatile(ptr::addr_of_mut!(INIT_YIELD_COUNT), count.saturating_add(1));
        }
        Ok(match decision {
            ScheduleDecision::Switch { to, .. } => Some(to),
            ScheduleDecision::Continue { .. } => None,
        })
    }

    fn exit_current(&mut self) -> Result<Self::SwitchTarget, SyscallError> {
        let scheduler = self.scheduler;
        if scheduler.is_null() {
            return Err(SyscallError::Io);
        }

        if let Some(next) =
            unsafe { (*scheduler).finish_current() }.map_err(|_| SyscallError::Io)?
        {
            #[cfg(feature = "process-multi-smoke-test")]
            unsafe {
                let count = ptr::read_volatile(ptr::addr_of!(PROCESS_MULTI_SWITCH_COUNT));
                ptr::write_volatile(
                    ptr::addr_of_mut!(PROCESS_MULTI_SWITCH_COUNT),
                    count.saturating_add(1),
                );
            }
            return Ok(next);
        }

        // When the last user process exits, finish_current enters idle because
        // the kernel reaper is Blocked. Wake it; the dispatcher transfers directly
        // to its dedicated kernel stack without an artificial same-CPL IRETQ.
        let reaper =
            unsafe { ptr::read_volatile(ptr::addr_of!(REAPER_TASK_ID)) }.ok_or(SyscallError::Io)?;
        unsafe {
            (*scheduler).wake(reaper).map_err(|_| SyscallError::Io)?;
            (*scheduler)
                .schedule_from_idle()
                .map_err(|_| SyscallError::Io)?
                .ok_or(SyscallError::Io)
        }
    }

    fn console_write(&mut self, user_address: u64, length: u64) -> Result<u64, SyscallError> {
        if self.scheduler.is_null() {
            return Err(SyscallError::Io);
        }
        let current =
            unsafe { (*self.scheduler).current().map(|task| task.id()) }.ok_or(SyscallError::Io)?;
        let record = process_record_for_task(current).ok_or(SyscallError::AccessDenied)?;
        let length = usize::try_from(length).map_err(|_| SyscallError::InvalidArgument)?;
        if length > MAX_INIT_CONSOLE_WRITE {
            return Err(SyscallError::InvalidArgument);
        }
        user_address
            .checked_add(length as u64)
            .ok_or(SyscallError::InvalidArgument)?;

        let process = unsafe { (*record.process).as_ref() }.ok_or(SyscallError::Io)?;
        let space = process.address_space().map_err(|_| SyscallError::Io)?;
        let mut buffer = [0u8; 256];
        let mut copied = 0usize;
        while copied < length {
            let count = (length - copied).min(buffer.len());
            let address = user_address
                .checked_add(copied as u64)
                .ok_or(SyscallError::InvalidArgument)?;
            user::copy_from_user(space, address, &mut buffer[..count])
                .map_err(|_| SyscallError::InvalidArgument)?;
            serial::COM1.write(&buffer[..count]);
            copied += count;
        }
        Ok(copied as u64)
    }

    fn console_read(&mut self, user_address: u64, length: u64) -> Result<u64, SyscallError> {
        if self.scheduler.is_null() {
            return Err(SyscallError::Io);
        }
        let current =
            unsafe { (*self.scheduler).current().map(|task| task.id()) }.ok_or(SyscallError::Io)?;
        let record = process_record_for_task(current).ok_or(SyscallError::AccessDenied)?;
        let length = usize::try_from(length).map_err(|_| SyscallError::InvalidArgument)?;
        if length > MAX_INIT_CONSOLE_READ {
            return Err(SyscallError::InvalidArgument);
        }
        if length == 0 {
            return Ok(0);
        }
        user_address
            .checked_add(length as u64)
            .ok_or(SyscallError::InvalidArgument)?;

        let process = unsafe { (*record.process).as_ref() }.ok_or(SyscallError::Io)?;
        let space = process.address_space().map_err(|_| SyscallError::Io)?;
        user::validate_user_range(space, user_address, length, user::UserAccess::Write)
            .map_err(|_| SyscallError::InvalidArgument)?;

        let mut buffer = [0u8; MAX_INIT_CONSOLE_READ];
        let mut read = 0usize;
        while read < length {
            let Some(byte) = serial::COM1.try_read_byte() else {
                break;
            };
            buffer[read] = byte;
            read += 1;
        }
        if read != 0 {
            user::copy_to_user(space, user_address, &buffer[..read])
                .map_err(|_| SyscallError::InvalidArgument)?;
        }
        Ok(read as u64)
    }

    fn spawn_worker(&mut self) -> Result<u64, SyscallError> {
        spawn_worker_process(self.scheduler)
    }

    fn file_read(
        &mut self,
        path_address: u64,
        path_length: u64,
        offset: u64,
        user_buffer: u64,
        buffer_length: u64,
    ) -> Result<u64, SyscallError> {
        if self.scheduler.is_null() {
            return Err(SyscallError::Io);
        }
        let current =
            unsafe { (*self.scheduler).current().map(|task| task.id()) }.ok_or(SyscallError::Io)?;
        let record = process_record_for_task(current).ok_or(SyscallError::AccessDenied)?;
        let path_length =
            usize::try_from(path_length).map_err(|_| SyscallError::InvalidArgument)?;
        let offset = usize::try_from(offset).map_err(|_| SyscallError::InvalidArgument)?;
        let buffer_length =
            usize::try_from(buffer_length).map_err(|_| SyscallError::InvalidArgument)?;
        if path_length == 0
            || path_length > USERLAND_MAX_PATH
            || buffer_length > USERLAND_MAX_FILE_READ
        {
            return Err(SyscallError::InvalidArgument);
        }
        path_address
            .checked_add(path_length as u64)
            .ok_or(SyscallError::InvalidArgument)?;
        user_buffer
            .checked_add(buffer_length as u64)
            .ok_or(SyscallError::InvalidArgument)?;

        let process = unsafe { (*record.process).as_ref() }.ok_or(SyscallError::Io)?;
        let space = process.address_space().map_err(|_| SyscallError::Io)?;
        user::validate_user_range(space, path_address, path_length, user::UserAccess::Read)
            .map_err(|_| SyscallError::InvalidArgument)?;
        if buffer_length != 0 {
            user::validate_user_range(space, user_buffer, buffer_length, user::UserAccess::Write)
                .map_err(|_| SyscallError::InvalidArgument)?;
        }

        let mut path_bytes = [0u8; USERLAND_MAX_PATH];
        user::copy_from_user(space, path_address, &mut path_bytes[..path_length])
            .map_err(|_| SyscallError::InvalidArgument)?;
        let path = core::str::from_utf8(&path_bytes[..path_length])
            .map_err(|_| SyscallError::InvalidArgument)?;
        if !path.starts_with('/') || path.as_bytes().contains(&0) {
            return Err(SyscallError::InvalidArgument);
        }

        let filesystem_slot = ptr::addr_of!(USERLAND_FILESYSTEM);
        let filesystem = unsafe { (*filesystem_slot).as_ref() }.ok_or(SyscallError::Io)?;
        let node =
            resolve_path(filesystem, filesystem.root(), path).map_err(|_| SyscallError::Io)?;
        if filesystem
            .metadata(node)
            .map_err(|_| SyscallError::Io)?
            .kind
            != NodeKind::File
        {
            return Err(SyscallError::InvalidArgument);
        }

        let mut buffer = [0u8; USERLAND_MAX_FILE_READ];
        let read = filesystem
            .read(node, offset, &mut buffer[..buffer_length])
            .map_err(|_| SyscallError::Io)?;
        if read != 0 {
            user::copy_to_user(space, user_buffer, &buffer[..read])
                .map_err(|_| SyscallError::InvalidArgument)?;
        }
        Ok(read as u64)
    }
}

#[cfg(feature = "process-runtime")]
fn spawn_worker_process(scheduler: *mut Scheduler<3>) -> Result<u64, SyscallError> {
    if scheduler.is_null() {
        return Err(SyscallError::Io);
    }

    let current_id =
        unsafe { (*scheduler).current().map(|task| task.id()) }.ok_or(SyscallError::Io)?;
    if unsafe { (*scheduler).current_capabilities() }
        .authorize(Capability::TaskSpawn)
        .is_err()
        || process_record_for_task(current_id).is_none()
    {
        return Err(SyscallError::AccessDenied);
    }

    let registry = ptr::addr_of!(PROCESS_REGISTRY);
    let mut free_slot = None;
    for slot in 0..MAX_PROCESS_RECORDS {
        let record = unsafe { ptr::read_volatile((*registry).as_ptr().add(slot)) };
        let process_is_empty = unsafe { (*process_object_pointer(slot)).is_none() };
        if record.is_none() && process_is_empty {
            free_slot = Some(slot);
            break;
        }
    }
    let slot = free_slot.ok_or(SyscallError::Io)?;
    let process_slot = process_object_pointer(slot);

    let setup = (|| {
        address_space::restore_kernel().map_err(|_| SyscallError::Io)?;
        user::clear_active_kernel_stack().map_err(|_| SyscallError::Io)?;

        let process =
            LoadedProcess::load(crate::EMBEDDED_WORKER_ELF).map_err(|_| SyscallError::Io)?;
        unsafe {
            ptr::write(process_slot, Some(process));
        }

        let bounds = process_kernel_stack_bounds(slot).ok_or(SyscallError::Io)?;
        if !ensure_process_stack_guard(slot, bounds) {
            return Err(SyscallError::Io);
        }

        let fx_state = process_fx_state_pointer(slot);
        unsafe { (*fx_state).initialize() }.map_err(|_| SyscallError::Io)?;

        let process = unsafe { (*process_slot).as_ref() }.ok_or(SyscallError::Io)?;
        process.activate().map_err(|_| SyscallError::Io)?;
        let space = process.address_space().map_err(|_| SyscallError::Io)?;
        let frame = user::prepare_entry(
            space,
            bounds.0,
            bounds.1,
            process.entry_point(),
            process.user_stack_pointer(),
        )
        .map_err(|_| SyscallError::Io)?;

        // Inject exactly one post-activation failure so the rollback path must
        // destroy a live worker address space and return its slot to the parent.
        #[cfg(feature = "process-worker-rollback-smoke-test")]
        unsafe {
            if !ptr::read_volatile(ptr::addr_of!(PROCESS_WORKER_ROLLBACK_INJECTED)) {
                ptr::write_volatile(ptr::addr_of_mut!(PROCESS_WORKER_ROLLBACK_INJECTED), true);
                serial::COM1.write(
                    b"P7.1: injected worker setup failure after address-space activation.\r\n",
                );
                return Err(SyscallError::Io);
            }
        }

        let frame_address = bounds
            .1
            .checked_sub(size_of::<user::UserEntryFrame>() as u64)
            .ok_or(SyscallError::Io)?;
        if frame_address & 0xF != 0 {
            return Err(SyscallError::Io);
        }
        unsafe {
            ptr::write(frame_address as *mut user::UserEntryFrame, frame);
        }

        let capabilities = CapabilitySet::EMPTY
            .with(Capability::TaskYield)
            .with(Capability::TaskExit)
            .with(Capability::ConsoleWrite);
        let task_id = unsafe {
            (*scheduler)
                .spawn_user(
                    "worker",
                    TaskContext::new(frame_address as usize),
                    UserTaskMetadata::new_with_capabilities(
                        process.root_address().map_err(|_| SyscallError::Io)?,
                        process.user_stack_pointer(),
                        bounds.1,
                        fx_state as usize,
                        capabilities,
                    ),
                )
                .map_err(|_| SyscallError::Io)?
        };
        if !register_process_record(
            slot,
            task_id,
            process_slot,
            bounds.0,
            bounds.1,
            fx_state as usize,
        ) {
            init_runtime_failure(b"P7.1: could not register spawned worker process.\r\n");
        }
        Ok(task_id)
    })();

    if setup.is_err() {
        let destroyed = unsafe {
            match (*process_slot).as_mut() {
                Some(process) => process.destroy().is_ok(),
                None => true,
            }
        };
        if !destroyed {
            init_runtime_failure(b"P7.1: failed to roll back an incomplete worker spawn.\r\n");
        }
        unsafe {
            ptr::write(process_slot, None);
        }
    }

    let _ = select_process_context(current_id);
    setup.map(|task_id| task_id.get())
}

#[cfg(feature = "process-runtime")]
fn select_process_context(id: TaskId) -> usize {
    let scheduler = ptr::addr_of_mut!(PROCESS_SCHEDULER);
    let task = unsafe { (*scheduler).get(id) }
        .unwrap_or_else(|| init_runtime_failure(b"P7.1: scheduler selected an unknown task.\r\n"));
    let frame = task.context().saved_stack_pointer();
    if frame == 0 || frame & 0xF != 0 {
        init_runtime_failure(b"P7.1: selected task has an invalid saved frame.\r\n");
    }

    if let Some(metadata) = task.user_metadata() {
        let record = process_record_for_task(id).unwrap_or_else(|| {
            init_runtime_failure(b"P7.1: selected user task has no process record.\r\n")
        });
        let frame_end =
            (frame as u64).checked_add(size_of::<syscall::UserTaskSyscallFrame>() as u64);
        if (frame as u64) < record.kernel_stack_bottom
            || frame_end.is_none_or(|end| end > record.kernel_stack_top)
            || metadata.kernel_stack_top() != record.kernel_stack_top
            || metadata.fx_state_pointer() != record.fx_state
        {
            init_runtime_failure(
                b"P7.1: selected process frame or architecture metadata is invalid.\r\n",
            );
        }

        let process = unsafe { (*record.process).as_ref() }
            .unwrap_or_else(|| init_runtime_failure(b"P7.1: selected process is missing.\r\n"));
        if process.root_address().ok() != Some(metadata.address_space_root())
            || process.activate().is_err()
        {
            init_runtime_failure(b"P7.1: failed to activate selected process address space.\r\n");
        }
        let space = process.address_space().unwrap_or_else(|_| {
            init_runtime_failure(b"P7.1: selected process address space is unavailable.\r\n")
        });
        if user::activate_kernel_stack(space, record.kernel_stack_bottom, record.kernel_stack_top)
            .is_err()
            || !syscall::update_current_task_context(record.kernel_stack_top, record.fx_state)
        {
            init_runtime_failure(b"P7.1: failed to update selected user-task entry context.\r\n");
        }
    } else {
        let reaper_id = unsafe { ptr::read_volatile(ptr::addr_of!(REAPER_TASK_ID)) }
            .unwrap_or_else(|| init_runtime_failure(b"P7.1: reaper task id is unavailable.\r\n"));
        if id != reaper_id {
            init_runtime_failure(b"P7.1: scheduler selected an unexpected kernel task.\r\n");
        }
        let Some((bottom, top)) = reaper_kernel_stack_bounds() else {
            init_runtime_failure(b"P7.1: reaper kernel stack bounds are invalid.\r\n");
        };
        let frame_end =
            (frame as u64).checked_add(size_of::<syscall::UserTaskSyscallFrame>() as u64);
        if (frame as u64) < bottom || frame_end.is_none_or(|end| end > top) {
            init_runtime_failure(b"P7.1: reaper frame escaped its kernel stack.\r\n");
        }
        if address_space::restore_kernel().is_err()
            || user::clear_active_kernel_stack().is_err()
            || !gdt::set_rsp0(top)
            || gdt::read_rsp0() != top
            || !syscall::update_current_task_context(top, ptr::addr_of!(REAPER_FX_STATE) as usize)
        {
            init_runtime_failure(b"P7.1: failed to activate the kernel reaper context.\r\n");
        }
    }

    frame
}

/// Transfers terminal kernel-reaper execution onto its own stack after all
/// user processes have switched away from their private address spaces.
#[cfg(feature = "process-runtime")]
fn handoff_to_kernel_reaper() -> ! {
    let scheduler = ptr::addr_of_mut!(PROCESS_SCHEDULER);
    let current_id = unsafe { (*scheduler).current().map(|task| task.id()) };
    let reaper_id = unsafe { ptr::read_volatile(ptr::addr_of!(REAPER_TASK_ID)) };
    if current_id.is_none() || current_id != reaper_id {
        init_runtime_failure(
            b"P7.1: direct reaper handoff selected the wrong scheduler task.\\r\n",
        );
    }

    let Some((_, stack_top)) = reaper_kernel_stack_bounds() else {
        init_runtime_failure(b"P7.1: reaper stack bounds are invalid during handoff.\\r\n");
    };
    if address_space::restore_kernel().is_err()
        || user::clear_active_kernel_stack().is_err()
        || !gdt::set_rsp0(stack_top)
        || gdt::read_rsp0() != stack_top
        || !syscall::update_current_task_context(stack_top, ptr::addr_of!(REAPER_FX_STATE) as usize)
    {
        init_runtime_failure(b"P7.1: could not prepare the direct kernel-reaper context.\\r\n");
    }

    unsafe {
        core::arch::asm!(
            "fxrstor64 [{}]",
            in(reg) ptr::addr_of!(REAPER_FX_STATE) as usize,
            options(nostack, preserves_flags)
        );
        exhi_process_reaper_enter(stack_top);
    }
}

/// Captures the stable syscall frame, saves the current process context and
/// dispatches through the shared capability gate and production scheduler.
#[cfg(feature = "process-runtime")]
#[no_mangle]
extern "sysv64" fn exhi_init_syscall_dispatch(frame: *mut syscall::UserTaskSyscallFrame) -> usize {
    if frame.is_null() {
        init_runtime_failure(b"P7.1: null syscall frame received.\r\n");
    }

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }
    let frame_ref = unsafe { &mut *frame };
    let scheduler = ptr::addr_of_mut!(PROCESS_SCHEDULER);
    let current_id = unsafe { (*scheduler).current().map(|task| task.id()) }.unwrap_or_else(|| {
        init_runtime_failure(b"P7.1: syscall arrived without a current task.\r\n")
    });
    let record = process_record_for_task(current_id).unwrap_or_else(|| {
        init_runtime_failure(b"P7.1: syscall has no registered process record.\r\n")
    });
    let frame_address = frame as usize;
    let frame_end =
        (frame_address as u64).checked_add(size_of::<syscall::UserTaskSyscallFrame>() as u64);
    if frame_address == 0
        || frame_address & 0xF != 0
        || (frame_address as u64) < record.kernel_stack_bottom
        || frame_end.is_none_or(|end| end > record.kernel_stack_top)
    {
        init_runtime_failure(b"P7.1: syscall frame escaped the active process kernel stack.\r\n");
    }

    unsafe {
        let task = (*scheduler).get_mut(current_id).unwrap_or_else(|| {
            init_runtime_failure(b"P7.1: active process task record is missing.\r\n")
        });
        task.context_mut().set_saved_stack_pointer(frame_address);
        let user_rsp = ptr::read_volatile(ptr::addr_of!((*frame).rsp));
        task.user_metadata_mut()
            .unwrap_or_else(|| init_runtime_failure(b"P7.1: process task metadata is missing.\r\n"))
            .set_user_stack_pointer(user_rsp);
    }

    #[cfg(feature = "process-run-smoke-test")]
    unsafe {
        let count = ptr::read_volatile(ptr::addr_of!(INIT_SYSCALL_COUNT));
        ptr::write_volatile(
            ptr::addr_of_mut!(INIT_SYSCALL_COUNT),
            count.saturating_add(1),
        );
    }

    let request = SyscallRequest::from_registers(
        frame_ref.rax,
        frame_ref.rdi,
        frame_ref.rsi,
        frame_ref.rdx,
        frame_ref.r10,
        frame_ref.r8,
        frame_ref.r9,
    );
    match dispatch_syscall(request, &mut InitServices { scheduler }) {
        DispatchOutcome::Return(value) => {
            unsafe {
                ptr::write_volatile(ptr::addr_of_mut!((*frame).rax), value);
            }
            frame_address
        }
        DispatchOutcome::Switch(next) => {
            if Some(next) == unsafe { ptr::read_volatile(ptr::addr_of!(REAPER_TASK_ID)) } {
                handoff_to_kernel_reaper();
            }
            select_process_context(next)
        }
    }
}

#[cfg(feature = "process-runtime")]
fn build_reaper_frame(kernel_stack_top: u64) -> *mut user::UserEntryFrame {
    let frame_address = kernel_stack_top
        .checked_sub(size_of::<user::UserEntryFrame>() as u64)
        .unwrap_or_else(|| init_runtime_failure(b"P7.1: reaper stack is too small.\r\n"));
    if frame_address & 0xF != 0 {
        init_runtime_failure(b"P7.1: reaper frame is misaligned.\r\n");
    }
    let frame = user::UserEntryFrame {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: 0,
        r11: 0,
        r10: 0,
        r9: 0,
        r8: 0,
        rdi: 0,
        rsi: 0,
        rbp: 0,
        rbx: 0,
        rdx: 0,
        rcx: 0,
        rax: 0,
        rip: exhi_process_reaper_bootstrap as *const () as u64,
        cs: u64::from(gdt::KERNEL_CODE_SELECTOR),
        rflags: 0x202,
        rsp: kernel_stack_top,
        ss: u64::from(gdt::KERNEL_DATA_SELECTOR),
    };
    let pointer = frame_address as *mut user::UserEntryFrame;
    unsafe {
        ptr::write(pointer, frame);
    }
    pointer
}

#[cfg(all(feature = "process-runtime", not(feature = "process-multi-smoke-test")))]
pub fn run_init(image: &[u8]) -> ! {
    run_processes(image, None)
}

#[cfg(feature = "process-multi-smoke-test")]
pub fn run_process_pair(first_image: &[u8], second_image: &[u8]) -> ! {
    run_processes(first_image, Some(second_image))
}

#[cfg(feature = "process-runtime")]
fn run_processes(first_image: &[u8], second_image: Option<&[u8]>) -> ! {
    let process_count = if second_image.is_some() { 2 } else { 1 };
    if process_count > MAX_PROCESS_RECORDS {
        init_runtime_failure(b"P7.1: process set exceeds runtime capacity.\r\n");
    }

    initialize_userland_filesystem();
    let scheduler = ptr::addr_of_mut!(PROCESS_SCHEDULER);
    let mut first_task_id = None;

    for slot in 0..process_count {
        if slot != 0
            && (address_space::restore_kernel().is_err()
                || user::clear_active_kernel_stack().is_err())
        {
            init_runtime_failure(b"P7.1: could not restore kernel mappings between processes.\r\n");
        }
        let image = if slot == 0 {
            first_image
        } else {
            second_image.unwrap_or_else(|| {
                init_runtime_failure(b"P7.1: second process image is missing.\r\n")
            })
        };
        let process =
            LoadedProcess::load(image).unwrap_or_else(|error| init_runtime_failure_fmt(error));
        let process_slot = process_object_pointer(slot);
        unsafe {
            ptr::write(process_slot, Some(process));
        }

        let Some((stack_bottom, stack_top)) = process_kernel_stack_bounds(slot) else {
            init_runtime_failure(b"P7.1: process kernel stack bounds are invalid.\r\n");
        };
        if !ensure_process_stack_guard(slot, (stack_bottom, stack_top)) {
            init_runtime_failure(b"P7.1: could not install a process kernel-stack guard.\r\n");
        }

        let fx_pointer = process_fx_state_pointer(slot);
        if unsafe { (*fx_pointer).initialize() }.is_err() {
            init_runtime_failure(b"P7.1: could not initialize process FXSAVE state.\r\n");
        }

        let process = unsafe { (*process_slot).as_ref() }
            .unwrap_or_else(|| init_runtime_failure(b"P7.1: process ownership was lost.\r\n"));
        if process.activate().is_err() {
            init_runtime_failure(b"P7.1: could not activate process address space.\r\n");
        }
        let space = process.address_space().unwrap_or_else(|_| {
            init_runtime_failure(b"P7.1: process address space is unavailable.\r\n")
        });
        let frame = user::prepare_entry(
            space,
            stack_bottom,
            stack_top,
            process.entry_point(),
            process.user_stack_pointer(),
        )
        .unwrap_or_else(|_| {
            init_runtime_failure(b"P7.1: process user-entry validation failed.\r\n")
        });

        let frame_address = stack_top
            .checked_sub(size_of::<user::UserEntryFrame>() as u64)
            .unwrap_or_else(|| {
                init_runtime_failure(b"P7.1: process kernel stack is too small.\r\n")
            });
        if frame_address & 0xF != 0 {
            init_runtime_failure(b"P7.1: process entry frame is misaligned.\r\n");
        }
        let frame_pointer = frame_address as *mut user::UserEntryFrame;
        unsafe {
            ptr::write(frame_pointer, frame);
        }

        let name = if slot == 0 { "init" } else { "worker" };
        let mut capabilities = CapabilitySet::EMPTY
            .with(Capability::TaskYield)
            .with(Capability::TaskExit)
            .with(Capability::ConsoleWrite);
        if slot == 0 {
            capabilities = capabilities
                .with(Capability::ConsoleRead)
                .with(Capability::FileRead)
                .with(Capability::TaskSpawn);
        }
        let task_id = unsafe {
            (*scheduler)
                .spawn_user(
                    name,
                    TaskContext::new(frame_address as usize),
                    UserTaskMetadata::new_with_capabilities(
                        process.root_address().unwrap_or_else(|_| {
                            init_runtime_failure(
                                b"P7.1: process address-space root is unavailable.\r\n",
                            )
                        }),
                        process.user_stack_pointer(),
                        stack_top,
                        fx_pointer as usize,
                        capabilities,
                    ),
                )
                .unwrap_or_else(|_| {
                    init_runtime_failure(b"P7.1: could not register user process.\r\n")
                })
        };
        if !register_process_record(
            slot,
            task_id,
            process_slot,
            stack_bottom,
            stack_top,
            fx_pointer as usize,
        ) {
            init_runtime_failure(b"P7.1: could not register process architecture resources.\r\n");
        }
        if slot == 0 {
            first_task_id = Some(task_id);
        }
    }

    let Some((reaper_stack_bottom, reaper_stack_top)) = reaper_kernel_stack_bounds() else {
        init_runtime_failure(b"P7.1: reaper kernel stack bounds are invalid.\r\n");
    };
    if address_space::restore_kernel().is_err() || user::clear_active_kernel_stack().is_err() {
        init_runtime_failure(b"P7.1: could not restore kernel mappings before reaper setup.\r\n");
    }
    let reaper_stack_storage =
        unsafe { ptr::addr_of_mut!(REAPER_KERNEL_STACK.0).cast::<u8>() as u64 };
    if !install_runtime_stack_guard(
        reaper_stack_storage,
        (reaper_stack_bottom, reaper_stack_top),
    ) {
        init_runtime_failure(b"P7.1: could not install the reaper kernel-stack guard.\r\n");
    }

    let reaper_fx_pointer = ptr::addr_of_mut!(REAPER_FX_STATE);
    if unsafe { (*reaper_fx_pointer).initialize() }.is_err() {
        init_runtime_failure(b"P7.1: could not initialize the reaper FXSAVE state.\r\n");
    }

    let reaper_frame = build_reaper_frame(reaper_stack_top);
    let reaper_task = unsafe {
        (*scheduler)
            .spawn("kernel-reaper", TaskContext::new(reaper_frame as usize))
            .unwrap_or_else(|_| {
                init_runtime_failure(b"P7.1: could not register kernel reaper.\r\n")
            })
    };
    if unsafe { (*scheduler).block_ready(reaper_task) }.is_err() {
        init_runtime_failure(b"P7.1: could not block the idle kernel reaper.\r\n");
    }

    let first_task_id = first_task_id.unwrap_or_else(|| {
        init_runtime_failure(b"P7.1: first user process task id is unavailable.\r\n")
    });
    if unsafe { (*scheduler).start() } != Ok(first_task_id) {
        init_runtime_failure(b"P7.1: scheduler did not start the first user process.\r\n");
    }
    unsafe {
        ptr::write(ptr::addr_of_mut!(REAPER_TASK_ID), Some(reaper_task));
    }

    let first_record = process_record_for_task(first_task_id).unwrap_or_else(|| {
        init_runtime_failure(b"P7.1: first process resources are not registered.\r\n")
    });
    if !syscall::initialize_cpu_local(
        first_record.kernel_stack_top,
        first_record.fx_state,
        exhi_init_syscall_dispatch as *const () as usize,
    ) {
        init_runtime_failure(b"P7.1: could not initialize process syscall CPU-local state.\r\n");
    }
    if syscall::initialize(syscall::entry_address()).is_err() {
        init_runtime_failure(b"P7.1: could not initialize process syscall MSRs.\r\n");
    }

    let first_process = unsafe { (*first_record.process).as_ref() }
        .unwrap_or_else(|| init_runtime_failure(b"P7.1: first process object is missing.\\r\n"));
    if first_process.activate().is_err() {
        init_runtime_failure(b"P7.1: could not activate the first process address space.\\r\n");
    }
    let first_space = first_process.address_space().unwrap_or_else(|_| {
        init_runtime_failure(b"P7.1: first process address space is unavailable.\\r\n")
    });
    if user::activate_kernel_stack(
        first_space,
        first_record.kernel_stack_bottom,
        first_record.kernel_stack_top,
    )
    .is_err()
    {
        init_runtime_failure(b"P7.1: could not activate the first process kernel stack.\\r\n");
    }
    #[cfg(feature = "process-stack-guard-smoke-test")]
    {
        if !verify_all_process_kernel_stack_guards() {
            init_runtime_failure(
                b"P2.6: init, worker or reaper kernel-stack guard verification failed.\r\n",
            );
        }
        let Some((worker_bottom, _)) = process_kernel_stack_bounds(1) else {
            init_runtime_failure(b"P2.6: worker kernel-stack bounds are unavailable.\r\n");
        };
        let Some(worker_guard) = worker_bottom.checked_sub(PAGE_SIZE) else {
            init_runtime_failure(b"P2.6: worker guard-page address underflowed.\r\n");
        };
        serial::COM1.write(
            b"P2.6: init, worker and reaper stacks have 64 KiB supervisor mappings and unmapped guards.\r\n",
        );
        serial::COM1.write(b"P2.6: forcing #PF/#DF from the worker kernel-stack guard.\r\n");
        idt::trigger_process_stack_guard_smoke_test(worker_bottom, worker_guard);
    }

    #[cfg(not(feature = "process-stack-guard-smoke-test"))]
    {
    let first_frame_address = unsafe {
        (*scheduler)
            .get(first_task_id)
            .map(|task| task.context().saved_stack_pointer())
    }
    .unwrap_or_else(|| init_runtime_failure(b"P7.1: first process entry frame is missing.\\r\n"));
    if first_frame_address == 0
        || first_frame_address & 0xF != 0
        || (first_frame_address as u64) < first_record.kernel_stack_bottom
        || (first_frame_address as u64)
            .checked_add(size_of::<user::UserEntryFrame>() as u64)
            .is_none_or(|end| end > first_record.kernel_stack_top)
    {
        init_runtime_failure(b"P7.1: first process entry frame is outside its kernel stack.\\r\n");
    }
    let frame = unsafe { &*(first_frame_address as *const user::UserEntryFrame) };
    let fx_state = unsafe { &*(first_record.fx_state as *const user::FxState) };
    match user::enter_user(frame, fx_state) {
        Ok(()) => {
            init_runtime_failure(b"P7.1: first user process IRETQ entry unexpectedly returned.\r\n")
        }
        Err(_) => init_runtime_failure(b"P7.1: first user process IRETQ handoff failed.\r\n"),
    }    }

}

/// The kernel reaper always runs after the user task has switched away from
/// its private kernel stack. Only here is it safe to reclaim the process frames
/// and address-space page tables owned by every finished user process.
#[cfg(feature = "process-runtime")]
#[no_mangle]
extern "sysv64" fn exhi_process_reaper_main() -> ! {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }
    serial::COM1.write(b"ExhiOS: kernel reaper started on its dedicated stack.\r\n");

    let scheduler = ptr::addr_of_mut!(PROCESS_SCHEDULER);
    let registry = ptr::addr_of!(PROCESS_REGISTRY);
    let mut cleanup_ok = true;
    let mut task_reaped = true;
    let mut reaped_count = 0usize;
    for slot in 0..MAX_PROCESS_RECORDS {
        let record = unsafe { ptr::read_volatile((*registry).as_ptr().add(slot)) };
        let Some(record) = record else {
            continue;
        };
        let finished = unsafe {
            (*scheduler)
                .get(record.task_id)
                .is_some_and(|task| task.state() == TaskState::Finished)
        };
        if !finished {
            continue;
        }

        let destroyed = unsafe {
            match (*record.process).as_mut() {
                Some(process) => process.destroy().is_ok(),
                None => false,
            }
        };
        if !destroyed {
            cleanup_ok = false;
            serial::COM1
                .write(b"ExhiOS: process resource reclamation failed; process retained.\r\n");
            continue;
        }

        unsafe {
            ptr::write(record.process, None);
        }
        if unsafe { (*scheduler).reap(record.task_id).is_ok() } {
            let _ = remove_process_record(record.task_id);
            reaped_count += 1;
        } else {
            task_reaped = false;
            serial::COM1
                .write(b"ExhiOS: process resources reclaimed but task record reaping failed.\r\n");
        }
    }
    if cleanup_ok && task_reaped && reaped_count != 0 {
        serial::COM1.write(
            b"ExhiOS: finished process address spaces, user frames and task records reclaimed.\r\n",
        );
    }
    let _task_reaped = cleanup_ok && task_reaped;
    #[cfg(any(
        feature = "process-run-smoke-test",
        feature = "process-shell-smoke-test",
        feature = "process-fault-smoke-test",
        feature = "process-multi-smoke-test",
        feature = "process-worker-fault-smoke-test"
    ))]
    let task_reaped = _task_reaped;

    #[cfg(feature = "process-run-smoke-test")]
    {
        let yields = unsafe { ptr::read_volatile(ptr::addr_of!(INIT_YIELD_COUNT)) };
        let syscalls = unsafe { ptr::read_volatile(ptr::addr_of!(INIT_SYSCALL_COUNT)) };
        let success = cleanup_ok
            && task_reaped
            && yields == INIT_YIELD_EXPECTED
            && syscalls == INIT_SYSCALL_EXPECTED;
        if success {
            serial::COM1.write(
                b"P7.2: init executed ConsoleWrite, ConsoleRead, three Yield calls and Exit through libexhi; kernel reaper reclaimed the process.\r\n",
            );
        } else {
            serial::COM1.write(
                b"P7.2: syscall sequence or kernel reaper cleanup did not match expectations.\r\n",
            );
        }
        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") crate::QEMU_DEBUG_EXIT_PORT,
                in("eax") if success { crate::QEMU_DEBUG_EXIT_VALUE } else { crate::QEMU_DEBUG_EXIT_VALUE + 1 },
                options(nomem, nostack, preserves_flags)
            );
        }
    }

    #[cfg(feature = "process-shell-smoke-test")]
    {
        let registry_empty = unsafe { (*registry).iter().all(Option::is_none) };
        let success = cleanup_ok && task_reaped && reaped_count == 2 && registry_empty;
        if success {
            serial::COM1.write(b"P7.1: shell-spawned worker and init were both reaped.\r\n");
            serial::COM1.write(b"P7.3: interactive serial shell smoke test passed.\r\n");
        } else {
            serial::COM1.write(b"P7.1: shell-spawned worker cleanup verification failed.\r\n");
        }
        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") crate::QEMU_DEBUG_EXIT_PORT,
                in("eax") if success { crate::QEMU_DEBUG_EXIT_VALUE } else { crate::QEMU_DEBUG_EXIT_VALUE + 1 },
                options(nomem, nostack, preserves_flags)
            );
        }
    }

    #[cfg(feature = "process-multi-smoke-test")]
    {
        let switches = unsafe { ptr::read_volatile(ptr::addr_of!(PROCESS_MULTI_SWITCH_COUNT)) };
        let registry = ptr::addr_of!(PROCESS_REGISTRY);
        let registry_empty = unsafe { (*registry).iter().all(Option::is_none) };
        let success =
            cleanup_ok && task_reaped && reaped_count == 2 && switches >= 2 && registry_empty;
        if success {
            serial::COM1.write(
                b"P7.1: two production Ring 3 processes yielded to each other, exited and were reaped.\r\n",
            );
        } else {
            serial::COM1
                .write(b"P7.1: production multi-process lifecycle verification failed.\r\n");
        }
        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") crate::QEMU_DEBUG_EXIT_PORT,
                in("eax") if success { crate::QEMU_DEBUG_EXIT_VALUE } else { crate::QEMU_DEBUG_EXIT_VALUE + 1 },
                options(nomem, nostack, preserves_flags)
            );
        }
    }

    #[cfg(feature = "process-fault-smoke-test")]
    {
        let vector = unsafe { ptr::read_volatile(ptr::addr_of!(INIT_FAULT_VECTOR)) };
        let expected_fault = vector == 6 && cleanup_ok && task_reaped;
        if expected_fault {
            serial::COM1.write(
                b"P7.1: Ring 3 invalid-opcode fault captured; reaper reclaimed init and task record.\r\n",
            );
        } else {
            serial::COM1
                .write(b"P7.1: user-fault termination or reaper cleanup verification failed.\r\n");
        }
        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") crate::QEMU_DEBUG_EXIT_PORT,
                in("eax") if expected_fault { crate::QEMU_DEBUG_EXIT_VALUE } else { crate::QEMU_DEBUG_EXIT_VALUE + 1 },
                options(nomem, nostack, preserves_flags)
            );
        }
    }

    #[cfg(feature = "process-worker-fault-smoke-test")]
    {
        let fault_vector =
            unsafe { ptr::read_volatile(ptr::addr_of!(PROCESS_WORKER_FAULT_VECTOR)) };
        let registry_empty = unsafe { (*registry).iter().all(Option::is_none) };
        let success =
            fault_vector == 6 && cleanup_ok && task_reaped && reaped_count == 2 && registry_empty;
        if success {
            serial::COM1.write(
                b"P7.1: worker fault returned to init; reaper reclaimed both processes.\r\n",
            );
        } else {
            serial::COM1.write(
                b"P7.1: worker fault recovery or two-process reaper verification failed.\r\n",
            );
        }
        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") crate::QEMU_DEBUG_EXIT_PORT,
                in("eax") if success { crate::QEMU_DEBUG_EXIT_VALUE } else { crate::QEMU_DEBUG_EXIT_VALUE + 1 },
                options(nomem, nostack, preserves_flags)
            );
        }
    }

    loop {
        unsafe {
            core::arch::asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}

/// Marks the current user task Finished and transfers execution to the next
/// scheduler context or the dedicated kernel reaper. No process-owned address
/// space or task record is reclaimed on the faulting task's kernel stack.
#[cfg(feature = "process-runtime")]
fn handoff_faulting_process_to_scheduler(vector: u64) -> ! {
    // The vector is only consumed by the dedicated worker-fault smoke mode.
    let _ = vector;
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }
    let scheduler = ptr::addr_of_mut!(PROCESS_SCHEDULER);
    let current_id = unsafe { (*scheduler).current().map(|task| task.id()) }.unwrap_or_else(|| {
        init_runtime_failure(b"P7.1: user fault arrived without a current task.\r\n")
    });
    if process_record_for_task(current_id).is_none() {
        init_runtime_failure(b"P7.1: user faulting task has no process record.\r\n");
    }
    #[cfg(feature = "process-worker-fault-smoke-test")]
    {
        let current_name = unsafe { (*scheduler).get(current_id).map(|task| task.name()) };
        if current_name == Some("worker") {
            unsafe {
                ptr::write_volatile(ptr::addr_of_mut!(PROCESS_WORKER_FAULT_VECTOR), vector);
            }
        }
    }
    let reaper_id =
        unsafe { ptr::read_volatile(ptr::addr_of!(REAPER_TASK_ID)) }.unwrap_or_else(|| {
            init_runtime_failure(b"P7.1: reaper task id is unavailable during fault handoff.\r\n")
        });

    let next = unsafe { (*scheduler).finish_current() }.unwrap_or_else(|_| {
        init_runtime_failure(b"P7.1: could not finish faulting user process.\r\n")
    });
    let selected = if let Some(next) = next {
        next
    } else {
        unsafe {
            (*scheduler).wake(reaper_id).unwrap_or_else(|_| {
                init_runtime_failure(b"P7.1: could not wake kernel reaper after user fault.\r\n")
            });
        }
        unsafe { (*scheduler).schedule_from_idle() }
            .unwrap_or_else(|_| {
                init_runtime_failure(b"P7.1: could not schedule after user fault.\r\n")
            })
            .unwrap_or_else(|| {
                init_runtime_failure(b"P7.1: scheduler selected no task after user fault.\r\n")
            })
    };

    if selected == reaper_id {
        handoff_to_kernel_reaper();
    }
    let frame = select_process_context(selected);
    let frame = unsafe { &*(frame as *const user::UserEntryFrame) };
    let fx_state = if selected == reaper_id {
        ptr::addr_of!(REAPER_FX_STATE) as usize
    } else {
        process_record_for_task(selected)
            .map(|record| record.fx_state)
            .unwrap_or_else(|| {
                init_runtime_failure(b"P7.1: selected process has no FXSAVE state.\r\n")
            })
    };
    let fx_state = unsafe { &*(fx_state as *const user::FxState) };

    // The fault handler entered kernel GS with SWAPGS. When another Ring 3
    // process is selected, restore the user GS layout before IRETQ so its next
    // SYSCALL swaps back to the trusted kernel-local area.
    unsafe {
        core::arch::asm!("swapgs", options(nomem, nostack, preserves_flags));
    }
    let entry = user::enter_user(frame, fx_state);
    // enter_user transfers control permanently on success. If validation
    // returns an error (or the transfer unexpectedly returns), restore kernel
    // GS before reporting the fatal handoff failure.
    unsafe {
        core::arch::asm!("swapgs", options(nomem, nostack, preserves_flags));
    }
    match entry {
        Ok(()) => init_runtime_failure(b"P7.1: scheduler fault handoff unexpectedly returned.\r\n"),
        Err(_) => init_runtime_failure(b"P7.1: scheduler fault handoff validation failed.\r\n"),
    }
}

#[cfg(feature = "process-runtime")]
pub fn terminate_user_after_user_exception(vector: u64, exception: user::UserException) -> ! {
    // IDT entry from CPL3 does not execute SWAPGS automatically. Unlike the
    // SYSCALL path, this fault path must establish the trusted kernel GS base
    // before updating the scheduler-selected syscall entry context.
    unsafe {
        core::arch::asm!("swapgs", options(nomem, nostack, preserves_flags));
    }
    serial::COM1.write(b"ExhiOS: terminating user process after exception; RIP=0x");
    serial::COM1.write_hex(exception.instruction_pointer);
    serial::COM1.write(b" user RSP=0x");
    serial::COM1.write_hex(exception.user_stack_pointer);
    serial::COM1.write(b".\r\n");
    #[cfg(feature = "process-fault-smoke-test")]
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(INIT_FAULT_VECTOR), vector);
    }
    handoff_faulting_process_to_scheduler(vector);
}

#[cfg(feature = "process-runtime")]
fn init_runtime_failure(message: &[u8]) -> ! {
    serial::COM1.write(message);
    #[cfg(any(
        feature = "process-run-smoke-test",
        feature = "process-shell-smoke-test",
        feature = "process-fault-smoke-test",
        feature = "process-multi-smoke-test",
        feature = "process-worker-fault-smoke-test"
    ))]
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") crate::QEMU_DEBUG_EXIT_PORT,
            in("eax") (crate::QEMU_DEBUG_EXIT_VALUE + 1),
            options(nomem, nostack, preserves_flags)
        );
    }
    loop {
        unsafe {
            core::arch::asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}

#[cfg(feature = "process-runtime")]
fn init_runtime_failure_fmt(_error: ProcessLoadError) -> ! {
    init_runtime_failure(b"P7.2: init process load failed.\r\n")
}
