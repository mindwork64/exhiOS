use core::arch::asm;
use core::ptr;

use crate::memory::{self, MemoryError};

const PAGE_SIZE: u64 = 4096;
const PAGE_ENTRIES: usize = 512;
const TWO_MIB: u64 = 0x20_0000;
const IDENTITY_MAP_LIMIT: u64 = 0x1_0000_0000;
const PAGE_DIRECTORY_COUNT: usize = 4;
const TABLE_COUNT: usize = 2 + PAGE_DIRECTORY_COUNT;

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const HUGE_PAGE: u64 = 1 << 7;
const PAGE_TABLE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const LARGE_PAGE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFE0_0000;

const CR0_PG: u64 = 1 << 31;
const CR4_PAE: u64 = 1 << 5;
const CR4_LA57: u64 = 1 << 12;
const EFER_MSR: u32 = 0xC000_0080;
const EFER_LMA: u64 = 1 << 10;

static mut PAGE_TABLE_FRAMES: [u64; TABLE_COUNT] = [0; TABLE_COUNT];
static mut PAGE_TABLES_ACTIVE: bool = false;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagingError {
    AlreadyInitialized,
    CpuNotInLongMode,
    PaeNotEnabled,
    FiveLevelPagingNotSupportedByBootstrap,
    FrameAllocator(MemoryError),
    InvalidFrameAddress,
    AddressOutOfRange,
    MissingPageTableEntry,
    UnexpectedPageSize,
    Cr3Mismatch,
    IdentityMappingVerificationFailed,
}

impl PagingError {
    pub const fn message(self) -> &'static [u8] {
        match self {
            Self::AlreadyInitialized => b"kernel page tables are already active",
            Self::CpuNotInLongMode => b"CPU is not in 64-bit paging mode",
            Self::PaeNotEnabled => b"CR4.PAE is not enabled",
            Self::FiveLevelPagingNotSupportedByBootstrap => {
                b"five-level paging is active; bootstrap supports four levels"
            }
            Self::FrameAllocator(_) => b"cannot allocate page-table frame",
            Self::InvalidFrameAddress => b"page-table frame is invalid or above 4 GiB",
            Self::AddressOutOfRange => b"virtual address is outside the initial 4 GiB identity map",
            Self::MissingPageTableEntry => b"required page-table entry is not present",
            Self::UnexpectedPageSize => b"unexpected page-table entry size",
            Self::Cr3Mismatch => b"CR3 readback does not match the new PML4 address",
            Self::IdentityMappingVerificationFailed => b"identity mapping verification failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PagingStatus {
    pub root_address: u64,
    pub table_pages: u64,
    pub mapped_bytes: u64,
    pub large_page_size: u64,
    pub active_cr3: u64,
    pub verified_stack_address: u64,
}

/// Builds and activates a kernel-owned four-level page-table hierarchy.
///
/// This first bootstrap map identity-maps physical addresses 0..4 GiB using
/// 2 MiB supervisor read/write pages. It is intentionally broad and does not
/// yet provide W^X or user/supervisor isolation. Later VMM work will replace
/// these coarse mappings with per-region 4 KiB mappings and explicit cache
/// attributes.
pub fn initialize_identity_map_4g() -> Result<PagingStatus, PagingError> {
    // SAFETY: This is called once during single-core startup with IF clear.
    if unsafe { PAGE_TABLES_ACTIVE } {
        return Err(PagingError::AlreadyInitialized);
    }

    let (cr0, cr4, efer) = read_cpu_paging_state();
    if cr0 & CR0_PG == 0 || efer & EFER_LMA == 0 {
        return Err(PagingError::CpuNotInLongMode);
    }
    if cr4 & CR4_PAE == 0 {
        return Err(PagingError::PaeNotEnabled);
    }
    if cr4 & CR4_LA57 != 0 {
        return Err(PagingError::FiveLevelPagingNotSupportedByBootstrap);
    }

    let mut frames = [0u64; TABLE_COUNT];
    let mut allocated_count = 0usize;

    while allocated_count < TABLE_COUNT {
        match memory::allocate_frame() {
            Ok(address)
                if address < IDENTITY_MAP_LIMIT && address % PAGE_SIZE == 0 =>
            {
                frames[allocated_count] = address;
                allocated_count += 1;
            }
            Ok(address) => {
                let _ = memory::release_frame(address);
                release_allocated_frames(&frames, allocated_count);
                return Err(PagingError::InvalidFrameAddress);
            }
            Err(error) => {
                release_allocated_frames(&frames, allocated_count);
                return Err(PagingError::FrameAllocator(error));
            }
        }
    }

    // SAFETY: Each frame was returned by the physical allocator from
    // EfiConventionalMemory and is still identity-accessible under the
    // currently active UEFI page tables. Each allocation is exactly 4 KiB.
    for frame in frames {
        unsafe {
            ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
        }
    }

    // PML4[0] -> one PDPT. Its first four entries cover [0, 4 GiB).
    write_entry(frames[0], 0, frames[1] | PRESENT | WRITABLE);

    for directory_index in 0..PAGE_DIRECTORY_COUNT {
        let directory_frame = frames[2 + directory_index];
        write_entry(
            frames[1],
            directory_index,
            directory_frame | PRESENT | WRITABLE,
        );

        for entry_index in 0..PAGE_ENTRIES {
            let page_number = (directory_index * PAGE_ENTRIES + entry_index) as u64;
            let physical_base = page_number * TWO_MIB;
            let entry = physical_base | PRESENT | WRITABLE | HUGE_PAGE;
            write_entry(directory_frame, entry_index, entry);
        }
    }

    // The old and new page tables both identity-map the executing code, stack,
    // table frames and firmware handoff data. Thus the next instruction and
    // its stack remain valid across this CR3 switch.
    let old_cr3 = read_cr3();
    let root_address = frames[0];

    // SAFETY: root_address is an aligned physical address of our populated
    // PML4 and is mapped at the same virtual address in the old page tables.
    unsafe {
        asm!(
            "mov cr3, {}",
            in(reg) root_address,
            options(nostack, preserves_flags)
        );
    }

    let active_cr3 = read_cr3() & PAGE_TABLE_ADDRESS_MASK;
    if active_cr3 != root_address {
        // SAFETY: The old CR3 value is restored before releasing the new tables.
        unsafe {
            asm!("mov cr3, {}", in(reg) old_cr3, options(nostack, preserves_flags));
        }
        release_allocated_frames(&frames, allocated_count);
        return Err(PagingError::Cr3Mismatch);
    }

    let stack_address = read_stack_pointer();
    let probes = [
        initialize_identity_map_4g as *const () as u64,
        stack_address,
        root_address,
    ];

    for address in probes {
        match translate_current(address) {
            Ok(physical) if physical == address => {}
            _ => {
                // SAFETY: Restore UEFI's root before returning new table frames.
                unsafe {
                    asm!("mov cr3, {}", in(reg) old_cr3, options(nostack, preserves_flags));
                }
                release_allocated_frames(&frames, allocated_count);
                return Err(PagingError::IdentityMappingVerificationFailed);
            }
        }
    }

    // SAFETY: Startup is single-threaded with IF clear; the static array
    // records frames that must remain allocated for the active page tables.
    unsafe {
        PAGE_TABLE_FRAMES = frames;
        PAGE_TABLES_ACTIVE = true;
    }

    Ok(PagingStatus {
        root_address,
        table_pages: TABLE_COUNT as u64,
        mapped_bytes: IDENTITY_MAP_LIMIT,
        large_page_size: TWO_MIB,
        active_cr3,
        verified_stack_address: stack_address,
    })
}

/// Translates an address using the currently active bootstrap page tables.
/// The initial implementation expects 2 MiB pages and addresses below 4 GiB.
pub fn translate_current(virtual_address: u64) -> Result<u64, PagingError> {
    if virtual_address >= IDENTITY_MAP_LIMIT {
        return Err(PagingError::AddressOutOfRange);
    }

    let root = read_cr3() & PAGE_TABLE_ADDRESS_MASK;
    let pml4_index = ((virtual_address >> 39) & 0x1FF) as usize;
    let pdpt_index = ((virtual_address >> 30) & 0x1FF) as usize;
    let pd_index = ((virtual_address >> 21) & 0x1FF) as usize;

    // SAFETY: The active CR3 points to our PML4 after initialization. Every
    // table is itself identity-mapped by the bootstrap hierarchy.
    unsafe {
        let pml4e = read_entry(root, pml4_index);
        if pml4e & PRESENT == 0 {
            return Err(PagingError::MissingPageTableEntry);
        }

        let pdpt = pml4e & PAGE_TABLE_ADDRESS_MASK;
        let pdpte = read_entry(pdpt, pdpt_index);
        if pdpte & PRESENT == 0 {
            return Err(PagingError::MissingPageTableEntry);
        }
        if pdpte & HUGE_PAGE != 0 {
            return Err(PagingError::UnexpectedPageSize);
        }

        let directory = pdpte & PAGE_TABLE_ADDRESS_MASK;
        let pde = read_entry(directory, pd_index);
        if pde & PRESENT == 0 {
            return Err(PagingError::MissingPageTableEntry);
        }
        if pde & HUGE_PAGE == 0 {
            return Err(PagingError::UnexpectedPageSize);
        }

        let physical_base = pde & LARGE_PAGE_ADDRESS_MASK;
        Ok(physical_base | (virtual_address & (TWO_MIB - 1)))
    }
}

fn release_allocated_frames(frames: &[u64; TABLE_COUNT], count: usize) {
    for address in frames.iter().take(count).copied() {
        let _ = memory::release_frame(address);
    }
}

fn write_entry(table_address: u64, index: usize, value: u64) {
    // SAFETY: table_address is a page allocated by the physical frame
    // allocator, identity-mapped by UEFI, and index is less than 512.
    unsafe {
        ptr::write_volatile((table_address as *mut u64).add(index), value);
    }
}

unsafe fn read_entry(table_address: u64, index: usize) -> u64 {
    // SAFETY: Callers provide a table base from an active page-table entry
    // and an index masked to the legal 0..512 range.
    unsafe { ptr::read_volatile((table_address as *const u64).add(index)) }
}

fn read_cpu_paging_state() -> (u64, u64, u64) {
    let cr0: u64;
    let cr4: u64;
    let efer_low: u32;
    let efer_high: u32;

    unsafe {
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
        asm!(
            "rdmsr",
            in("ecx") EFER_MSR,
            out("eax") efer_low,
            out("edx") efer_high,
            options(nomem, nostack, preserves_flags)
        );
    }

    let efer = (u64::from(efer_high) << 32) | u64::from(efer_low);
    (cr0, cr4, efer)
}

fn read_cr3() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
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
