#![no_main]
#![no_std]

use core::time::Duration;
use uefi::prelude::*;

#[entry]
fn main() -> Status {
    let _ = uefi::helpers::init();

    uefi::println!("ExhiOS Bootloader 0.1");
    uefi::println!("UEFI application entry point reached.");
    uefi::println!("P0.1: firmware entry smoke test.");
    uefi::println!("ELF loading and kernel handoff are not implemented yet.");
    uefi::println!("");
    uefi::println!("This diagnostic remains visible for 15 seconds.");

    // Returning from a UEFI application immediately hands control back to
    // firmware, which can make the diagnostic look like a brief flash.
    uefi::boot::stall(Duration::from_secs(15));

    Status::SUCCESS
}
