use core::alloc::{GlobalAlloc, Layout};
use core::arch::asm;
use core::hint::spin_loop;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use exhi_heap::{Heap, HeapError};

use crate::arch::x86_64::vmm::{self, PageFlags, VmmError};
use crate::memory;

const PAGE_SIZE: u64 = 4096;
const HEAP_BASE: u64 = 0x1_0000_0000;
const HEAP_MAX_BYTES: u64 = 64 * 1024 * 1024;
const HEAP_MAX_PAGES: usize = (HEAP_MAX_BYTES / PAGE_SIZE) as usize;
const INITIAL_HEAP_PAGES: usize = 4;

static HEAP_LOCK: AtomicBool = AtomicBool::new(false);
static mut HEAP_STATE: HeapState = HeapState::new();

struct HeapState {
    allocator: Heap,
    mapped_pages: usize,
    initialized: bool,
}

impl HeapState {
    const fn new() -> Self {
        Self {
            allocator: Heap::new(),
            mapped_pages: 0,
            initialized: false,
        }
    }
}

pub struct KernelHeapAllocator;

unsafe impl Sync for KernelHeapAllocator {}

#[global_allocator]
static GLOBAL_HEAP_ALLOCATOR: KernelHeapAllocator = KernelHeapAllocator;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapInitError {
    AlreadyInitialized,
    OutOfVirtualSpace,
    OutOfPhysicalFrames,
    InvalidPhysicalFrame,
    Vmm(VmmError),
    Heap(HeapError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapTestError {
    NotInitialized,
    AllocationFailed,
    DataMismatch,
    ReuseMismatch,
    FreeListNotCoalesced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeapStatus {
    pub virtual_base: u64,
    pub mapped_pages: usize,
    pub mapped_bytes: u64,
    pub free_bytes: usize,
}

/// Maps the initial heap pages and makes the global allocator ready.
///
/// All mutations run with local interrupts disabled and under a spin lock.
/// The current kernel is single-core; this also gives a safe serialization
/// point for later SMP work.
pub fn initialize() -> Result<HeapStatus, HeapInitError> {
    let _guard = HeapLockGuard::acquire();
    // SAFETY: Access is serialized by HEAP_LOCK with local interrupts disabled.
    let state = unsafe { &mut *ptr::addr_of_mut!(HEAP_STATE) };
    if state.initialized {
        return Err(HeapInitError::AlreadyInitialized);
    }

    grow_pages_locked(state, INITIAL_HEAP_PAGES)?;
    state.initialized = true;
    Ok(status_locked(state))
}

/// Runs allocator alignment, cross-page write/read, growth, free and reuse tests.
pub fn allocation_smoke_test() -> Result<HeapStatus, HeapTestError> {
    {
        let _guard = HeapLockGuard::acquire();
        // SAFETY: Access is serialized by HEAP_LOCK with local interrupts disabled.
        let state = unsafe { &mut *ptr::addr_of_mut!(HEAP_STATE) };
        if !state.initialized {
            return Err(HeapTestError::NotInitialized);
        }
    }

    let small_layout = Layout::from_size_align(256, 64).map_err(|_| HeapTestError::AllocationFailed)?;
    let large_layout = Layout::from_size_align(48 * 1024, 4096)
        .map_err(|_| HeapTestError::AllocationFailed)?;

    // SAFETY: The allocator is initialized and internally serializes each call.
    let small = unsafe { GLOBAL_HEAP_ALLOCATOR.alloc(small_layout) };
    if small.is_null() {
        return Err(HeapTestError::AllocationFailed);
    }
    if small as usize % 64 != 0 {
        unsafe { GLOBAL_HEAP_ALLOCATOR.dealloc(small, small_layout) };
        return Err(HeapTestError::DataMismatch);
    }

    // SAFETY: The allocator is initialized. This size exceeds the initial
    // arena, forcing page-backed heap growth.
    let large = unsafe { GLOBAL_HEAP_ALLOCATOR.alloc(large_layout) };
    if large.is_null() {
        unsafe { GLOBAL_HEAP_ALLOCATOR.dealloc(small, small_layout) };
        return Err(HeapTestError::AllocationFailed);
    }
    if large as usize % 4096 != 0 {
        unsafe {
            GLOBAL_HEAP_ALLOCATOR.dealloc(large, large_layout);
            GLOBAL_HEAP_ALLOCATOR.dealloc(small, small_layout);
        }
        return Err(HeapTestError::DataMismatch);
    }

    // SAFETY: Both allocations are live, writable and disjoint.
    unsafe {
        for index in 0..large_layout.size() {
            ptr::write_volatile(large.add(index), (index as u8).wrapping_mul(37) ^ 0xA5);
        }
        for index in [0usize, 4095, 4096, 8191, large_layout.size() - 1] {
            let expected = (index as u8).wrapping_mul(37) ^ 0xA5;
            if ptr::read_volatile(large.add(index)) != expected {
                GLOBAL_HEAP_ALLOCATOR.dealloc(large, large_layout);
                GLOBAL_HEAP_ALLOCATOR.dealloc(small, small_layout);
                return Err(HeapTestError::DataMismatch);
            }
        }

        GLOBAL_HEAP_ALLOCATOR.dealloc(large, large_layout);
        GLOBAL_HEAP_ALLOCATOR.dealloc(small, small_layout);
    }

    // First-fit after complete coalescing should reuse the original small block.
    // SAFETY: The previous allocations have both been returned.
    let reused = unsafe { GLOBAL_HEAP_ALLOCATOR.alloc(small_layout) };
    if reused.is_null() {
        return Err(HeapTestError::AllocationFailed);
    }
    if reused != small {
        unsafe { GLOBAL_HEAP_ALLOCATOR.dealloc(reused, small_layout) };
        return Err(HeapTestError::ReuseMismatch);
    }
    unsafe { GLOBAL_HEAP_ALLOCATOR.dealloc(reused, small_layout) };

    let _guard = HeapLockGuard::acquire();
    // SAFETY: Access is serialized by HEAP_LOCK with local interrupts disabled.
    let state = unsafe { &mut *ptr::addr_of_mut!(HEAP_STATE) };
    if state.allocator.free_bytes() != state.allocator.total_bytes() {
        return Err(HeapTestError::FreeListNotCoalesced);
    }
    Ok(status_locked(state))
}

impl KernelHeapAllocator {
    unsafe fn allocate_locked(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Caller owns HEAP_LOCK and has disabled local interrupts.
        let state = unsafe { &mut *ptr::addr_of_mut!(HEAP_STATE) };
        if !state.initialized {
            return ptr::null_mut();
        }

        if let Some(allocation) = state.allocator.allocate(layout) {
            return allocation.as_ptr();
        }

        let required = match exhi_heap::minimum_region_size(layout) {
            Some(value) => value,
            None => return ptr::null_mut(),
        };
        let required_u64 = match u64::try_from(required) {
            Ok(value) => value,
            Err(_) => return ptr::null_mut(),
        };
        let pages_u64 = match required_u64.checked_add(PAGE_SIZE - 1) {
            Some(value) => value / PAGE_SIZE,
            None => return ptr::null_mut(),
        };
        let pages = match usize::try_from(pages_u64.max(1)) {
            Ok(value) => value,
            Err(_) => return ptr::null_mut(),
        };

        if grow_pages_locked(state, pages).is_err() {
            return ptr::null_mut();
        }

        state
            .allocator
            .allocate(layout)
            .map_or(ptr::null_mut(), |allocation| allocation.as_ptr())
    }
}

unsafe impl GlobalAlloc for KernelHeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = HeapLockGuard::acquire();
        // SAFETY: The lock guard serializes heap state and disables local IRQs.
        unsafe { self.allocate_locked(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        if pointer.is_null() {
            return;
        }
        let _guard = HeapLockGuard::acquire();
        // SAFETY: GlobalAlloc's caller must pass a live pointer from this heap.
        let state = unsafe { &mut *ptr::addr_of_mut!(HEAP_STATE) };
        if state.initialized {
            if let Some(pointer) = core::ptr::NonNull::new(pointer) {
                // SAFETY: This is the matching GlobalAlloc deallocation contract.
                unsafe { state.allocator.deallocate(pointer) };
            }
        }
    }
}

fn grow_pages_locked(state: &mut HeapState, pages: usize) -> Result<(), HeapInitError> {
    let start_page = state.mapped_pages;
    let end_page = start_page
        .checked_add(pages)
        .ok_or(HeapInitError::OutOfVirtualSpace)?;
    if pages == 0 || end_page > HEAP_MAX_PAGES {
        return Err(HeapInitError::OutOfVirtualSpace);
    }

    let mut mapped = 0usize;
    while mapped < pages {
        let frame = match memory::allocate_frame() {
            Ok(frame) => frame,
            Err(_) => {
                rollback_pages(start_page, mapped);
                return Err(HeapInitError::OutOfPhysicalFrames);
            }
        };
        if frame == 0 || frame % PAGE_SIZE != 0 || frame >= 0x1_0000_0000 {
            let _ = memory::release_frame(frame);
            rollback_pages(start_page, mapped);
            return Err(HeapInitError::InvalidPhysicalFrame);
        }

        let virtual_address = HEAP_BASE + ((start_page + mapped) as u64) * PAGE_SIZE;
        if let Err(error) = vmm::map_page(virtual_address, frame, PageFlags::kernel_data()) {
            let _ = memory::release_frame(frame);
            rollback_pages(start_page, mapped);
            return Err(HeapInitError::Vmm(error));
        }
        mapped += 1;
    }

    let region_start = (HEAP_BASE + (start_page as u64) * PAGE_SIZE) as *mut u8;
    let region_size = pages * PAGE_SIZE as usize;
    // SAFETY: Every page in this contiguous virtual range was mapped to a
    // dedicated allocated physical frame immediately above.
    if let Err(error) = unsafe { state.allocator.add_region(region_start, region_size) } {
        rollback_pages(start_page, mapped);
        return Err(HeapInitError::Heap(error));
    }

    state.mapped_pages = end_page;
    Ok(())
}

fn rollback_pages(start_page: usize, mapped: usize) {
    for offset in (0..mapped).rev() {
        let virtual_address = HEAP_BASE + ((start_page + offset) as u64) * PAGE_SIZE;
        if let Ok(frame) = vmm::unmap_page(virtual_address) {
            let _ = memory::release_frame(frame);
        }
    }
}

fn status_locked(state: &HeapState) -> HeapStatus {
    HeapStatus {
        virtual_base: HEAP_BASE,
        mapped_pages: state.mapped_pages,
        mapped_bytes: (state.mapped_pages as u64) * PAGE_SIZE,
        free_bytes: state.allocator.free_bytes(),
    }
}

struct HeapLockGuard {
    restore_interrupts: bool,
}

impl HeapLockGuard {
    fn acquire() -> Self {
        let flags = read_rflags();
        // SAFETY: CLI is privileged and this allocator only runs in ring 0.
        unsafe { asm!("cli", options(nomem, nostack)) };

        while HEAP_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spin_loop();
        }

        Self {
            restore_interrupts: flags & (1 << 9) != 0,
        }
    }
}

impl Drop for HeapLockGuard {
    fn drop(&mut self) {
        HEAP_LOCK.store(false, Ordering::Release);
        if self.restore_interrupts {
            // SAFETY: Restore the caller's prior IF state after releasing lock.
            unsafe { asm!("sti", options(nomem, nostack)) };
        }
    }
}

fn read_rflags() -> u64 {
    let flags: u64;
    // SAFETY: PUSHFQ/POP reads the current flags in ring 0.
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(preserves_flags)) };
    flags
}
