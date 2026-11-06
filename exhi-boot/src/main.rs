#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::{ptr, slice, time::Duration};
use core::ptr::NonNull;

use exhi_abi::{BootInfo, FramebufferInfo};
use exhi_elf::{ElfFile, ProgramHeader, PT_LOAD};
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};
use uefi::{boot, cstr16};
use uefi::fs::FileSystem;

const KERNEL_PATH: &uefi::CStr16 = cstr16!("\\exhi-kernel.elf");
const MAX_KERNEL_SIZE: u64 = 64 * 1024 * 1024;
const PAGE_SIZE: u64 = boot::PAGE_SIZE as u64;

struct AllocatedPages {
    base: NonNull<u8>,
    page_count: usize,
}

#[entry]
fn main() -> Status {
    let _ = uefi::helpers::init();

    uefi::println!("ExhiOS Bootloader 0.3");
    uefi::println!("UEFI application entry point reached.");
    uefi::println!("P0.3: loading kernel segments into physical memory.");

    let image_fs = match boot::get_image_file_system(boot::image_handle()) {
        Ok(file_system) => file_system,
        Err(error) => {
            uefi::println!("ERROR: cannot access the boot volume: {error:?}");
            return finish(Status::NOT_FOUND);
        }
    };

    let mut file_system = FileSystem::new(image_fs);

    let metadata = match file_system.metadata(KERNEL_PATH) {
        Ok(metadata) => metadata,
        Err(error) => {
            uefi::println!("ERROR: cannot find \\exhi-kernel.elf: {error:?}");
            return finish(Status::NOT_FOUND);
        }
    };

    if metadata.is_directory() {
        uefi::println!("ERROR: \\exhi-kernel.elf is a directory.");
        return finish(Status::LOAD_ERROR);
    }

    let expected_size = metadata.file_size();
    if expected_size == 0 || expected_size > MAX_KERNEL_SIZE {
        uefi::println!(
            "ERROR: kernel file size is outside the allowed range: {expected_size} bytes."
        );
        return finish(Status::LOAD_ERROR);
    }

    uefi::println!("Kernel file found: {expected_size} bytes.");

    let kernel_bytes = match file_system.read(KERNEL_PATH) {
        Ok(bytes) => bytes,
        Err(error) => {
            uefi::println!("ERROR: failed to read kernel ELF: {error:?}");
            return finish(Status::DEVICE_ERROR);
        }
    };

    if kernel_bytes.len() as u64 != expected_size {
        uefi::println!(
            "ERROR: short or inconsistent kernel read (expected {expected_size}, got {}).",
            kernel_bytes.len()
        );
        return finish(Status::LOAD_ERROR);
    }

    uefi::println!("Kernel file read successfully.");

    let elf = match ElfFile::parse(&kernel_bytes) {
        Ok(elf) => elf,
        Err(error) => {
            uefi::println!("ERROR: kernel ELF validation failed: {error:?}");
            return finish(Status::LOAD_ERROR);
        }
    };

    uefi::println!("ELF64 validation: OK");
    uefi::println!("Entry point: {:#018x}", elf.entry_point());
    uefi::println!("Program headers: {}", elf.program_header_count());

    let mut allocations = Vec::new();
    if allocations.try_reserve(elf.program_header_count() + 1).is_err() {
        uefi::println!("ERROR: cannot reserve allocation tracking storage.");
        return finish(Status::OUT_OF_RESOURCES);
    }

    if let Err(status) = load_kernel_segments(&elf, &kernel_bytes, &mut allocations) {
        uefi::println!("ERROR: kernel segment loading failed: {status:?}");
        release_allocations(&mut allocations);
        return finish(status);
    }

    let framebuffer = collect_framebuffer_info();
    if framebuffer.physical_address != 0 {
        uefi::println!(
            "GOP framebuffer: {:#018x}, {}x{}, stride={}, format={}",
            framebuffer.physical_address,
            framebuffer.width,
            framebuffer.height,
            framebuffer.pixels_per_scan_line,
            framebuffer.pixel_format
        );
    } else {
        uefi::println!("GOP framebuffer: unavailable; continuing without it.");
    }

    let acpi_rsdp_address = find_acpi_rsdp();
    if acpi_rsdp_address != 0 {
        uefi::println!("ACPI RSDP: {acpi_rsdp_address:#018x}");
    } else {
        uefi::println!("ACPI RSDP: not found.");
    }

    let mut boot_info = BootInfo::empty();
    boot_info.framebuffer = framebuffer;
    boot_info.acpi_rsdp_address = acpi_rsdp_address;

    let boot_info_page = match boot::allocate_pages(
        boot::AllocateType::AnyPages,
        boot::MemoryType::LOADER_DATA,
        1,
    ) {
        Ok(page) => page,
        Err(error) => {
            uefi::println!("ERROR: cannot allocate BootInfo page: {error:?}");
            release_allocations(&mut allocations);
            return finish(Status::OUT_OF_RESOURCES);
        }
    };

    // SAFETY: The allocated page is writable, correctly aligned, and large
    // enough for BootInfo. The allocation is retained until cleanup or handoff.
    unsafe {
        boot_info_page.as_ptr().cast::<BootInfo>().write(boot_info);
    }

    allocations.push(AllocatedPages {
        base: boot_info_page,
        page_count: 1,
    });

    uefi::println!(
        "BootInfo prepared at {:#018x}.",
        boot_info_page.as_ptr() as u64
    );
    uefi::println!("BootInfo framebuffer and ACPI fields populated.");
    uefi::println!("BootInfo memory map will be finalized in P0.4.");

    uefi::println!("");
    uefi::println!("P0.3 preparation complete.");
    uefi::println!("Kernel segments are loaded at their fixed physical addresses.");
    uefi::println!("Segment contents and zero-filled tails were verified.");
    uefi::println!("Kernel has not been entered; UEFI Boot Services remain active.");

    release_allocations(&mut allocations);
    finish(Status::SUCCESS)
}

fn load_kernel_segments(
    elf: &ElfFile<'_>,
    image: &[u8],
    allocations: &mut Vec<AllocatedPages>,
) -> Result<(), Status> {
    for (index, header) in elf.program_headers().enumerate() {
        if header.kind != PT_LOAD {
            continue;
        }

        load_one_segment(index, header, image, allocations)?;
    }

    Ok(())
}

fn load_one_segment(
    index: usize,
    header: ProgramHeader,
    image: &[u8],
    allocations: &mut Vec<AllocatedPages>,
) -> Result<(), Status> {
    if header.physical_address != header.virtual_address {
        uefi::println!(
            "PT_LOAD #{index}: physical and virtual addresses differ; this loader requires identity loading."
        );
        return Err(Status::LOAD_ERROR);
    }

    let physical_address = header.physical_address;
    let memory_end = physical_address
        .checked_add(header.memory_size)
        .ok_or(Status::LOAD_ERROR)?;

    let page_start = physical_address & !(PAGE_SIZE - 1);
    let page_end = memory_end
        .checked_add(PAGE_SIZE - 1)
        .ok_or(Status::LOAD_ERROR)?
        & !(PAGE_SIZE - 1);

    let segment_offset =
        usize::try_from(physical_address - page_start).map_err(|_| Status::LOAD_ERROR)?;
    let page_count_u64 = (page_end - page_start) / PAGE_SIZE;
    let page_count = usize::try_from(page_count_u64).map_err(|_| Status::LOAD_ERROR)?;
    let allocation_size = page_count
        .checked_mul(boot::PAGE_SIZE)
        .ok_or(Status::LOAD_ERROR)?;

    let file_offset = usize::try_from(header.offset).map_err(|_| Status::LOAD_ERROR)?;
    let file_size = usize::try_from(header.file_size).map_err(|_| Status::LOAD_ERROR)?;
    let memory_size = usize::try_from(header.memory_size).map_err(|_| Status::LOAD_ERROR)?;
    let file_end = file_offset
        .checked_add(file_size)
        .ok_or(Status::LOAD_ERROR)?;
    let source = image.get(file_offset..file_end).ok_or(Status::LOAD_ERROR)?;

    uefi::println!(
        "Reserving PT_LOAD #{index}: physical={:#018x}, pages={}, bytes={}",
        page_start,
        page_count,
        allocation_size
    );

    let destination_pages = match boot::allocate_pages(
        boot::AllocateType::Address(page_start),
        boot::MemoryType::LOADER_DATA,
        page_count,
    ) {
        Ok(pages) => pages,
        Err(error) => {
            uefi::println!(
                "ERROR: UEFI could not reserve PT_LOAD #{index} at {page_start:#018x}: {error:?}"
            );
            return Err(Status::OUT_OF_RESOURCES);
        }
    };

    if destination_pages.as_ptr() as u64 != page_start {
        uefi::println!(
            "ERROR: UEFI returned an unexpected address for PT_LOAD #{index}."
        );
        // SAFETY: This allocation was just returned by allocate_pages with
        // exactly this page count.
        let _ = unsafe { boot::free_pages(destination_pages, page_count) };
        return Err(Status::LOAD_ERROR);
    }

    let destination = unsafe { destination_pages.as_ptr().add(segment_offset) };

    // Track the allocation before any later verification can fail.
    allocations.push(AllocatedPages {
        base: destination_pages,
        page_count,
    });

    // SAFETY:
    // - The entire destination allocation consists of writable UEFI pages.
    // - The source range was bounds-checked against the ELF file above.
    // - The ELF parser proved file_size <= memory_size and checked address ranges.
    // - The parser rejects page-overlapping PT_LOAD segments.
    unsafe {
        ptr::write_bytes(destination_pages.as_ptr(), 0, allocation_size);
        ptr::copy_nonoverlapping(source.as_ptr(), destination, file_size);
        ptr::write_bytes(
            destination.add(file_size),
            0,
            memory_size - file_size,
        );
    }

    // Verify the bytes copied from the file and the zero-filled memory tail.
    // SAFETY: Both slices refer to ranges inside the allocation above.
    let copied = unsafe { slice::from_raw_parts(destination, file_size) };
    if copied != source {
        uefi::println!("ERROR: PT_LOAD #{index} memory verification failed.");
        return Err(Status::LOAD_ERROR);
    }

    let zero_tail_size = memory_size - file_size;
    if zero_tail_size != 0 {
        // SAFETY: The tail lies inside the allocated segment memory range.
        let zero_tail =
            unsafe { slice::from_raw_parts(destination.add(file_size), zero_tail_size) };
        if zero_tail.iter().any(|byte| *byte != 0) {
            uefi::println!("ERROR: PT_LOAD #{index} zero-fill verification failed.");
            return Err(Status::LOAD_ERROR);
        }
    }

    uefi::println!(
        "PT_LOAD #{index}: loaded at {physical_address:#018x}; file={} bytes; memory={} bytes; zero-tail={zero_tail_size} bytes; verified.",
        header.file_size,
        header.memory_size
    );

    Ok(())
}

fn collect_framebuffer_info() -> FramebufferInfo {
    let handle = match boot::get_handle_for_protocol::<GraphicsOutput>() {
        Ok(handle) => handle,
        Err(_) => return FramebufferInfo::empty(),
    };

    // SAFETY: This application only reads the GOP mode and framebuffer
    // metadata. It does not change the video mode or write to the framebuffer,
    // so using the non-exclusive GetProtocol access does not conflict with the
    // firmware text console.
    let mut graphics = match unsafe {
        boot::open_protocol::<GraphicsOutput>(
            boot::OpenProtocolParams {
                handle,
                agent: boot::image_handle(),
                controller: None,
            },
            boot::OpenProtocolAttributes::GetProtocol,
        )
    } {
        Ok(graphics) => graphics,
        Err(_) => return FramebufferInfo::empty(),
    };

    let mode = graphics.current_mode_info();
    if mode.pixel_format() == PixelFormat::BltOnly {
        return FramebufferInfo::empty();
    }

    let (width, height) = mode.resolution();
    let stride = mode.stride();
    let mut framebuffer = graphics.frame_buffer();

    let (Ok(width), Ok(height), Ok(stride)) = (
        u32::try_from(width),
        u32::try_from(height),
        u32::try_from(stride),
    ) else {
        return FramebufferInfo::empty();
    };

    FramebufferInfo {
        physical_address: framebuffer.as_mut_ptr() as u64,
        buffer_size: framebuffer.size() as u64,
        width,
        height,
        pixels_per_scan_line: stride,
        pixel_format: mode.pixel_format() as u32,
    }
}

fn find_acpi_rsdp() -> u64 {
    uefi::system::with_config_table(|entries| {
        entries
            .iter()
            .find(|entry| entry.guid == uefi::table::cfg::ConfigTableEntry::ACPI2_GUID)
            .or_else(|| {
                entries
                    .iter()
                    .find(|entry| entry.guid == uefi::table::cfg::ConfigTableEntry::ACPI_GUID)
            })
            .map(|entry| entry.address as u64)
            .unwrap_or(0)
    })
}

fn release_allocations(allocations: &mut Vec<AllocatedPages>) {
    while let Some(allocation) = allocations.pop() {
        // SAFETY: Every entry is a matching allocation returned by allocate_pages
        // and no pointers into these pages are used after this call.
        if let Err(error) =
            unsafe { boot::free_pages(allocation.base, allocation.page_count) }
        {
            uefi::println!("WARNING: could not release a temporary page allocation: {error:?}");
        }
    }
}

fn finish(status: Status) -> Status {
    uefi::println!("Returning to UEFI firmware in 15 seconds.");
    boot::stall(Duration::from_secs(15));
    status
}
