use core::arch::asm;
use core::ptr;

use exhi_abi::BootInfo;

const PAGE_SIZE: u64 = 4096;
const PAGE_ENTRIES: usize = 512;
const TWO_MIB: u64 = 0x20_0000;
const IDENTITY_MAP_LIMIT: u64 = 0x1_0000_0000;
const PAGE_DIRECTORY_COUNT: usize = 4;
const TABLE_COUNT: usize = 2 + PAGE_DIRECTORY_COUNT;
const TABLE_STORAGE_BYTES: u64 = PAGE_SIZE * TABLE_COUNT as u64;

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const HUGE_PAGE: u64 = 1 << 7;
const PAGE_TABLE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const LARGE_PAGE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFE0_0000;

const CR0_PG: u64 = 1 << 31;
const CR4_PAE: u64 = 1 << 5;
const CR4_LA57: u64 = 1 << 12;
const RFLAGS_IF: u64 = 1 << 9;
const EFER_MSR: u32 = 0xC000_0080;
const EFER_LMA: u64 = 1 << 10;

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageTable {
    entries: [u64; PAGE_ENTRIES],
}

impl PageTable {
    const fn empty() -> Self {
        Self {
            entries: [0; PAGE_ENTRIES],
        }
    }
}

// Bootstrap tables live in the kernel's own BSS. This is deliberate: before
// the first CR3 switch, the kernel must not assume that arbitrary Conventional
// Memory frames returned by the allocator are accessible through UEFI mappings.
static mut BOOTSTRAP_TABLES: [PageTable; TABLE_COUNT] = [PageTable::empty(); TABLE_COUNT];
static mut PAGE_TABLES_ACTIVE: bool = false;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagingError {
    AlreadyInitialized,
    NotInitialized,
    InterruptsEnabled,
    CpuNotInLongMode,
    PaeNotEnabled,
    FiveLevelPagingNotSupportedByBootstrap,
    InvalidPageTableStorage,
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
            Self::NotInitialized => b"kernel page tables are not initialized",
            Self::InterruptsEnabled => b"interrupts must remain disabled during paging setup",
            Self::CpuNotInLongMode => b"CPU is not in 64-bit paging mode",
            Self::PaeNotEnabled => b"CR4.PAE is not enabled",
            Self::FiveLevelPagingNotSupportedByBootstrap => {
                b"five-level paging is active; bootstrap supports four levels"
            }
            Self::InvalidPageTableStorage => b"bootstrap page-table storage is invalid or above 4 GiB",
            Self::AddressOutOfRange => b"required address range is outside the initial 4 GiB identity map",
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
    pub verified_ranges: u64,
}

/// Builds and activates a kernel-owned four-level page-table hierarchy.
///
/// The initial map identity-maps physical addresses 0..4 GiB using 2 MiB
/// supervisor read/write pages. It includes the kernel, current stack, BootInfo,
/// UEFI memory map, GOP framebuffer, ACPI RSDP, GDT, IDT and the tables
/// themselves. This is a temporary bootstrap map: it does not yet implement
/// W^X, user/supervisor isolation, per-device cache attributes or process
/// address spaces.
///
/// The page tables are stored in aligned kernel BSS so they can be populated
/// safely under the firmware's existing mappings. Dynamic page-table frames
/// can be allocated from the physical allocator after this map is active.
pub fn initialize_identity_map_4g(boot_info: &BootInfo) -> Result<PagingStatus, PagingError> {
    // SAFETY: Kernel startup is single-threaded and IF remains clear.
    if unsafe { PAGE_TABLES_ACTIVE } {
        return Err(PagingError::AlreadyInitialized);
    }

    if read_rflags() & RFLAGS_IF != 0 {
        return Err(PagingError::InterruptsEnabled);
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

    let root_address = table_address(0);
    let table_end = root_address
        .checked_add(TABLE_STORAGE_BYTES)
        .ok_or(PagingError::InvalidPageTableStorage)?;

    if root_address % PAGE_SIZE != 0
        || root_address == 0
        || table_end > IDENTITY_MAP_LIMIT
    {
        return Err(PagingError::InvalidPageTableStorage);
    }

    let (gdtr_base, gdtr_limit) = read_gdtr();
    let (idtr_base, idtr_limit) = read_idtr();
    let boot_info_address = boot_info as *const BootInfo as u64;
    let kernel_code_address = initialize_identity_map_4g as *const () as u64;
    let stack_address = read_stack_pointer();

    let ranges = [
        (kernel_code_address, 1u64),
        (stack_address, 8u64),
        (boot_info_address, core::mem::size_of::<BootInfo>() as u64),
        (
            boot_info.memory_map.physical_address,
            boot_info.memory_map.map_size,
        ),
        (
            boot_info.framebuffer.physical_address,
            boot_info.framebuffer.buffer_size,
        ),
        (boot_info.acpi_rsdp_address, 36u64),
        (gdtr_base, u64::from(gdtr_limit) + 1),
        (idtr_base, u64::from(idtr_limit) + 1),
        (root_address, TABLE_STORAGE_BYTES),
    ];

    // Validate all firmware handoff objects before switching away from the
    // firmware root. A successful check guarantees that the bootstrap map
    // covers every address the kernel will continue using.
    for &(start, length) in &ranges {
        validate_identity_range(start, length)?;
    }

    // Zero the six page-aligned BSS pages and build:
    // PML4[0] -> PDPT[0..4] -> four page directories -> 2048 2 MiB pages.
    // SAFETY: The table array is a dedicated aligned BSS allocation owned by
    // the kernel. It is writable through the currently active firmware map.
    unsafe {
        ptr::write_bytes(
            ptr::addr_of_mut!(BOOTSTRAP_TABLES).cast::<u8>(),
            0,
            TABLE_STORAGE_BYTES as usize,
        );
    }

    write_entry(table_address(0), 0, table_address(1) | PRESENT | WRITABLE);

    for directory_index in 0..PAGE_DIRECTORY_COUNT {
        let directory_address = table_address(2 + directory_index);
        write_entry(
            table_address(1),
            directory_index,
            directory_address | PRESENT | WRITABLE,
        );

        for entry_index in 0..PAGE_ENTRIES {
            let page_number = (directory_index * PAGE_ENTRIES + entry_index) as u64;
            let physical_base = page_number * TWO_MIB;
            write_entry(
                directory_address,
                entry_index,
                physical_base | PRESENT | WRITABLE | HUGE_PAGE,
            );
        }
    }

    // Check the constructed tables while the firmware root is still active.
    // The translator walks only the BSS tables built above.
    let mut verified_ranges = 0u64;
    for &(start, length) in &ranges {
        if length != 0 {
            verify_identity_range(root_address, start, length)?;
            verified_ranges += 1;
        }
    }

    let old_cr3 = read_cr3();

    // SAFETY: The new PML4 is page-aligned, fully populated and itself lies
    // inside the region identity-mapped by both the old and new roots.
    unsafe {
        asm!(
            "mov cr3, {}",
            in(reg) root_address,
            options(nostack, preserves_flags)
        );
    }

    let active_cr3 = read_cr3() & PAGE_TABLE_ADDRESS_MASK;
    if active_cr3 != root_address {
        // SAFETY: Restore the old root before reporting the failed switch.
        unsafe {
            asm!("mov cr3, {}", in(reg) old_cr3, options(nostack, preserves_flags));
        }
        return Err(PagingError::Cr3Mismatch);
    }

    // Re-check the same critical address ranges using the now-active CR3.
    for &(start, length) in &ranges {
        if length != 0 {
            if let Err(error) = verify_identity_range(active_cr3, start, length) {
                // SAFETY: Return to the firmware root before returning an error.
                unsafe {
                    asm!("mov cr3, {}", in(reg) old_cr3, options(nostack, preserves_flags));
                }
                return Err(error);
            }
        }
    }

    // SAFETY: Startup is single-threaded with IF clear. The static tables stay
    // reserved as part of the kernel image for the lifetime of the kernel.
    unsafe {
        PAGE_TABLES_ACTIVE = true;
    }

    Ok(PagingStatus {
        root_address,
        table_pages: TABLE_COUNT as u64,
        mapped_bytes: IDENTITY_MAP_LIMIT,
        large_page_size: TWO_MIB,
        active_cr3,
        verified_stack_address: stack_address,
        verified_ranges,
    })
}

/// Translates an address through the currently active kernel page tables.
pub fn translate_current(virtual_address: u64) -> Result<u64, PagingError> {
    // SAFETY: This flag is written once during single-core startup with IF clear.
    if !unsafe { PAGE_TABLES_ACTIVE } {
        return Err(PagingError::NotInitialized);
    }

    translate_from_root(read_cr3() & PAGE_TABLE_ADDRESS_MASK, virtual_address)
}

fn verify_identity_range(
    root_address: u64,
    start: u64,
    length: u64,
) -> Result<(), PagingError> {
    validate_identity_range(start, length)?;
    if length == 0 {
        return Ok(());
    }

    let end = start
        .checked_add(length - 1)
        .ok_or(PagingError::AddressOutOfRange)?;

    let mut address = start;
    loop {
        if translate_from_root(root_address, address)? != address {
            return Err(PagingError::IdentityMappingVerificationFailed);
        }

        if address == end {
            break;
        }

        let next_boundary = (address & !(TWO_MIB - 1))
            .checked_add(TWO_MIB)
            .ok_or(PagingError::AddressOutOfRange)?;
        if next_boundary > end {
            address = end;
        } else {
            address = next_boundary;
        }
    }

    Ok(())
}

fn validate_identity_range(start: u64, length: u64) -> Result<(), PagingError> {
    if length == 0 {
        return Ok(());
    }

    let end = start
        .checked_add(length - 1)
        .ok_or(PagingError::AddressOutOfRange)?;

    if start >= IDENTITY_MAP_LIMIT || end >= IDENTITY_MAP_LIMIT {
        return Err(PagingError::AddressOutOfRange);
    }

    Ok(())
}

fn translate_from_root(root_address: u64, virtual_address: u64) -> Result<u64, PagingError> {
    if virtual_address >= IDENTITY_MAP_LIMIT {
        return Err(PagingError::AddressOutOfRange);
    }

    let pml4_index = ((virtual_address >> 39) & 0x1FF) as usize;
    let pdpt_index = ((virtual_address >> 30) & 0x1FF) as usize;
    let pd_index = ((virtual_address >> 21) & 0x1FF) as usize;

    // SAFETY: Before the switch root_address is the kernel-owned BSS PML4;
    // after the switch it is the active PML4. All children are builder-owned
    // identity-mapped BSS tables, and indexes are masked to 0..512.
    unsafe {
        let pml4e = read_entry(root_address, pml4_index);
        if pml4e & PRESENT == 0 {
            return Err(PagingError::MissingPageTableEntry);
        }

        let pdpt_address = pml4e & PAGE_TABLE_ADDRESS_MASK;
        let pdpte = read_entry(pdpt_address, pdpt_index);
        if pdpte & PRESENT == 0 {
            return Err(PagingError::MissingPageTableEntry);
        }
        if pdpte & HUGE_PAGE != 0 {
            return Err(PagingError::UnexpectedPageSize);
        }

        let directory_address = pdpte & PAGE_TABLE_ADDRESS_MASK;
        let pde = read_entry(directory_address, pd_index);
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

fn table_address(index: usize) -> u64 {
    // SAFETY: Internal callers use only indexes 0..TABLE_COUNT.
    unsafe {
        (ptr::addr_of_mut!(BOOTSTRAP_TABLES) as *mut PageTable).add(index) as u64
    }
}

fn write_entry(table_address: u64, index: usize, value: u64) {
    // SAFETY: The caller supplies one of our aligned table addresses and an
    // index below 512; the BSS is writable under the current firmware map.
    unsafe {
        ptr::write_volatile((table_address as *mut u64).add(index), value);
    }
}

unsafe fn read_entry(table_address: u64, index: usize) -> u64 {
    // SAFETY: Callers guarantee a valid table base and an index below 512.
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

fn read_gdtr() -> (u64, u16) {
    let mut bytes = [0u8; 10];
    unsafe {
        asm!(
            "sgdt [{}]",
            in(reg) bytes.as_mut_ptr(),
            options(nostack, preserves_flags)
        );
    }
    decode_descriptor_table_pointer(bytes)
}

fn read_idtr() -> (u64, u16) {
    let mut bytes = [0u8; 10];
    unsafe {
        asm!(
            "sidt [{}]",
            in(reg) bytes.as_mut_ptr(),
            options(nostack, preserves_flags)
        );
    }
    decode_descriptor_table_pointer(bytes)
}

fn decode_descriptor_table_pointer(bytes: [u8; 10]) -> (u64, u16) {
    let limit = u16::from_le_bytes([bytes[0], bytes[1]]);
    let base = u64::from_le_bytes([
        bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
    ]);
    (base, limit)
}
