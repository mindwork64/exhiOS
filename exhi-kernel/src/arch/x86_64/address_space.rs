use core::arch::asm;
use core::ptr;

use crate::memory;

const PAGE_SIZE: u64 = 4096;
const PAGE_ENTRIES: usize = 512;
const TWO_MIB: u64 = 0x20_0000;
const ONE_GIB: u64 = 0x4000_0000;
const PHYSICAL_LIMIT: u64 = 0x1_0000_0000;
const VIRTUAL_LIMIT: u64 = 0x0000_0080_0000_0000;
const USER_VIRTUAL_BASE: u64 = 0x0000_0002_0000_0000;
const USER_FIRST_PDPT_INDEX: usize = (USER_VIRTUAL_BASE / ONE_GIB) as usize;
const SHARED_PDPT_ENTRIES: usize = 5;

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const PAGE_SIZE_BIT: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const PAGE_TABLE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const LARGE_PAGE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFE0_0000;
const RFLAGS_IF: u64 = 1 << 9;

static mut KERNEL_ROOT: u64 = 0;
static mut ADDRESS_SPACES_INITIALIZED: bool = false;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressSpaceError {
    NotInitialized,
    AlreadyInitialized,
    InterruptsEnabled,
    OutOfPhysicalFrames,
    InvalidPhysicalFrame,
    KernelMappingMissing,
    UnsupportedKernelPageTableLayout,
    AddressOutOfRange,
    UnalignedAddress,
    AlreadyMapped,
    NotMapped,
    MissingPageTableEntry,
    UnexpectedLargePage,
    AddressSpaceActive,
    Cr3Mismatch,
    FrameReleaseFailed,
    MappingVerificationFailed,
}

impl AddressSpaceError {
    pub const fn message(self) -> &'static [u8] {
        match self {
            Self::NotInitialized => b"address-space manager is not initialized",
            Self::AlreadyInitialized => b"address-space manager is already initialized",
            Self::InterruptsEnabled => b"address-space operations require IF=0",
            Self::OutOfPhysicalFrames => b"physical allocator cannot provide an address-space table",
            Self::InvalidPhysicalFrame => b"physical allocator returned an invalid page-table frame",
            Self::KernelMappingMissing => b"required shared kernel mapping is missing",
            Self::UnsupportedKernelPageTableLayout => b"kernel page tables do not match the current sharing policy",
            Self::AddressOutOfRange => b"user virtual address is outside the reserved address-space window",
            Self::UnalignedAddress => b"mapping address is not 4 KiB aligned",
            Self::AlreadyMapped => b"user virtual page is already mapped",
            Self::NotMapped => b"user virtual page is not mapped",
            Self::MissingPageTableEntry => b"required upper-level page-table entry is missing",
            Self::UnexpectedLargePage => b"unexpected huge page in a private user page-table subtree",
            Self::AddressSpaceActive => b"cannot destroy the currently active address space",
            Self::Cr3Mismatch => b"CR3 did not switch to the requested address-space root",
            Self::FrameReleaseFailed => b"page-table frame could not be returned to the physical allocator",
            Self::MappingVerificationFailed => b"address-space mapping verification failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserPageFlags {
    pub writable: bool,
    pub executable: bool,
}

impl UserPageFlags {
    pub const fn read_write_nx() -> Self {
        Self {
            writable: true,
            executable: false,
        }
    }

    pub const fn read_only_nx() -> Self {
        Self {
            writable: false,
            executable: false,
        }
    }

    pub const fn read_execute() -> Self {
        Self {
            writable: false,
            executable: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageMapping {
    pub virtual_page: u64,
    pub physical_page: u64,
    pub writable: bool,
    pub user: bool,
    pub executable: bool,
}

pub struct AddressSpace {
    root_frame: u64,
    pdpt_frame: u64,
}

/// Captures the active kernel root after the VMM and kernel heap are ready.
///
/// The first five PDPT entries are the shared kernel region: 0..4 GiB physical
/// identity map and the heap window beginning at 4 GiB. PDPT entries 5..7 stay
/// reserved. Each new address space receives a private PDPT and uses entries
/// 8..511 for private user mappings.
pub fn initialize() -> Result<u64, AddressSpaceError> {
    ensure_interrupts_disabled()?;
    if unsafe { ADDRESS_SPACES_INITIALIZED } {
        return Err(AddressSpaceError::AlreadyInitialized);
    }

    let kernel_root = read_cr3() & PAGE_TABLE_ADDRESS_MASK;
    if kernel_root == 0 || kernel_root >= PHYSICAL_LIMIT || kernel_root % PAGE_SIZE != 0 {
        return Err(AddressSpaceError::InvalidPhysicalFrame);
    }

    let pml4_entry = unsafe { read_entry(kernel_root, 0) };
    if pml4_entry & PRESENT == 0 || pml4_entry & PAGE_SIZE_BIT != 0 {
        return Err(AddressSpaceError::KernelMappingMissing);
    }

    for index in 1..PAGE_ENTRIES {
        if unsafe { read_entry(kernel_root, index) } & PRESENT != 0 {
            return Err(AddressSpaceError::UnsupportedKernelPageTableLayout);
        }
    }

    let kernel_pdpt = pml4_entry & PAGE_TABLE_ADDRESS_MASK;
    for index in 0..SHARED_PDPT_ENTRIES {
        if unsafe { read_entry(kernel_pdpt, index) } & PRESENT == 0 {
            return Err(AddressSpaceError::KernelMappingMissing);
        }
    }

    for index in SHARED_PDPT_ENTRIES..USER_FIRST_PDPT_INDEX {
        if unsafe { read_entry(kernel_pdpt, index) } & PRESENT != 0 {
            return Err(AddressSpaceError::UnsupportedKernelPageTableLayout);
        }
    }

    for index in USER_FIRST_PDPT_INDEX..PAGE_ENTRIES {
        if unsafe { read_entry(kernel_pdpt, index) } & PRESENT != 0 {
            return Err(AddressSpaceError::UnsupportedKernelPageTableLayout);
        }
    }

    let kernel_root_ptr = core::ptr::addr_of_mut!(KERNEL_ROOT);
    let initialized_ptr = core::ptr::addr_of_mut!(ADDRESS_SPACES_INITIALIZED);
    // SAFETY: Called once during single-core startup, before address spaces exist.
    unsafe {
        *kernel_root_ptr = kernel_root;
        *initialized_ptr = true;
    }

    Ok(kernel_root)
}

impl AddressSpace {
    /// Creates a private PML4/PDPT pair and shares only the kernel's first five
    /// PDPT entries. No user mappings are inherited or shared.
    pub fn create() -> Result<Self, AddressSpaceError> {
        ensure_initialized()?;
        ensure_interrupts_disabled()?;

        let kernel_root = kernel_root()?;
        let kernel_pml4_entry = unsafe { read_entry(kernel_root, 0) };
        let kernel_pdpt = kernel_pml4_entry & PAGE_TABLE_ADDRESS_MASK;

        let root_frame = allocate_zeroed_table()?;
        let pdpt_frame = match allocate_zeroed_table() {
            Ok(frame) => frame,
            Err(error) => {
                release_table(root_frame)?;
                return Err(error);
            }
        };

        for index in 0..SHARED_PDPT_ENTRIES {
            let entry = unsafe { read_entry(kernel_pdpt, index) };
            write_entry(pdpt_frame, index, entry);
        }

        let root_flags = (kernel_pml4_entry & !PAGE_TABLE_ADDRESS_MASK) | PRESENT | WRITABLE;
        write_entry(root_frame, 0, pdpt_frame | root_flags);

        Ok(Self {
            root_frame,
            pdpt_frame,
        })
    }

    pub fn root_address(&self) -> u64 {
        self.root_frame
    }

    /// Maps a caller-owned physical frame into this address space's private user range.
    ///
    /// # Safety
    /// The caller must own the frame, keep it allocated until unmapping, and
    /// initialize/scrub its contents before exposing it to untrusted user code.
    pub unsafe fn map_user_page(
        &mut self,
        virtual_address: u64,
        physical_address: u64,
        flags: UserPageFlags,
    ) -> Result<(), AddressSpaceError> {
        ensure_initialized()?;
        ensure_interrupts_disabled()?;
        validate_user_virtual_address(virtual_address)?;
        validate_physical_address(physical_address)?;

        let pml4_entry = unsafe { read_entry(self.root_frame, 0) };
        if pml4_entry & PRESENT == 0 {
            return Err(AddressSpaceError::KernelMappingMissing);
        }

        let pdpt_index = ((virtual_address >> 30) & 0x1FF) as usize;
        let directory_index = ((virtual_address >> 21) & 0x1FF) as usize;
        let page_index = ((virtual_address >> 12) & 0x1FF) as usize;

        let mut pdpt_entry = unsafe { read_entry(self.pdpt_frame, pdpt_index) };
        let mut allocated_directory = false;
        if pdpt_entry & PRESENT == 0 {
            let directory_frame = allocate_zeroed_table()?;
            pdpt_entry = directory_frame | PRESENT | WRITABLE | USER;
            write_entry(self.pdpt_frame, pdpt_index, pdpt_entry);
            allocated_directory = true;
        }
        if pdpt_entry & PAGE_SIZE_BIT != 0 {
            return Err(AddressSpaceError::UnexpectedLargePage);
        }

        let directory_frame = pdpt_entry & PAGE_TABLE_ADDRESS_MASK;
        let mut directory_entry = unsafe { read_entry(directory_frame, directory_index) };
        let mut allocated_table = false;
        if directory_entry & PRESENT == 0 {
            let table_frame = match allocate_zeroed_table() {
                Ok(frame) => frame,
                Err(error) => {
                    if allocated_directory {
                        write_entry(self.pdpt_frame, pdpt_index, 0);
                        release_table(directory_frame)?;
                    }
                    return Err(error);
                }
            };
            directory_entry = table_frame | PRESENT | WRITABLE | USER;
            write_entry(directory_frame, directory_index, directory_entry);
            allocated_table = true;
        } else if directory_entry & PAGE_SIZE_BIT != 0 {
            return Err(AddressSpaceError::UnexpectedLargePage);
        }

        let table_frame = directory_entry & PAGE_TABLE_ADDRESS_MASK;
        let existing = unsafe { read_entry(table_frame, page_index) };
        if existing & PRESENT != 0 {
            if allocated_table {
                write_entry(directory_frame, directory_index, 0);
                release_table(table_frame)?;
            }
            if allocated_directory {
                write_entry(self.pdpt_frame, pdpt_index, 0);
                release_table(directory_frame)?;
            }
            return Err(AddressSpaceError::AlreadyMapped);
        }

        let mut leaf = physical_address | PRESENT | USER;
        if flags.writable {
            leaf |= WRITABLE;
        }
        if !flags.executable {
            leaf |= NO_EXECUTE;
        }

        write_entry(table_frame, page_index, leaf);
        write_entry(self.pdpt_frame, pdpt_index, pdpt_entry | USER | WRITABLE);
        write_entry(self.root_frame, 0, pml4_entry | USER | WRITABLE);

        if is_active(self.root_frame) {
            invalidate_page(virtual_address);
        }

        Ok(())
    }

    /// Removes a user mapping and reclaims empty page-table levels.
    /// The returned physical frame remains owned by the caller.
    pub fn unmap_user_page(
        &mut self,
        virtual_address: u64,
    ) -> Result<u64, AddressSpaceError> {
        ensure_initialized()?;
        ensure_interrupts_disabled()?;
        validate_user_virtual_address(virtual_address)?;

        let pdpt_index = ((virtual_address >> 30) & 0x1FF) as usize;
        let directory_index = ((virtual_address >> 21) & 0x1FF) as usize;
        let page_index = ((virtual_address >> 12) & 0x1FF) as usize;

        let pdpt_entry = unsafe { read_entry(self.pdpt_frame, pdpt_index) };
        if pdpt_entry & PRESENT == 0 {
            return Err(AddressSpaceError::NotMapped);
        }
        if pdpt_entry & PAGE_SIZE_BIT != 0 {
            return Err(AddressSpaceError::UnexpectedLargePage);
        }

        let directory_frame = pdpt_entry & PAGE_TABLE_ADDRESS_MASK;
        let directory_entry = unsafe { read_entry(directory_frame, directory_index) };
        if directory_entry & PRESENT == 0 {
            return Err(AddressSpaceError::NotMapped);
        }
        if directory_entry & PAGE_SIZE_BIT != 0 {
            return Err(AddressSpaceError::UnexpectedLargePage);
        }

        let table_frame = directory_entry & PAGE_TABLE_ADDRESS_MASK;
        let leaf = unsafe { read_entry(table_frame, page_index) };
        if leaf & PRESENT == 0 {
            return Err(AddressSpaceError::NotMapped);
        }

        write_entry(table_frame, page_index, 0);
        if is_active(self.root_frame) {
            invalidate_page(virtual_address);
        }

        if table_is_empty(table_frame) {
            write_entry(directory_frame, directory_index, 0);
            release_table(table_frame)?;
        }

        if table_is_empty(directory_frame) {
            write_entry(self.pdpt_frame, pdpt_index, 0);
            release_table(directory_frame)?;
        }

        if private_pdpt_is_empty(self.pdpt_frame) {
            let pml4_entry = unsafe { read_entry(self.root_frame, 0) };
            write_entry(self.root_frame, 0, pml4_entry & !USER);
        }

        Ok(leaf & PAGE_TABLE_ADDRESS_MASK)
    }

    /// Software page-table walk, usable whether this address space is active
    /// or inactive. The returned flags are the effective permissions.
    pub fn translate(&self, virtual_address: u64) -> Result<PageMapping, AddressSpaceError> {
        ensure_initialized()?;
        translate_from_root(self.root_frame, virtual_address)
    }

    /// Switches CR3 to this address space. The caller must restore the kernel
    /// root before destroying the space or returning to code without shared maps.
    pub fn activate(&self) -> Result<(), AddressSpaceError> {
        ensure_initialized()?;
        ensure_interrupts_disabled()?;
        if self.root_frame == 0 || self.root_frame % PAGE_SIZE != 0 {
            return Err(AddressSpaceError::InvalidPhysicalFrame);
        }

        unsafe {
            asm!(
                "mov cr3, {}",
                in(reg) self.root_frame,
                options(nostack, preserves_flags)
            );
        }
        if read_cr3() & PAGE_TABLE_ADDRESS_MASK != self.root_frame {
            return Err(AddressSpaceError::Cr3Mismatch);
        }
        Ok(())
    }

    /// Releases private user page tables, PDPT and PML4. Mapped data frames
    /// are not freed; ownership stays with the caller.
    pub fn destroy(&mut self) -> Result<(), AddressSpaceError> {
        ensure_initialized()?;
        ensure_interrupts_disabled()?;
        if self.root_frame == 0 || self.pdpt_frame == 0 {
            return Ok(());
        }
        if is_active(self.root_frame) {
            return Err(AddressSpaceError::AddressSpaceActive);
        }

        for pdpt_index in USER_FIRST_PDPT_INDEX..PAGE_ENTRIES {
            let pdpt_entry = unsafe { read_entry(self.pdpt_frame, pdpt_index) };
            if pdpt_entry & PRESENT == 0 {
                continue;
            }
            if pdpt_entry & PAGE_SIZE_BIT != 0 {
                return Err(AddressSpaceError::UnexpectedLargePage);
            }

            let directory_frame = pdpt_entry & PAGE_TABLE_ADDRESS_MASK;
            for directory_index in 0..PAGE_ENTRIES {
                let directory_entry = unsafe { read_entry(directory_frame, directory_index) };
                if directory_entry & PRESENT == 0 {
                    continue;
                }
                if directory_entry & PAGE_SIZE_BIT != 0 {
                    return Err(AddressSpaceError::UnexpectedLargePage);
                }

                let table_frame = directory_entry & PAGE_TABLE_ADDRESS_MASK;
                release_table(table_frame)?;
            }
            release_table(directory_frame)?;
        }

        release_table(self.pdpt_frame)?;
        release_table(self.root_frame)?;
        self.pdpt_frame = 0;
        self.root_frame = 0;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressSpaceTestStatus {
    pub kernel_root: u64,
    pub first_root: u64,
    pub second_root: u64,
    pub user_virtual_page: u64,
    pub first_physical_page: u64,
    pub second_physical_page: u64,
    pub shared_kernel_translation: u64,
    pub shared_heap_translation: u64,
}

/// Creates two independent roots, maps the same user VA to different frames,
/// switches between roots while executing kernel code, then unmaps and frees
/// all private structures.
pub fn smoke_test() -> Result<AddressSpaceTestStatus, AddressSpaceError> {
    ensure_initialized()?;
    ensure_interrupts_disabled()?;

    let kernel_root = kernel_root()?;
    let mut first = AddressSpace::create()?;
    let mut second = match AddressSpace::create() {
        Ok(space) => space,
        Err(error) => {
            first.destroy()?;
            return Err(error);
        }
    };

    let first_frame = match memory::allocate_frame() {
        Ok(frame) if frame != 0 && frame < PHYSICAL_LIMIT && frame % PAGE_SIZE == 0 => frame,
        Ok(frame) => {
            let _ = memory::release_frame(frame);
            let _ = first.destroy();
            let _ = second.destroy();
            return Err(AddressSpaceError::InvalidPhysicalFrame);
        }
        Err(_) => {
            let _ = first.destroy();
            let _ = second.destroy();
            return Err(AddressSpaceError::OutOfPhysicalFrames);
        }
    };
    let second_frame = match memory::allocate_frame() {
        Ok(frame) if frame != 0 && frame < PHYSICAL_LIMIT && frame % PAGE_SIZE == 0 => frame,
        Ok(frame) => {
            let _ = memory::release_frame(frame);
            let _ = memory::release_frame(first_frame);
            let _ = first.destroy();
            let _ = second.destroy();
            return Err(AddressSpaceError::InvalidPhysicalFrame);
        }
        Err(_) => {
            let _ = memory::release_frame(first_frame);
            let _ = first.destroy();
            let _ = second.destroy();
            return Err(AddressSpaceError::OutOfPhysicalFrames);
        }
    };

    // SAFETY: first_frame was freshly allocated and remains owned by this test.
    if let Err(error) = unsafe { first.map_user_page(USER_VIRTUAL_BASE, first_frame, UserPageFlags::read_write_nx()) } {
        let _ = memory::release_frame(first_frame);
        let _ = memory::release_frame(second_frame);
        let _ = first.destroy();
        let _ = second.destroy();
        return Err(error);
    }
    // SAFETY: second_frame was freshly allocated and remains owned by this test.
    if let Err(error) = unsafe { second.map_user_page(USER_VIRTUAL_BASE, second_frame, UserPageFlags::read_write_nx()) } {
        let _ = first.unmap_user_page(USER_VIRTUAL_BASE);
        let _ = memory::release_frame(first_frame);
        let _ = memory::release_frame(second_frame);
        let _ = first.destroy();
        let _ = second.destroy();
        return Err(error);
    }

    let first_mapping = first.translate(USER_VIRTUAL_BASE)?;
    let second_mapping = second.translate(USER_VIRTUAL_BASE)?;
    if first_mapping.physical_page != first_frame
        || second_mapping.physical_page != second_frame
        || first_mapping.physical_page == second_mapping.physical_page
        || !first_mapping.user
        || !second_mapping.user
        || !first_mapping.writable
        || !second_mapping.writable
        || first_mapping.executable
        || second_mapping.executable
    {
        return Err(AddressSpaceError::MappingVerificationFailed);
    }

    let kernel_entry = crate::arch::x86_64::address_space::smoke_test as *const () as u64;
    let stack_pointer = read_stack_pointer();
    const HEAP_PROBE: u64 = 0x1_0000_0000;
    let kernel_translation = first.translate(kernel_entry)?;
    let stack_translation = first.translate(stack_pointer)?;
    let heap_translation = first.translate(HEAP_PROBE)?;
    if kernel_translation.physical_page != (kernel_entry & !(PAGE_SIZE - 1))
        || stack_translation.physical_page != (stack_pointer & !(PAGE_SIZE - 1))
        || heap_translation.physical_page == HEAP_PROBE
        || kernel_translation.user
        || stack_translation.user
        || heap_translation.user
    {
        return Err(AddressSpaceError::MappingVerificationFailed);
    }

    first.activate()?;
    unsafe {
        ptr::write_volatile(USER_VIRTUAL_BASE as *mut u8, 0xA1);
    }
    restore_kernel()?;

    second.activate()?;
    unsafe {
        ptr::write_volatile(USER_VIRTUAL_BASE as *mut u8, 0xB2);
    }
    restore_kernel()?;

    first.activate()?;
    let first_value = unsafe { ptr::read_volatile(USER_VIRTUAL_BASE as *const u8) };
    restore_kernel()?;

    second.activate()?;
    let second_value = unsafe { ptr::read_volatile(USER_VIRTUAL_BASE as *const u8) };
    restore_kernel()?;

    if first_value != 0xA1 || second_value != 0xB2 {
        return Err(AddressSpaceError::MappingVerificationFailed);
    }

    let first_root = first.root_address();
    let second_root = second.root_address();

    let removed_first = first.unmap_user_page(USER_VIRTUAL_BASE)?;
    let removed_second = second.unmap_user_page(USER_VIRTUAL_BASE)?;
    if removed_first != first_frame || removed_second != second_frame {
        return Err(AddressSpaceError::MappingVerificationFailed);
    }

    memory::release_frame(first_frame).map_err(|_| AddressSpaceError::FrameReleaseFailed)?;
    memory::release_frame(second_frame).map_err(|_| AddressSpaceError::FrameReleaseFailed)?;

    first.destroy()?;
    second.destroy()?;

    Ok(AddressSpaceTestStatus {
        kernel_root,
        first_root,
        second_root,
        user_virtual_page: USER_VIRTUAL_BASE,
        first_physical_page: first_frame,
        second_physical_page: second_frame,
        shared_kernel_translation: kernel_translation.physical_page,
        shared_heap_translation: heap_translation.physical_page,
    })
}

pub fn restore_kernel() -> Result<(), AddressSpaceError> {
    ensure_initialized()?;
    ensure_interrupts_disabled()?;
    let root = kernel_root()?;
    unsafe {
        asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags));
    }
    if read_cr3() & PAGE_TABLE_ADDRESS_MASK != root {
        return Err(AddressSpaceError::Cr3Mismatch);
    }
    Ok(())
}

fn translate_from_root(
    root_frame: u64,
    virtual_address: u64,
) -> Result<PageMapping, AddressSpaceError> {
    if virtual_address >= VIRTUAL_LIMIT {
        return Err(AddressSpaceError::AddressOutOfRange);
    }

    let pml4_index = ((virtual_address >> 39) & 0x1FF) as usize;
    let pdpt_index = ((virtual_address >> 30) & 0x1FF) as usize;
    let directory_index = ((virtual_address >> 21) & 0x1FF) as usize;
    let page_index = ((virtual_address >> 12) & 0x1FF) as usize;
    let pml4_entry = unsafe { read_entry(root_frame, pml4_index) };
    if pml4_entry & PRESENT == 0 {
        return Err(AddressSpaceError::MissingPageTableEntry);
    }

    let pdpt_frame = pml4_entry & PAGE_TABLE_ADDRESS_MASK;
    let pdpt_entry = unsafe { read_entry(pdpt_frame, pdpt_index) };
    if pdpt_entry & PRESENT == 0 {
        return Err(AddressSpaceError::MissingPageTableEntry);
    }
    if pdpt_entry & PAGE_SIZE_BIT != 0 {
        return Err(AddressSpaceError::UnexpectedLargePage);
    }

    let directory_frame = pdpt_entry & PAGE_TABLE_ADDRESS_MASK;
    let directory_entry = unsafe { read_entry(directory_frame, directory_index) };
    if directory_entry & PRESENT == 0 {
        return Err(AddressSpaceError::MissingPageTableEntry);
    }

    if directory_entry & PAGE_SIZE_BIT != 0 {
        let base = directory_entry & LARGE_PAGE_ADDRESS_MASK;
        return Ok(PageMapping {
            virtual_page: virtual_address & !(PAGE_SIZE - 1),
            physical_page: (base | (virtual_address & (TWO_MIB - 1))) & !(PAGE_SIZE - 1),
            writable: (pml4_entry & WRITABLE != 0)
                && (pdpt_entry & WRITABLE != 0)
                && (directory_entry & WRITABLE != 0),
            user: (pml4_entry & USER != 0)
                && (pdpt_entry & USER != 0)
                && (directory_entry & USER != 0),
            executable: (pml4_entry & NO_EXECUTE == 0)
                && (pdpt_entry & NO_EXECUTE == 0)
                && (directory_entry & NO_EXECUTE == 0),
        });
    }

    let table_frame = directory_entry & PAGE_TABLE_ADDRESS_MASK;
    let leaf = unsafe { read_entry(table_frame, page_index) };
    if leaf & PRESENT == 0 {
        return Err(AddressSpaceError::NotMapped);
    }

    let base = leaf & PAGE_TABLE_ADDRESS_MASK;
    Ok(PageMapping {
        virtual_page: virtual_address & !(PAGE_SIZE - 1),
        physical_page: base,
        writable: (pml4_entry & WRITABLE != 0)
            && (pdpt_entry & WRITABLE != 0)
            && (directory_entry & WRITABLE != 0)
            && (leaf & WRITABLE != 0),
        user: (pml4_entry & USER != 0)
            && (pdpt_entry & USER != 0)
            && (directory_entry & USER != 0)
            && (leaf & USER != 0),
        executable: (pml4_entry & NO_EXECUTE == 0)
            && (pdpt_entry & NO_EXECUTE == 0)
            && (directory_entry & NO_EXECUTE == 0)
            && (leaf & NO_EXECUTE == 0),
    })
}

fn validate_user_virtual_address(address: u64) -> Result<(), AddressSpaceError> {
    if address % PAGE_SIZE != 0 {
        return Err(AddressSpaceError::UnalignedAddress);
    }
    if !(USER_VIRTUAL_BASE..VIRTUAL_LIMIT).contains(&address) {
        return Err(AddressSpaceError::AddressOutOfRange);
    }
    Ok(())
}

fn validate_physical_address(address: u64) -> Result<(), AddressSpaceError> {
    if address % PAGE_SIZE != 0 {
        return Err(AddressSpaceError::UnalignedAddress);
    }
    if address == 0 || address >= PHYSICAL_LIMIT {
        return Err(AddressSpaceError::AddressOutOfRange);
    }
    Ok(())
}

fn allocate_zeroed_table() -> Result<u64, AddressSpaceError> {
    let frame = memory::allocate_frame().map_err(|_| AddressSpaceError::OutOfPhysicalFrames)?;
    if frame == 0 || frame >= PHYSICAL_LIMIT || frame % PAGE_SIZE != 0 {
        let _ = memory::release_frame(frame);
        return Err(AddressSpaceError::InvalidPhysicalFrame);
    }
    unsafe {
        ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
    }
    Ok(frame)
}

fn release_table(frame: u64) -> Result<(), AddressSpaceError> {
    memory::release_frame(frame).map_err(|_| AddressSpaceError::FrameReleaseFailed)
}

fn table_is_empty(frame: u64) -> bool {
    for index in 0..PAGE_ENTRIES {
        if unsafe { read_entry(frame, index) } & PRESENT != 0 {
            return false;
        }
    }
    true
}

fn private_pdpt_is_empty(pdpt_frame: u64) -> bool {
    for index in USER_FIRST_PDPT_INDEX..PAGE_ENTRIES {
        if unsafe { read_entry(pdpt_frame, index) } & PRESENT != 0 {
            return false;
        }
    }
    true
}

fn ensure_initialized() -> Result<(), AddressSpaceError> {
    if unsafe { ADDRESS_SPACES_INITIALIZED } {
        Ok(())
    } else {
        Err(AddressSpaceError::NotInitialized)
    }
}

fn ensure_interrupts_disabled() -> Result<(), AddressSpaceError> {
    if read_rflags() & RFLAGS_IF == 0 {
        Ok(())
    } else {
        Err(AddressSpaceError::InterruptsEnabled)
    }
}

fn kernel_root() -> Result<u64, AddressSpaceError> {
    let root = unsafe { KERNEL_ROOT };
    if root == 0 {
        Err(AddressSpaceError::NotInitialized)
    } else {
        Ok(root)
    }
}

fn is_active(root: u64) -> bool {
    read_cr3() & PAGE_TABLE_ADDRESS_MASK == root
}

fn write_entry(table_frame: u64, index: usize, value: u64) {
    unsafe {
        ptr::write_volatile((table_frame as *mut u64).add(index), value);
    }
}

unsafe fn read_entry(table_frame: u64, index: usize) -> u64 {
    unsafe { ptr::read_volatile((table_frame as *const u64).add(index)) }
}

fn invalidate_page(address: u64) {
    unsafe {
        asm!(
            "invlpg [{}]",
            in(reg) address,
            options(nostack, preserves_flags)
        );
    }
}

fn read_cr3() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

fn read_rflags() -> u64 {
    let value: u64;
    unsafe {
        asm!("pushfq", "pop {}", out(reg) value, options(preserves_flags));
    }
    value
}

fn read_stack_pointer() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, rsp", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}
