#![no_std]

use core::alloc::Layout;
use core::mem::{align_of, size_of};
use core::ptr::{self, NonNull};

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

#[repr(C)]
struct AllocationHeader {
    start: usize,
    size: usize,
}

/// A first-fit free-list heap over caller-provided memory regions.
///
/// Region memory must remain mapped and exclusively owned by this heap. The
/// caller must serialize all mutations and deallocations must use live pointers
/// returned by allocate.
pub struct Heap {
    head: *mut FreeBlock,
    total_bytes: usize,
}

impl Heap {
    pub const fn new() -> Self {
        Self { head: ptr::null_mut(), total_bytes: 0 }
    }

    /// Adds a new non-overlapping region to the heap.
    ///
    /// # Safety
    /// The range must be writable, mapped memory, not already owned by another
    /// allocator, and must remain mapped while allocations from it are live.
    pub unsafe fn add_region(&mut self, start: *mut u8, size: usize) -> Result<(), HeapError> {
        if start.is_null() || size < size_of::<FreeBlock>() {
            return Err(HeapError::InvalidRegion);
        }
        let raw_start = start as usize;
        let raw_end = raw_start.checked_add(size).ok_or(HeapError::AddressOverflow)?;
        let alignment = align_of::<FreeBlock>();
        let region_start = align_up(raw_start, alignment).ok_or(HeapError::AddressOverflow)?;
        let region_end = raw_end & !(alignment - 1);
        if region_end <= region_start || region_end - region_start < size_of::<FreeBlock>() {
            return Err(HeapError::InvalidRegion);
        }
        let region_size = region_end - region_start;
        self.insert_free_region(region_start, region_size)?;
        self.total_bytes = self.total_bytes.checked_add(region_size).ok_or(HeapError::AddressOverflow)?;
        Ok(())
    }

    /// Allocates a block satisfying layout, or returns None when no region fits.
    pub fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        let requested_size = layout.size().max(1);
        let alignment = layout.align().max(align_of::<AllocationHeader>());
        let header_size = size_of::<AllocationHeader>();
        let node_size = size_of::<FreeBlock>();
        let node_alignment = align_of::<FreeBlock>();
        let mut previous: *mut FreeBlock = ptr::null_mut();
        let mut current = self.head;

        while !current.is_null() {
            let block_start = current as usize;
            // SAFETY: Every node belongs to a registered writable heap region.
            let block_size = unsafe { (*current).size };
            let block_end = block_start.checked_add(block_size)?;
            let next = unsafe { (*current).next };

            let first_payload = block_start.checked_add(header_size)?;
            let mut user_start = align_up(first_payload, alignment)?;
            let mut allocation_start = user_start.checked_sub(header_size)?;
            let mut prefix = allocation_start.checked_sub(block_start)?;

            // A prefix smaller than FreeBlock could not be represented.
            if prefix != 0 && prefix < node_size {
                user_start = align_up(user_start.checked_add(node_size)?, alignment)?;
                allocation_start = user_start.checked_sub(header_size)?;
                prefix = allocation_start.checked_sub(block_start)?;
            }

            let payload_end = user_start.checked_add(requested_size)?;
            let mut allocation_end = align_up(payload_end, node_alignment)?;
            if allocation_end > block_end || allocation_start < block_start {
                previous = current;
                current = next;
                continue;
            }

            let mut suffix = block_end - allocation_end;
            if suffix != 0 && suffix < node_size {
                allocation_end = block_end;
                suffix = 0;
            }
            let allocation_size = allocation_end.checked_sub(allocation_start)?;
            if allocation_size < header_size.checked_add(requested_size)? {
                previous = current;
                current = next;
                continue;
            }

            if prefix != 0 {
                // Keep the original node as the prefix.
                unsafe {
                    (*current).size = prefix;
                    if suffix != 0 {
                        let suffix_node = allocation_end as *mut FreeBlock;
                        ptr::write(suffix_node, FreeBlock { size: suffix, next });
                        (*current).next = suffix_node;
                    } else {
                        (*current).next = next;
                    }
                }
            } else if suffix != 0 {
                let suffix_node = allocation_end as *mut FreeBlock;
                unsafe {
                    ptr::write(suffix_node, FreeBlock { size: suffix, next });
                    if previous.is_null() {
                        self.head = suffix_node;
                    } else {
                        (*previous).next = suffix_node;
                    }
                }
            } else if previous.is_null() {
                self.head = next;
            } else {
                unsafe { (*previous).next = next };
            }

            let user_pointer = user_start as *mut u8;
            let header_pointer = (user_start - header_size) as *mut AllocationHeader;
            // SAFETY: Header is inside the reserved allocation and aligned.
            unsafe {
                ptr::write(header_pointer, AllocationHeader {
                    start: allocation_start,
                    size: allocation_size,
                });
            }
            return NonNull::new(user_pointer);
        }
        None
    }

    /// Returns a previously allocated block to the free list.
    ///
    /// # Safety
    /// Pointer must be live, returned by this heap, and not previously freed.
    pub unsafe fn deallocate(&mut self, pointer: NonNull<u8>) {
        let header_address = (pointer.as_ptr() as usize)
            .checked_sub(size_of::<AllocationHeader>())
            .expect("heap allocation header underflow");
        // SAFETY: Caller guarantees this is a live allocation from this heap.
        let header = unsafe { ptr::read(header_address as *const AllocationHeader) };
        self.insert_free_region(header.start, header.size)
            .expect("heap free-list invariant violated");
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn free_bytes(&self) -> usize {
        let mut total = 0usize;
        let mut current = self.head;
        while !current.is_null() {
            // SAFETY: Free-list nodes are valid and linked by this heap.
            unsafe {
                total = total.saturating_add((*current).size);
                current = (*current).next;
            }
        }
        total
    }

    fn insert_free_region(&mut self, start: usize, size: usize) -> Result<(), HeapError> {
        if size < size_of::<FreeBlock>() || start % align_of::<FreeBlock>() != 0 {
            return Err(HeapError::InvalidRegion);
        }
        let end = start.checked_add(size).ok_or(HeapError::AddressOverflow)?;
        let mut previous: *mut FreeBlock = ptr::null_mut();
        let mut current = self.head;
        while !current.is_null() && (current as usize) < start {
            previous = current;
            current = unsafe { (*current).next };
        }

        if !previous.is_null() {
            let previous_end = (previous as usize)
                .checked_add(unsafe { (*previous).size })
                .ok_or(HeapError::AddressOverflow)?;
            if previous_end > start {
                return Err(HeapError::OverlappingRegion);
            }
        }
        if !current.is_null() && end > current as usize {
            return Err(HeapError::OverlappingRegion);
        }

        let new_node = start as *mut FreeBlock;
        unsafe { ptr::write(new_node, FreeBlock { size, next: current }) };
        let mut merged = new_node;

        if !current.is_null() && end == current as usize {
            let current_size = unsafe { (*current).size };
            let current_next = unsafe { (*current).next };
            unsafe {
                (*merged).size = size.checked_add(current_size).ok_or(HeapError::AddressOverflow)?;
                (*merged).next = current_next;
            }
        }

        if previous.is_null() {
            self.head = merged;
        } else {
            let previous_end = (previous as usize)
                .checked_add(unsafe { (*previous).size })
                .ok_or(HeapError::AddressOverflow)?;
            if previous_end == merged as usize {
                let merged_size = unsafe { (*merged).size };
                let merged_next = unsafe { (*merged).next };
                unsafe {
                    (*previous).size = (*previous).size.checked_add(merged_size)
                        .ok_or(HeapError::AddressOverflow)?;
                    (*previous).next = merged_next;
                }
                merged = previous;
            } else {
                unsafe { (*previous).next = merged };
            }
        }

        let next = unsafe { (*merged).next };
        if !next.is_null() {
            let merged_end = (merged as usize)
                .checked_add(unsafe { (*merged).size })
                .ok_or(HeapError::AddressOverflow)?;
            if merged_end == next as usize {
                let next_size = unsafe { (*next).size };
                let next_next = unsafe { (*next).next };
                unsafe {
                    (*merged).size = (*merged).size.checked_add(next_size)
                        .ok_or(HeapError::AddressOverflow)?;
                    (*merged).next = next_next;
                }
            }
        }
        Ok(())
    }
}

impl Default for Heap {
    fn default() -> Self { Self::new() }
}

/// Conservative contiguous space required, including alignment and metadata.
pub fn minimum_region_size(layout: Layout) -> Option<usize> {
    layout.size().max(1)
        .checked_add(layout.align())?
        .checked_add(size_of::<AllocationHeader>())?
        .checked_add(size_of::<FreeBlock>())
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value.checked_add(alignment - 1).map(|v| v & !(alignment - 1))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapError {
    InvalidRegion,
    AddressOverflow,
    OverlappingRegion,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{alloc, dealloc as system_dealloc};

    fn test_heap(size: usize) -> (Heap, *mut u8, Layout) {
        let layout = Layout::from_size_align(size, 4096).unwrap();
        let backing = unsafe { alloc(layout) };
        assert!(!backing.is_null());
        let mut heap = Heap::new();
        unsafe { heap.add_region(backing, size).unwrap() };
        (heap, backing, layout)
    }

    #[test]
    fn allocations_honor_size_and_alignment() {
        let (mut heap, backing, backing_layout) = test_heap(128 * 1024);
        let layout_a = Layout::from_size_align(127, 64).unwrap();
        let layout_b = Layout::from_size_align(8193, 4096).unwrap();
        let a = heap.allocate(layout_a).unwrap();
        let b = heap.allocate(layout_b).unwrap();
        assert_eq!(a.as_ptr() as usize % 64, 0);
        assert_eq!(b.as_ptr() as usize % 4096, 0);
        assert_ne!(a, b);
        unsafe {
            a.as_ptr().write_bytes(0xA5, layout_a.size());
            b.as_ptr().write_bytes(0x5A, layout_b.size());
            assert_eq!(*a.as_ptr(), 0xA5);
            assert_eq!(*b.as_ptr(), 0x5A);
            heap.deallocate(a);
            heap.deallocate(b);
        }
        assert_eq!(heap.free_bytes(), heap.total_bytes());
        unsafe { system_dealloc(backing, backing_layout) };
    }

    #[test]
    fn adjacent_regions_coalesce_into_one_large_region() {
        let backing_layout = Layout::from_size_align(128 * 1024, 4096).unwrap();
        let backing = unsafe { alloc(backing_layout) };
        assert!(!backing.is_null());
        let mut heap = Heap::new();
        unsafe {
            heap.add_region(backing, 64 * 1024).unwrap();
            heap.add_region(backing.add(64 * 1024), 64 * 1024).unwrap();
        }
        let large = heap.allocate(Layout::from_size_align(100 * 1024, 16).unwrap()).unwrap();
        unsafe { heap.deallocate(large) };
        assert_eq!(heap.free_bytes(), heap.total_bytes());
        unsafe { system_dealloc(backing, backing_layout) };
    }

    #[test]
    fn freeing_blocks_coalesces_and_allows_reuse() {
        let (mut heap, backing, backing_layout) = test_heap(64 * 1024);
        let layout = Layout::from_size_align(512, 32).unwrap();
        let first = heap.allocate(layout).unwrap();
        let second = heap.allocate(layout).unwrap();
        unsafe {
            heap.deallocate(first);
            heap.deallocate(second);
        }
        let reused = heap.allocate(layout).unwrap();
        assert_eq!(reused, first);
        unsafe { heap.deallocate(reused) };
        assert_eq!(heap.free_bytes(), heap.total_bytes());
        unsafe { system_dealloc(backing, backing_layout) };
    }

    #[test]
    fn rejects_overlapping_regions() {
        let (mut heap, backing, backing_layout) = test_heap(16 * 1024);
        let result = unsafe { heap.add_region(backing.add(4096), 4096) };
        assert_eq!(result, Err(HeapError::OverlappingRegion));
        unsafe { system_dealloc(backing, backing_layout) };
    }
}
