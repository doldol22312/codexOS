use core::arch::global_asm;

use crate::{keyboard, mouse, pic, println, timer};
use crate::serial_println;

#[repr(C)]
pub struct InterruptFrame {
    pub gs: u32,
    pub fs: u32,
    pub es: u32,
    pub ds: u32,
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
    pub int_no: u32,
    pub err_code: u32,
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
}

const EXCEPTION_MESSAGES: [&str; 32] = [
    "Divide-by-zero",
    "Debug",
    "NMI",
    "Breakpoint",
    "Overflow",
    "Bound range exceeded",
    "Invalid opcode",
    "Device not available",
    "Double fault",
    "Coprocessor segment overrun",
    "Invalid TSS",
    "Segment not present",
    "Stack segment fault",
    "General protection fault",
    "Page fault",
    "Reserved",
    "x87 floating point",
    "Alignment check",
    "Machine check",
    "SIMD floating point",
    "Virtualization",
    "Control protection",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Hypervisor injection",
    "VMM communication",
    "Security exception",
    "Reserved",
];

global_asm!(
    r#"
.global isr_common
isr_common:
    pusha
    xor eax, eax
    mov ax, ds
    push eax
    mov ax, es
    push eax
    mov ax, fs
    push eax
    mov ax, gs
    push eax
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    push esp
    call interrupt_dispatch
    add esp, 4
    pop eax
    mov gs, ax
    pop eax
    mov fs, ax
    pop eax
    mov es, ax
    pop eax
    mov ds, ax
    popa
    add esp, 8
    iretd
"#
);

unsafe extern "C" {
    fn isr_common();
}

macro_rules! isr_stub_no_error {
    ($name:ident, $vector:expr) => {
        #[unsafe(naked)]
        pub extern "C" fn $name() {
            core::arch::naked_asm!(
                "push 0",
                "push {vector}",
                "jmp {common}",
                vector = const $vector,
                common = sym isr_common
            );
        }
    };
}

macro_rules! isr_stub_error {
    ($name:ident, $vector:expr) => {
        #[unsafe(naked)]
        pub extern "C" fn $name() {
            core::arch::naked_asm!(
                "push {vector}",
                "jmp {common}",
                vector = const $vector,
                common = sym isr_common
            );
        }
    };
}

isr_stub_no_error!(isr0, 0);
isr_stub_no_error!(isr1, 1);
isr_stub_no_error!(isr2, 2);
isr_stub_no_error!(isr3, 3);
isr_stub_no_error!(isr4, 4);
isr_stub_no_error!(isr5, 5);
isr_stub_no_error!(isr6, 6);
isr_stub_no_error!(isr7, 7);
isr_stub_error!(isr8, 8);
isr_stub_no_error!(isr9, 9);
isr_stub_error!(isr10, 10);
isr_stub_error!(isr11, 11);
isr_stub_error!(isr12, 12);
isr_stub_error!(isr13, 13);
isr_stub_error!(isr14, 14);
isr_stub_no_error!(isr15, 15);
isr_stub_no_error!(isr16, 16);
isr_stub_error!(isr17, 17);
isr_stub_no_error!(isr18, 18);
isr_stub_no_error!(isr19, 19);
isr_stub_no_error!(isr20, 20);
isr_stub_error!(isr21, 21);
isr_stub_no_error!(isr22, 22);
isr_stub_no_error!(isr23, 23);
isr_stub_no_error!(isr24, 24);
isr_stub_no_error!(isr25, 25);
isr_stub_no_error!(isr26, 26);
isr_stub_no_error!(isr27, 27);
isr_stub_no_error!(isr28, 28);
isr_stub_error!(isr29, 29);
isr_stub_error!(isr30, 30);
isr_stub_no_error!(isr31, 31);

isr_stub_no_error!(isr32, 32);
isr_stub_no_error!(isr33, 33);
isr_stub_no_error!(isr34, 34);
isr_stub_no_error!(isr35, 35);
isr_stub_no_error!(isr36, 36);
isr_stub_no_error!(isr37, 37);
isr_stub_no_error!(isr38, 38);
isr_stub_no_error!(isr39, 39);
isr_stub_no_error!(isr40, 40);
isr_stub_no_error!(isr41, 41);
isr_stub_no_error!(isr42, 42);
isr_stub_no_error!(isr43, 43);
isr_stub_no_error!(isr44, 44);
isr_stub_no_error!(isr45, 45);
isr_stub_no_error!(isr46, 46);
isr_stub_no_error!(isr47, 47);

#[no_mangle]
pub extern "C" fn interrupt_dispatch(frame: *mut InterruptFrame) {
    let frame = unsafe { &mut *frame };
    let vector = frame.int_no as u8;

    match vector {
        0..=31 => handle_exception(frame),
        32 => {
            timer::handle_interrupt();
            pic::send_eoi(0);
        }
        33 => {
            keyboard::handle_interrupt();
            pic::send_eoi(1);
        }
        44 => {
            mouse::handle_interrupt();
            pic::send_eoi(12);
        }
        34..=47 => {
            pic::send_eoi(vector - 32);
        }
        _ => {}
    }
}

fn handle_exception(frame: &InterruptFrame) -> ! {
    let vector = frame.int_no as usize;
    serial_println!(
        "exception: vec={} err={:#x} eip={:#x}",
        vector,
        frame.err_code,
        frame.eip
    );
    println!();
    println!("EXCEPTION {}: {}", vector, EXCEPTION_MESSAGES[vector]);
    println!("err={} eip={:#010x}", frame.err_code, frame.eip);

    loop {
        unsafe {
            core::arch::asm!("cli; hlt", options(nomem, nostack));
        }
    }
}
