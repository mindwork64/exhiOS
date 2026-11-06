use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::ptr;

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
pub const TSS_SELECTOR: u16 = 0x18;

const GDT_NULL: u64 = 0;
const GDT_KERNEL_CODE: u64 = 0x00AF_9A00_0000_FFFF;
const GDT_KERNEL_DATA: u64 = 0x00CF_9200_0000_FFFF;
const TSS_DESCRIPTOR_TYPE: u64 = 0x89;
const DOUBLE_FAULT_IST_STACK_SIZE: usize = 16 * 1024;
const PAGE_SIZE: u64 = 4096;
const PHYSICAL_LIMIT: u64 = 0x1_0000_0000;

#[repr(C, align(16))]
struct GlobalDescriptorTable([u64; 5]);

static mut GDT: GlobalDescriptorTable =
    GlobalDescriptorTable([GDT_NULL, GDT_KERNEL_CODE, GDT_KERNEL_DATA, 0, 0]);

#[repr(C, packed)]
struct TaskStateSegment {
    reserved0: u32,
    rsp: [u64; 3],
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap_base: u16,
}

impl TaskStateSegment {
    const EMPTY: Self = Self {
        reserved0: 0,
        rsp: [0; 3],
        reserved1: 0,
        ist: [0; 7],
        reserved2: 0,
        reserved3: 0,
        iomap_base: size_of::<Self>() as u16,
    };
}

const _: [(); 104] = [(); size_of::<TaskStateSegment>()];

#[repr(C, align(16))]
struct TaskStateSegmentStorage(TaskStateSegment);

static mut TSS: TaskStateSegmentStorage =
    TaskStateSegmentStorage(TaskStateSegment::EMPTY);

#[repr(C, align(4096))]
struct InterruptStack([u8; DOUBLE_FAULT_IST_STACK_SIZE]);

static mut DOUBLE_FAULT_IST_STACK: InterruptStack =
    InterruptStack([0; DOUBLE_FAULT_IST_STACK_SIZE]);

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

unsafe extern "sysv64" {
    fn exhi_load_gdt(pointer: *const DescriptorTablePointer);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TssStatus {
    pub selector: u16,
    pub task_register: u16,
    pub tss_base: u64,
    pub double_fault_ist_bottom: u64,
    pub double_fault_ist_top: u64,
    pub valid: bool,
}

/// Installs the kernel GDT, a 64-bit TSS and the dedicated double-fault IST1 stack.
///
/// The TSS RSP0 is deliberately left unset until per-task kernel stacks are
/// introduced. IST1 is independent and is ready for the current fatal #DF path.
pub fn initialize() -> TssStatus {
    let stack_bottom = ptr::addr_of!(DOUBLE_FAULT_IST_STACK) as u64;
    let stack_top = stack_bottom + DOUBLE_FAULT_IST_STACK_SIZE as u64;
    let tss_base = ptr::addr_of!(TSS) as u64;

    // Validate the static kernel objects before publishing their addresses to
    // the processor. They live in the loaded, identity-mapped kernel image.
    assert!(stack_bottom != 0);
    assert!(stack_top < PHYSICAL_LIMIT);
    assert!(stack_bottom % PAGE_SIZE == 0);
    assert!(stack_top % 16 == 0);
    assert!(tss_base != 0);
    assert!(tss_base + size_of::<TaskStateSegment>() as u64 <= PHYSICAL_LIMIT);

    let mut tss = TaskStateSegment::EMPTY;
    tss.ist[0] = stack_top;
    // iomap_base is beyond the TSS limit: no I/O permission bitmap is present.
    tss.iomap_base = size_of::<TaskStateSegment>() as u16;

    // SAFETY: The TSS storage is static and aligned; the complete packed value
    // is written before its descriptor is loaded with LGDT/LTR.
    unsafe {
        ptr::write(ptr::addr_of_mut!(TSS.0), tss);
    }

    let tss_limit = (size_of::<TaskStateSegment>() - 1) as u64;
    let descriptor_low = (tss_limit & 0xFFFF)
        | ((tss_base & 0x00FF_FFFF) << 16)
        | (TSS_DESCRIPTOR_TYPE << 40)
        | (((tss_limit >> 16) & 0xF) << 48)
        | (((tss_base >> 24) & 0xFF) << 56);
    let descriptor_high = tss_base >> 32;

    // SAFETY: The GDT is writable static storage. Slots 3 and 4 are reserved
    // for the two-qword 64-bit TSS descriptor and are initialized before LGDT.
    unsafe {
        let entries = ptr::addr_of_mut!(GDT.0).cast::<u64>();
        ptr::write_volatile(entries.add(3), descriptor_low);
        ptr::write_volatile(entries.add(4), descriptor_high);
    }

    let pointer = DescriptorTablePointer {
        limit: (size_of::<GlobalDescriptorTable>() - 1) as u16,
        base: ptr::addr_of!(GDT) as u64,
    };

    // SAFETY: GDT, TSS and IST stack are static, fully initialized and mapped.
    unsafe {
        exhi_load_gdt(&pointer);
        asm!(
            "ltr {selector:x}",
            selector = in(reg) TSS_SELECTOR,
            options(nostack, preserves_flags)
        );
    }

    let task_register: u16;
    // SAFETY: STR is valid in ring 0 and stores the active task-register selector.
    unsafe {
        asm!(
            "str {selector:x}",
            selector = lateout(reg) task_register,
            options(nomem, nostack, preserves_flags)
        );
    }

    TssStatus {
        selector: TSS_SELECTOR,
        task_register,
        tss_base,
        double_fault_ist_bottom: stack_bottom,
        double_fault_ist_top: stack_top,
        valid: task_register == TSS_SELECTOR
            && stack_top - stack_bottom == DOUBLE_FAULT_IST_STACK_SIZE as u64
            && stack_top % 16 == 0,
    }
}

/// Returns whether an exception frame was placed inside the dedicated IST1 stack.
pub fn double_fault_ist_contains(stack_pointer: u64) -> bool {
    let bottom = ptr::addr_of!(DOUBLE_FAULT_IST_STACK) as u64;
    let top = bottom + DOUBLE_FAULT_IST_STACK_SIZE as u64;
    (bottom..top).contains(&stack_pointer)
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
