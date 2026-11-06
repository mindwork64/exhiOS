#![no_main]
#![no_std]

use core::time::Duration;

use exhi_elf::{ElfFile, PT_LOAD};
use uefi::prelude::*;
use uefi::{boot, cstr16};
use uefi::fs::FileSystem;

const KERNEL_PATH: &uefi::CStr16 = cstr16!("\\exhi-kernel.elf");
const MAX_KERNEL_SIZE: u64 = 64 * 1024 * 1024;

#[entry]
fn main() -> Status {
    let _ = uefi::helpers::init();

    uefi::println!("ExhiOS Bootloader 0.2");
    uefi::println!("UEFI application entry point reached.");
    uefi::println!("P0.2: loading and validating the kernel ELF64 image.");

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

    for (index, header) in elf.program_headers().enumerate() {
        if header.kind == PT_LOAD {
            uefi::println!(
                "PT_LOAD #{index}: vaddr={:#018x}, file={} bytes, memory={} bytes, flags={:#x}",
                header.virtual_address,
                header.file_size,
                header.memory_size,
                header.flags
            );
        }
    }

    uefi::println!("");
    uefi::println!("P0.2 complete: kernel image located, read and validated.");
    uefi::println!("No segments have been loaded into physical memory yet.");
    finish(Status::SUCCESS)
}

fn finish(status: Status) -> Status {
    uefi::println!("Returning to UEFI firmware in 15 seconds.");
    boot::stall(Duration::from_secs(15));
    status
}
