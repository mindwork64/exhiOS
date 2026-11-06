#![no_main]
#![no_std]

use uefi::prelude::*;

#[entry]
fn main() -> Status {
    let _ = uefi::helpers::init();

    uefi::println!("ExhiOS Bootloader 0.1");
    uefi::println!("UEFI application entry point reached.");
    uefi::println!("P0.1: firmware entry smoke test.");
    uefi::println!("ELF loading and kernel handoff are not implemented yet.");

    Status::SUCCESS
}
