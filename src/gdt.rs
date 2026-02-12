use core::arch::{asm, global_asm};

const GDT_ENTRIES: usize = 6;

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
const USER_CODE_SELECTOR: u16 = 0x1B;
const USER_DATA_SELECTOR: u16 = 0x23;
const TSS_SELECTOR: u16 = 0x28;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

impl GdtEntry {
    const fn null() -> Self {
        Self {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            granularity: 0,
            base_high: 0,
        }
    }

    const fn new(base: u32, limit: u32, access: u8, granularity: u8) -> Self {
        Self {
            limit_low: (limit & 0xFFFF) as u16,
            base_low: (base & 0xFFFF) as u16,
            base_mid: ((base >> 16) & 0xFF) as u8,
            access,
            granularity: (((limit >> 16) & 0x0F) as u8) | (granularity & 0xF0),
            base_high: ((base >> 24) & 0xFF) as u8,
        }
    }
}

#[repr(C, packed)]
struct GdtDescriptor {
    limit: u16,
    base: u32,
}

#[repr(C, packed)]
struct TaskStateSegment {
    prev_tss: u32,
    esp0: u32,
    ss0: u32,
    esp1: u32,
    ss1: u32,
    esp2: u32,
    ss2: u32,
    cr3: u32,
    eip: u32,
    eflags: u32,
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esp: u32,
    ebp: u32,
    esi: u32,
    edi: u32,
    es: u32,
    cs: u32,
    ss: u32,
    ds: u32,
    fs: u32,
    gs: u32,
    ldt: u32,
    trap: u16,
    iomap_base: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            prev_tss: 0,
            esp0: 0,
            ss0: KERNEL_DATA_SELECTOR as u32,
            esp1: 0,
            ss1: 0,
            esp2: 0,
            ss2: 0,
            cr3: 0,
            eip: 0,
            eflags: 0,
            eax: 0,
            ecx: 0,
            edx: 0,
            ebx: 0,
            esp: 0,
            ebp: 0,
            esi: 0,
            edi: 0,
            es: 0,
            cs: 0,
            ss: 0,
            ds: 0,
            fs: 0,
            gs: 0,
            ldt: 0,
            trap: 0,
            iomap_base: core::mem::size_of::<TaskStateSegment>() as u16,
        }
    }
}

static mut GDT: [GdtEntry; GDT_ENTRIES] = [GdtEntry::null(); GDT_ENTRIES];
static mut TSS: TaskStateSegment = TaskStateSegment::new();

global_asm!(
    r#"
.global gdt_flush
gdt_flush:
    mov eax, [esp + 4]
    lgdt [eax]
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    push 0x08
    lea eax, [gdt_flush_done]
    push eax
    retf
gdt_flush_done:
    ret
"#
);

unsafe extern "C" {
    fn gdt_flush(gdt_descriptor: *const GdtDescriptor);
}

pub const fn kernel_code_selector() -> u16 {
    KERNEL_CODE_SELECTOR
}

pub const fn kernel_data_selector() -> u16 {
    KERNEL_DATA_SELECTOR
}

pub const fn user_code_selector() -> u16 {
    USER_CODE_SELECTOR
}

pub const fn user_data_selector() -> u16 {
    USER_DATA_SELECTOR
}

pub fn set_kernel_stack(stack_top: u32) {
    unsafe {
        TSS.esp0 = stack_top;
    }
}

pub fn init() {
    unsafe {
        GDT[0] = GdtEntry::null();
        GDT[1] = GdtEntry::new(0, 0x000F_FFFF, 0x9A, 0xCF);
        GDT[2] = GdtEntry::new(0, 0x000F_FFFF, 0x92, 0xCF);
        GDT[3] = GdtEntry::new(0, 0x000F_FFFF, 0xFA, 0xCF);
        GDT[4] = GdtEntry::new(0, 0x000F_FFFF, 0xF2, 0xCF);

        TSS = TaskStateSegment::new();

        let tss_base = core::ptr::addr_of!(TSS) as u32;
        let tss_limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u32;
        // 0x89: present + available 32-bit TSS, 0x40: 32-bit system segment.
        GDT[5] = GdtEntry::new(tss_base, tss_limit, 0x89, 0x40);

        let gdt_descriptor = GdtDescriptor {
            limit: (core::mem::size_of::<[GdtEntry; GDT_ENTRIES]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u32,
        };

        gdt_flush(&gdt_descriptor as *const GdtDescriptor);
        asm!("ltr ax", in("ax") TSS_SELECTOR, options(nostack, preserves_flags));
    }
}
