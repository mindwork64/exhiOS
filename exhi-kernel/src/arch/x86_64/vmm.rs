use core::arch::asm;
use core::arch::x86_64::__cpuid;
use core::ptr;

use crate::arch::x86_64::paging;
use crate::memory;

const PAGE_SIZE: u64 = 4096;
const TWO_MIB: u64 = 0x20_0000;
const IDENTITY_MAP_LIMIT: u64 = 0x1_0000_0000;
const VIRTUAL_ADDRESS_LIMIT: u64 = 0x0000_0080_0000_0000;

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const WRITE_THROUGH: u64 = 1 << 3;
const CACHE_DISABLE: u64 = 1 << 4;
const ACCESSED: u64 = 1 << 5;
const DIRTY: u64 = 1 << 6;
const PAGE_SIZE_BIT: u64 = 1 << 7;
const GLOBAL: u64 = 1 << 8;
const LARGE_PAGE_PAT: u64 = 1 << 12;
const PAGE_PAT: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;

const PAGE_TABLE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const LARGE_PAGE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFE0_0000;
const EFER_MSR: u32 = 0xC000_0080;
const EFER_NXE: u64 = 1 << 11;
const RFLAGS_IF: u64 = 1 << 9;

static mut VMM_INITIALIZED: bool = false;
static mut NX_ENABLED: bool = false;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageFlags {
    pub writable: bool,
    pub user: bool,
    pub executable: bool,
    pub write_through: bool,
    pub cache_disable: bool,
    pub global: bool,
}

impl PageFlags {
    /// Supervisor, writable, executable mapping.
    pub const fn kernel_read_write_execute() -> Self {
        Self {
            writable: true,
            user: false,
            executable: true,
            write_through: false,
            cache_disable: false,
            global: false,
        }
    }

    /// Supervisor, writable, non-executable mapping.
    pub const fn kernel_data() -> Self {
        Self {
            writable: true,
            user: false,
            executable: false,
            write_through: false,
            cache_disable: false,
            global: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageMapping {
    pub virtual_page: u64,
    pub physical_page: u64,
    pub flags: PageFlags,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmmStatus {
    pub page_size: u64,
    pub nx_enabled: bool,
    pub identity_map_limit: u64,
    pub virtual_address_limit: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmmError {
    NotInitialized,
    AlreadyInitialized,
    PagingNotReady,
    InterruptsEnabled,
    AddressOutOfRange,
    UnalignedAddress,
    AlreadyMapped,
    NotMapped,
    MissingPageTableEntry,
    UnsupportedOneGiBPage,
    OutOfPhysicalFrames,
    InvalidPhysicalFrame,
    PhysicalFrameReleaseFailed,
    NxNotSupported,
}

impl VmmError {
    pub const fn message(self) -> &'static [u8] {
        match self {
            Self::NotInitialized => b"virtual memory manager is not initialized",
            Self::AlreadyInitialized => b"virtual memory manager is already initialized",
            Self::PagingNotReady => b"bootstrap paging is not active",
            Self::InterruptsEnabled => b"interrupts must remain disabled during VMM operations",
            Self::AddressOutOfRange => b"virtual address is outside the lower 512 GiB window or physical address is outside the first 4 GiB",
            Self::UnalignedAddress => b"page mapping address is not 4 KiB aligned",
            Self::AlreadyMapped => b"virtual page is already mapped",
            Self::NotMapped => b"virtual page is not mapped",
            Self::MissingPageTableEntry => b"upper-level page-table entry is missing",
            Self::UnsupportedOneGiBPage => b"1 GiB page entries are not supported by this VMM",
            Self::OutOfPhysicalFrames => b"physical frame allocator cannot provide a page-table frame",
            Self::InvalidPhysicalFrame => b"physical allocator returned an invalid page-table frame",
            Self::PhysicalFrameReleaseFailed => b"page-table frame could not be safely returned to the allocator",
            Self::NxNotSupported => b"CPU does not support the NX page-table permission",
        }
    }
}

/// Enables the VMM after bootstrap paging has installed the kernel CR3.
///
/// Physical frames remain restricted to the bootstrap identity-mapped 0..4 GiB
/// range, while virtual mappings may use the lower canonical 512 GiB window.
/// Missing PDPT entries are created on demand. It deliberately retains page-table frames
/// after unmapping: reclaiming a table is safe only after proving that every
/// entry is empty and no CPU can still walk or cache it.
pub fn initialize() -> Result<VmmStatus, VmmError> {
    // SAFETY: Initialized once during single-core startup with IF clear.
    if unsafe { VMM_INITIALIZED } {
        return Err(VmmError::AlreadyInitialized);
    }

    if read_rflags() & RFLAGS_IF != 0 {
        return Err(VmmError::InterruptsEnabled);
    }

    if paging::translate_current(0).is_err() {
        return Err(VmmError::PagingNotReady);
    }

    let nx_enabled = enable_nx();
    // SAFETY: Startup is single-threaded and IF remains clear.
    unsafe {
        NX_ENABLED = nx_enabled;
        VMM_INITIALIZED = true;
    }

    Ok(VmmStatus {
        page_size: PAGE_SIZE,
        nx_enabled,
        identity_map_limit: IDENTITY_MAP_LIMIT,
        virtual_address_limit: VIRTUAL_ADDRESS_LIMIT,
    })
}

/// Maps one previously unmapped 4 KiB virtual page.
///
/// Physical addresses are currently restricted to the first 4 GiB because
/// that is the range covered by the bootstrap identity map. The caller owns
/// the mapped physical frame and must keep it allocated until unmapping.
pub fn map_page(
    virtual_address: u64,
    physical_address: u64,
    flags: PageFlags,
) -> Result<(), VmmError> {
    ensure_ready()?;
    ensure_interrupts_disabled()?;
    validate_virtual_page_address(virtual_address)?;
    validate_physical_page_address(physical_address)?;

    // NX is opt-in at the leaf. Do not set the reserved bit on CPUs that lack
    // NX support or have not enabled EFER.NXE.
    if !flags.executable && !unsafe { NX_ENABLED } {
        return Err(VmmError::NxNotSupported);
    }

    let path = walk_to_directory_for_mapping(virtual_address)?;
    if path.directory_entry & PRESENT != 0 && path.directory_entry & PAGE_SIZE_BIT != 0 {
        // A 2 MiB leaf already covers this address. The caller must explicitly
        // unmap the 4 KiB page first; silently replacing it would hide a map.
        return Err(VmmError::AlreadyMapped);
    }

    let table_address = if path.directory_entry & PRESENT == 0 {
        let frame = allocate_zeroed_table()?;
        write_entry(
            path.directory_address,
            path.directory_index,
            frame | PRESENT | WRITABLE,
        );
        frame
    } else {
        path.directory_entry & PAGE_TABLE_ADDRESS_MASK
    };

    let page_index = ((virtual_address >> 12) & 0x1FF) as usize;
    let existing = unsafe { read_entry(table_address, page_index) };
    if existing & PRESENT != 0 {
        return Err(VmmError::AlreadyMapped);
    }

    let mut entry = physical_address | PRESENT;
    if flags.writable {
        entry |= WRITABLE;
    }
    if flags.user {
        entry |= USER;
    }
    if flags.write_through {
        entry |= WRITE_THROUGH;
    }
    if flags.cache_disable {
        entry |= CACHE_DISABLE;
    }
    if flags.global {
        entry |= GLOBAL;
    }
    if !flags.executable {
        entry |= NO_EXECUTE;
    }

    // User access requires the USER bit at every paging level. Setting it in
    // upper levels does not expose supervisor leaves whose PTE USER bit stays 0.
    if flags.user {
        write_entry(path.root_address, path.pml4_index, path.pml4_entry | USER);
        write_entry(path.pdpt_address, path.pdpt_index, path.pdpt_entry | USER);
        let current_directory_entry =
            unsafe { read_entry(path.directory_address, path.directory_index) };
        write_entry(
            path.directory_address,
            path.directory_index,
            current_directory_entry | USER | WRITABLE,
        );
    }

    write_entry(table_address, page_index, entry);
    invalidate_page(virtual_address);
    Ok(())
}

/// Removes one 4 KiB mapping and returns the mapped physical page base.
///
/// If the address currently belongs to a 2 MiB bootstrap mapping, that large
/// mapping is first split into 512 equivalent 4 KiB entries. Only the selected
/// leaf is then removed. The page-table frame is retained for safe reuse.
pub fn unmap_page(virtual_address: u64) -> Result<u64, VmmError> {
    ensure_ready()?;
    ensure_interrupts_disabled()?;
    validate_virtual_page_address(virtual_address)?;

    let path = walk_to_directory(virtual_address)?;
    if path.directory_entry & PRESENT == 0 {
        return Err(VmmError::NotMapped);
    }

    let table_address = if path.directory_entry & PAGE_SIZE_BIT != 0 {
        split_large_page(path)?
    } else {
        path.directory_entry & PAGE_TABLE_ADDRESS_MASK
    };

    let page_index = ((virtual_address >> 12) & 0x1FF) as usize;
    let entry = unsafe { read_entry(table_address, page_index) };
    if entry & PRESENT == 0 {
        return Err(VmmError::NotMapped);
    }

    write_entry(table_address, page_index, 0);
    invalidate_page(virtual_address);

    Ok(entry & PAGE_TABLE_ADDRESS_MASK)
}

/// Returns the current 4 KiB mapping and effective leaf flags for an address.
pub fn query_page(virtual_address: u64) -> Result<PageMapping, VmmError> {
    ensure_ready()?;
    if virtual_address >= VIRTUAL_ADDRESS_LIMIT {
        return Err(VmmError::AddressOutOfRange);
    }

    let path = walk_to_directory(virtual_address)?;
    if path.directory_entry & PRESENT == 0 {
        return Err(VmmError::NotMapped);
    }

    let virtual_page = virtual_address & !(PAGE_SIZE - 1);
    if path.directory_entry & PAGE_SIZE_BIT != 0 {
        let physical_base = path.directory_entry & LARGE_PAGE_ADDRESS_MASK;
        let physical_page = (physical_base | (virtual_address & (TWO_MIB - 1))) & !(PAGE_SIZE - 1);
        return Ok(PageMapping {
            virtual_page,
            physical_page,
            flags: flags_from_entry(path.directory_entry),
        });
    }

    let table_address = path.directory_entry & PAGE_TABLE_ADDRESS_MASK;
    let page_index = ((virtual_address >> 12) & 0x1FF) as usize;
    let entry = unsafe { read_entry(table_address, page_index) };
    if entry & PRESENT == 0 {
        return Err(VmmError::NotMapped);
    }

    Ok(PageMapping {
        virtual_page,
        physical_page: entry & PAGE_TABLE_ADDRESS_MASK,
        flags: flags_from_entry(entry),
    })
}

/// Attempts to collapse a complete 2 MiB region of equivalent 4 KiB entries.
///
/// The operation succeeds only when all 512 leaves are present, physically
/// contiguous, 2 MiB aligned and have identical permissions/cache attributes.
/// Accessed and dirty bits may differ and are merged. After installing the
/// huge-page entry, every 4 KiB address in the region is invalidated before
/// the now-unreferenced page-table frame is returned to the physical allocator.
pub fn try_coalesce_large_page(virtual_address: u64) -> Result<bool, VmmError> {
    ensure_ready()?;
    ensure_interrupts_disabled()?;
    if virtual_address >= VIRTUAL_ADDRESS_LIMIT {
        return Err(VmmError::AddressOutOfRange);
    }

    let path = walk_to_directory(virtual_address)?;
    if path.directory_entry & PRESENT == 0 {
        return Ok(false);
    }
    if path.directory_entry & PAGE_SIZE_BIT != 0 {
        return Ok(true);
    }

    let table_address = path.directory_entry & PAGE_TABLE_ADDRESS_MASK;
    if table_address == 0 || table_address % PAGE_SIZE != 0 {
        return Err(VmmError::InvalidPhysicalFrame);
    }

    // A 4 KiB PAT bit is bit 7; in a 2 MiB PDE, PAT moves to bit 12.
    const LEAF_ATTRIBUTES: u64 =
        PRESENT | WRITABLE | USER | WRITE_THROUGH | CACHE_DISABLE | GLOBAL | NO_EXECUTE | PAGE_PAT;
    const HARDWARE_STATUS: u64 = ACCESSED | DIRTY;

    let first = unsafe { read_entry(table_address, 0) };
    if first & PRESENT == 0 {
        return Ok(false);
    }

    let physical_base = first & PAGE_TABLE_ADDRESS_MASK;
    if physical_base % TWO_MIB != 0 {
        return Ok(false);
    }

    let common_attributes = first & LEAF_ATTRIBUTES;
    let mut hardware_status = first & HARDWARE_STATUS;

    for index in 0..512usize {
        let entry = unsafe { read_entry(table_address, index) };
        if entry & PRESENT == 0 {
            return Ok(false);
        }

        let expected_physical = physical_base + index as u64 * PAGE_SIZE;
        if entry & PAGE_TABLE_ADDRESS_MASK != expected_physical {
            return Ok(false);
        }

        if entry & LEAF_ATTRIBUTES != common_attributes {
            return Ok(false);
        }

        hardware_status |= entry & HARDWARE_STATUS;
    }

    let mut large_entry =
        physical_base | (common_attributes & !PAGE_PAT) | hardware_status | PAGE_SIZE_BIT;
    if common_attributes & PAGE_PAT != 0 {
        large_entry |= LARGE_PAGE_PAT;
    }

    write_entry(path.directory_address, path.directory_index, large_entry);

    let virtual_base = virtual_address & !(TWO_MIB - 1);
    for index in 0..512u64 {
        invalidate_page(virtual_base + index * PAGE_SIZE);
    }

    // The PDE no longer references the table and no translation in this
    // 2 MiB region can retain a walk through it. This kernel is single-core;
    // IF is clear, so no concurrent page walker can be using the old table.
    memory::release_frame(table_address).map_err(|_| VmmError::PhysicalFrameReleaseFailed)?;

    Ok(true)
}

struct DirectoryPath {
    root_address: u64,
    pml4_index: usize,
    pml4_entry: u64,
    pdpt_address: u64,
    pdpt_index: usize,
    pdpt_entry: u64,
    directory_address: u64,
    directory_index: usize,
    directory_entry: u64,
}

fn walk_to_directory_for_mapping(virtual_address: u64) -> Result<DirectoryPath, VmmError> {
    if virtual_address >= VIRTUAL_ADDRESS_LIMIT {
        return Err(VmmError::AddressOutOfRange);
    }

    let root_address = read_cr3() & PAGE_TABLE_ADDRESS_MASK;
    let pml4_index = ((virtual_address >> 39) & 0x1FF) as usize;
    let pdpt_index = ((virtual_address >> 30) & 0x1FF) as usize;
    let directory_index = ((virtual_address >> 21) & 0x1FF) as usize;

    let pml4_entry = unsafe { read_entry(root_address, pml4_index) };
    if pml4_entry & PRESENT == 0 {
        return Err(VmmError::MissingPageTableEntry);
    }

    let pdpt_address = pml4_entry & PAGE_TABLE_ADDRESS_MASK;
    let mut pdpt_entry = unsafe { read_entry(pdpt_address, pdpt_index) };
    if pdpt_entry & PRESENT == 0 {
        let directory_frame = allocate_zeroed_table()?;
        pdpt_entry = directory_frame | PRESENT | WRITABLE;
        write_entry(pdpt_address, pdpt_index, pdpt_entry);
    }
    if pdpt_entry & PAGE_SIZE_BIT != 0 {
        return Err(VmmError::UnsupportedOneGiBPage);
    }

    let directory_address = pdpt_entry & PAGE_TABLE_ADDRESS_MASK;
    let directory_entry = unsafe { read_entry(directory_address, directory_index) };

    Ok(DirectoryPath {
        root_address,
        pml4_index,
        pml4_entry,
        pdpt_address,
        pdpt_index,
        pdpt_entry,
        directory_address,
        directory_index,
        directory_entry,
    })
}

fn walk_to_directory(virtual_address: u64) -> Result<DirectoryPath, VmmError> {
    if virtual_address >= VIRTUAL_ADDRESS_LIMIT {
        return Err(VmmError::AddressOutOfRange);
    }

    let root_address = read_cr3() & PAGE_TABLE_ADDRESS_MASK;
    let pml4_index = ((virtual_address >> 39) & 0x1FF) as usize;
    let pdpt_index = ((virtual_address >> 30) & 0x1FF) as usize;
    let directory_index = ((virtual_address >> 21) & 0x1FF) as usize;

    let pml4_entry = unsafe { read_entry(root_address, pml4_index) };
    if pml4_entry & PRESENT == 0 {
        return Err(VmmError::MissingPageTableEntry);
    }

    let pdpt_address = pml4_entry & PAGE_TABLE_ADDRESS_MASK;
    let pdpt_entry = unsafe { read_entry(pdpt_address, pdpt_index) };
    if pdpt_entry & PRESENT == 0 {
        return Err(VmmError::MissingPageTableEntry);
    }
    if pdpt_entry & PAGE_SIZE_BIT != 0 {
        return Err(VmmError::UnsupportedOneGiBPage);
    }

    let directory_address = pdpt_entry & PAGE_TABLE_ADDRESS_MASK;
    let directory_entry = unsafe { read_entry(directory_address, directory_index) };

    Ok(DirectoryPath {
        root_address,
        pml4_index,
        pml4_entry,
        pdpt_address,
        pdpt_index,
        pdpt_entry,
        directory_address,
        directory_index,
        directory_entry,
    })
}

fn split_large_page(path: DirectoryPath) -> Result<u64, VmmError> {
    let old_entry = path.directory_entry;
    if old_entry & PRESENT == 0 || old_entry & PAGE_SIZE_BIT == 0 {
        return Err(VmmError::NotMapped);
    }

    let table_address = allocate_zeroed_table()?;
    let physical_base = old_entry & LARGE_PAGE_ADDRESS_MASK;

    // Preserve the large-page attributes when converting the PAT location
    // from bit 12 (2 MiB leaf) to bit 7 (4 KiB leaf).
    let mut leaf_flags = old_entry
        & (PRESENT
            | WRITABLE
            | USER
            | WRITE_THROUGH
            | CACHE_DISABLE
            | ACCESSED
            | DIRTY
            | GLOBAL
            | NO_EXECUTE);
    if old_entry & LARGE_PAGE_PAT != 0 {
        leaf_flags |= PAGE_PAT;
    }

    for index in 0..512usize {
        let physical_address = physical_base + index as u64 * PAGE_SIZE;
        write_entry(table_address, index, physical_address | leaf_flags);
    }

    // The parent becomes a page-table pointer, so its RW bit is enabled and
    // access control is enforced by each 4 KiB leaf.
    let mut parent_flags = PRESENT | WRITABLE;
    if old_entry & USER != 0 {
        parent_flags |= USER;
    }
    write_entry(
        path.directory_address,
        path.directory_index,
        table_address | parent_flags,
    );

    Ok(table_address)
}

fn allocate_zeroed_table() -> Result<u64, VmmError> {
    let frame = memory::allocate_frame().map_err(|_| VmmError::OutOfPhysicalFrames)?;
    if frame == 0 || frame % PAGE_SIZE != 0 || frame >= IDENTITY_MAP_LIMIT {
        let _ = memory::release_frame(frame);
        return Err(VmmError::InvalidPhysicalFrame);
    }

    // The active bootstrap identity map covers all allocator-managed frames.
    // The frame remains allocated for as long as a page-table entry refers to it.
    unsafe {
        ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
    }
    Ok(frame)
}

fn validate_virtual_page_address(address: u64) -> Result<(), VmmError> {
    if address % PAGE_SIZE != 0 {
        return Err(VmmError::UnalignedAddress);
    }
    if address >= VIRTUAL_ADDRESS_LIMIT {
        return Err(VmmError::AddressOutOfRange);
    }
    Ok(())
}

fn validate_physical_page_address(address: u64) -> Result<(), VmmError> {
    if address % PAGE_SIZE != 0 {
        return Err(VmmError::UnalignedAddress);
    }
    if address >= IDENTITY_MAP_LIMIT {
        return Err(VmmError::AddressOutOfRange);
    }
    Ok(())
}

fn flags_from_entry(entry: u64) -> PageFlags {
    PageFlags {
        writable: entry & WRITABLE != 0,
        user: entry & USER != 0,
        executable: entry & NO_EXECUTE == 0,
        write_through: entry & WRITE_THROUGH != 0,
        cache_disable: entry & CACHE_DISABLE != 0,
        global: entry & GLOBAL != 0,
    }
}

fn ensure_ready() -> Result<(), VmmError> {
    // SAFETY: VMM_INITIALIZED is set once during single-core startup.
    if unsafe { VMM_INITIALIZED } {
        Ok(())
    } else {
        Err(VmmError::NotInitialized)
    }
}

fn ensure_interrupts_disabled() -> Result<(), VmmError> {
    if read_rflags() & RFLAGS_IF == 0 {
        Ok(())
    } else {
        Err(VmmError::InterruptsEnabled)
    }
}

fn enable_nx() -> bool {
    let maximum = __cpuid(0x8000_0000);
    if maximum.eax < 0x8000_0001 {
        return false;
    }

    let features = __cpuid(0x8000_0001);
    if features.edx & (1 << 20) == 0 {
        return false;
    }

    let current = read_msr(EFER_MSR);
    write_msr(EFER_MSR, current | EFER_NXE);
    read_msr(EFER_MSR) & EFER_NXE != 0
}

fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

fn write_msr(msr: u32, value: u64) {
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nomem, nostack, preserves_flags)
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

fn write_entry(table_address: u64, index: usize, value: u64) {
    // SAFETY: The caller supplies a valid identity-mapped page-table frame and
    // an index below 512. Page-table frames are never released while active.
    unsafe {
        ptr::write_volatile((table_address as *mut u64).add(index), value);
    }
}

unsafe fn read_entry(table_address: u64, index: usize) -> u64 {
    // SAFETY: Callers have validated the parent entry and index bounds.
    unsafe { ptr::read_volatile((table_address as *const u64).add(index)) }
}

fn invalidate_page(virtual_address: u64) {
    // SAFETY: INVLPG is privileged and this code only runs in ring 0. The
    // address is page-aligned or belongs to the page just changed.
    unsafe {
        asm!(
            "invlpg [{}]",
            in(reg) virtual_address,
            options(nostack, preserves_flags)
        );
    }
}
