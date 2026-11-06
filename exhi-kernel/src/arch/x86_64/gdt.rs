use core::arch::{asm, global_asm};
use core::mem::size_of;

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;

const GDT_NULL: u64 = 0;
const GDT_KERNEL_CODE: u64 = 0x00AF_9A00_0000_FFFF;
const GDT_KERNEL_DATA: u64 = 0x00CF_9200_0000_FFFF;

#[repr(C, align(8))]
struct GlobalDescriptorTable([u64; 3]);

static GDT: GlobalDescriptorTable =
    GlobalDescriptorTable([GDT_NULL, GDT_KERNEL_CODE, GDT_KERNEL_DATA]);

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

unsafe extern "sysv64" {
    fn exhi_load_gdt(pointer: *const DescriptorTablePointer);
}

/// Installs ExhiOS's kernel GDT and reloads the active code/data selectors.
///
/// The bootloader enters the kernel in 64-bit mode with paging enabled. The
/// GDT lives in the kernel's identity-mapped image, so its address remains
/// valid while the current UEFI-created page tables are still active.
pub fn initialize() {
    let pointer = DescriptorTablePointer {
        limit: (size_of::<GlobalDescriptorTable>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u64,
    };

    // SAFETY: The descriptor table is static, correctly sized, and mapped at
    // its linked physical address. The assembly helper loads this GDTR and
    // reloads CS, DS, ES, and SS with selectors present in the table.
    unsafe {
        exhi_load_gdt(&pointer);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentSelectors {
    pub cs: u16,
    pub ds: u16,
    pub es: u16,
    pub ss: u16,
}

impl SegmentSelectors {
    pub const fn is_valid(self) -> bool {
        self.cs == KERNEL_CODE_SELECTOR
            && self.ds == KERNEL_DATA_SELECTOR
            && self.es == KERNEL_DATA_SELECTOR
            && self.ss == KERNEL_DATA_SELECTOR
    }
}

/// Reads the currently loaded segment selectors for the GDT smoke test.
pub fn read_segment_selectors() -> SegmentSelectors {
    let cs: u16;
    let ds: u16;
    let es: u16;
    let ss: u16;

    unsafe {
        asm!("mov {0:x}, cs", out(reg) cs, options(nomem, nostack, preserves_flags));
        asm!("mov {0:x}, ds", out(reg) ds, options(nomem, nostack, preserves_flags));
        asm!("mov {0:x}, es", out(reg) es, options(nomem, nostack, preserves_flags));
        asm!("mov {0:x}, ss", out(reg) ss, options(nomem, nostack, preserves_flags));
    }

    SegmentSelectors { cs, ds, es, ss }
}

global_asm!(
    r#"
    .section .text
    .global exhi_load_gdt
    .type exhi_load_gdt, @function
exhi_load_gdt:
    lgdt [rdi]

    push 0x08
    lea rax, [rip + 1f]
    push rax
    retfq

1:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    ret
    "#
);
