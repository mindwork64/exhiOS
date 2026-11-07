use exhi_abi::BootInfo;

use crate::{arch::x86_64::vmm, memory};

const PAGE_SIZE: u64 = 4096;
const IDENTITY_MAP_LIMIT: u64 = 0x1_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StackGuardError {
    InvalidStackBounds,
    GuardAddressOverflow,
    PhysicalReservationFailed,
    UnexpectedPhysicalMapping,
    GuardMappingStillPresent,
    StackMappingLost,
    Vmm(vmm::VmmError),
}

impl StackGuardError {
    pub const fn message(self) -> &'static [u8] {
        match self {
            Self::InvalidStackBounds => b"BootInfo kernel stack bounds are invalid",
            Self::GuardAddressOverflow => b"kernel stack has no valid page below it",
            Self::PhysicalReservationFailed => b"physical guard page could not be reserved",
            Self::UnexpectedPhysicalMapping => {
                b"guard virtual address does not map to the expected physical page"
            }
            Self::GuardMappingStillPresent => b"kernel stack guard mapping is still present",
            Self::StackMappingLost => {
                b"kernel stack mapping was changed while installing guard page"
            }
            Self::Vmm(error) => error.message(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StackGuardReservation {
    pub stack_base: u64,
    pub stack_size: u64,
    pub stack_end: u64,
    pub guard_page: u64,
}

/// Validates the boot stack and reserves its lower guard frame before any
/// subsequent kernel allocations can claim it.
pub fn reserve_before_allocations(
    boot_info: &BootInfo,
) -> Result<StackGuardReservation, StackGuardError> {
    let stack_base = boot_info.kernel_stack_base;
    let stack_size = boot_info.kernel_stack_size;
    let stack_end = stack_base
        .checked_add(stack_size)
        .ok_or(StackGuardError::InvalidStackBounds)?;

    if stack_base == 0
        || stack_size < PAGE_SIZE * 2
        || stack_base % PAGE_SIZE != 0
        || stack_size % PAGE_SIZE != 0
        || stack_end > IDENTITY_MAP_LIMIT
    {
        return Err(StackGuardError::InvalidStackBounds);
    }

    let guard_page = stack_base
        .checked_sub(PAGE_SIZE)
        .ok_or(StackGuardError::GuardAddressOverflow)?;

    // The UEFI memory-map buffer can occupy this physical page. The allocator
    // consumes that buffer before this function is called; after this point it
    // must never allocate the page even when firmware labels it Conventional.
    memory::reserve_frame(guard_page).map_err(|_| StackGuardError::PhysicalReservationFailed)?;

    Ok(StackGuardReservation {
        stack_base,
        stack_size,
        stack_end,
        guard_page,
    })
}

/// Removes the supervisor identity mapping directly below the boot stack.
pub fn install(reservation: StackGuardReservation) -> Result<(), StackGuardError> {
    let before = vmm::query_page(reservation.guard_page).map_err(StackGuardError::Vmm)?;
    if before.physical_page != reservation.guard_page || before.flags.user {
        return Err(StackGuardError::UnexpectedPhysicalMapping);
    }

    let removed = vmm::unmap_page(reservation.guard_page).map_err(StackGuardError::Vmm)?;
    if removed != reservation.guard_page {
        return Err(StackGuardError::UnexpectedPhysicalMapping);
    }

    if vmm::query_page(reservation.guard_page) != Err(vmm::VmmError::NotMapped) {
        return Err(StackGuardError::GuardMappingStillPresent);
    }

    let first_stack_page = vmm::query_page(reservation.stack_base).map_err(StackGuardError::Vmm)?;
    let last_stack_page_address = reservation.stack_end - PAGE_SIZE;
    let last_stack_page = vmm::query_page(last_stack_page_address).map_err(StackGuardError::Vmm)?;

    if first_stack_page.physical_page != reservation.stack_base
        || last_stack_page.physical_page != last_stack_page_address
        || first_stack_page.flags.user
        || last_stack_page.flags.user
    {
        return Err(StackGuardError::StackMappingLost);
    }

    Ok(())
}
