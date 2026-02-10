use core::arch::global_asm;

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

static mut GDT: [GdtEntry; 3] = [GdtEntry::null(); 3];

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

pub fn init() {
    unsafe {
        GDT[0] = GdtEntry::null();
        GDT[1] = GdtEntry::new(0, 0x000F_FFFF, 0x9A, 0xCF);
        GDT[2] = GdtEntry::new(0, 0x000F_FFFF, 0x92, 0xCF);

        let gdt_descriptor = GdtDescriptor {
            limit: (core::mem::size_of::<[GdtEntry; 3]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u32,
        };

        gdt_flush(&gdt_descriptor as *const GdtDescriptor);
    }
}
