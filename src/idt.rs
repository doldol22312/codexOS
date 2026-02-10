use core::arch::asm;

use crate::interrupts;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    zero: u8,
    flags: u8,
    offset_high: u16,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            zero: 0,
            flags: 0,
            offset_high: 0,
        }
    }

    fn new(handler: extern "C" fn()) -> Self {
        let address = handler as usize as u32;
        Self {
            offset_low: (address & 0xFFFF) as u16,
            selector: 0x08,
            zero: 0,
            flags: 0x8E,
            offset_high: ((address >> 16) & 0xFFFF) as u16,
        }
    }
}

#[repr(C, packed)]
struct IdtDescriptor {
    limit: u16,
    base: u32,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

pub fn init() {
    unsafe {
        IDT[0] = IdtEntry::new(interrupts::isr0);
        IDT[1] = IdtEntry::new(interrupts::isr1);
        IDT[2] = IdtEntry::new(interrupts::isr2);
        IDT[3] = IdtEntry::new(interrupts::isr3);
        IDT[4] = IdtEntry::new(interrupts::isr4);
        IDT[5] = IdtEntry::new(interrupts::isr5);
        IDT[6] = IdtEntry::new(interrupts::isr6);
        IDT[7] = IdtEntry::new(interrupts::isr7);
        IDT[8] = IdtEntry::new(interrupts::isr8);
        IDT[9] = IdtEntry::new(interrupts::isr9);
        IDT[10] = IdtEntry::new(interrupts::isr10);
        IDT[11] = IdtEntry::new(interrupts::isr11);
        IDT[12] = IdtEntry::new(interrupts::isr12);
        IDT[13] = IdtEntry::new(interrupts::isr13);
        IDT[14] = IdtEntry::new(interrupts::isr14);
        IDT[15] = IdtEntry::new(interrupts::isr15);
        IDT[16] = IdtEntry::new(interrupts::isr16);
        IDT[17] = IdtEntry::new(interrupts::isr17);
        IDT[18] = IdtEntry::new(interrupts::isr18);
        IDT[19] = IdtEntry::new(interrupts::isr19);
        IDT[20] = IdtEntry::new(interrupts::isr20);
        IDT[21] = IdtEntry::new(interrupts::isr21);
        IDT[22] = IdtEntry::new(interrupts::isr22);
        IDT[23] = IdtEntry::new(interrupts::isr23);
        IDT[24] = IdtEntry::new(interrupts::isr24);
        IDT[25] = IdtEntry::new(interrupts::isr25);
        IDT[26] = IdtEntry::new(interrupts::isr26);
        IDT[27] = IdtEntry::new(interrupts::isr27);
        IDT[28] = IdtEntry::new(interrupts::isr28);
        IDT[29] = IdtEntry::new(interrupts::isr29);
        IDT[30] = IdtEntry::new(interrupts::isr30);
        IDT[31] = IdtEntry::new(interrupts::isr31);

        IDT[32] = IdtEntry::new(interrupts::isr32);
        IDT[33] = IdtEntry::new(interrupts::isr33);
        IDT[34] = IdtEntry::new(interrupts::isr34);
        IDT[35] = IdtEntry::new(interrupts::isr35);
        IDT[36] = IdtEntry::new(interrupts::isr36);
        IDT[37] = IdtEntry::new(interrupts::isr37);
        IDT[38] = IdtEntry::new(interrupts::isr38);
        IDT[39] = IdtEntry::new(interrupts::isr39);
        IDT[40] = IdtEntry::new(interrupts::isr40);
        IDT[41] = IdtEntry::new(interrupts::isr41);
        IDT[42] = IdtEntry::new(interrupts::isr42);
        IDT[43] = IdtEntry::new(interrupts::isr43);
        IDT[44] = IdtEntry::new(interrupts::isr44);
        IDT[45] = IdtEntry::new(interrupts::isr45);
        IDT[46] = IdtEntry::new(interrupts::isr46);
        IDT[47] = IdtEntry::new(interrupts::isr47);

        let idt_descriptor = IdtDescriptor {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u32,
        };

        asm!(
            "lidt [{0}]",
            in(reg) &idt_descriptor,
            options(readonly, nostack, preserves_flags)
        );
    }
}
