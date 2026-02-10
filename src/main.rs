#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

mod allocator;
mod boot;
mod gdt;
mod idt;
mod interrupts;
mod io;
mod keyboard;
mod matrix;
mod mouse;
mod pic;
mod reboot;
mod serial;
mod shell;
mod shutdown;
mod timer;
pub mod vga;

use core::alloc::Layout;
use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    serial::init();
    serial_println!("serial: initialized");

    vga::set_color(0x0F, 0x00);
    vga::clear_screen();
    println!("codexOS booting...");
    serial_println!("boot: vga ready");

    gdt::init();
    serial_println!("boot: gdt ready");
    idt::init();
    serial_println!("boot: idt ready");
    pic::init();
    serial_println!("boot: pic ready");
    timer::init(100);
    serial_println!("boot: pit ready ({}hz)", timer::frequency_hz());
    keyboard::init();
    serial_println!("boot: keyboard ready");
    mouse::init();
    serial_println!("boot: mouse ready");

    unsafe {
        core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
    }
    serial_println!("boot: interrupts enabled");

    println!("Interrupts online. Starting shell.");
    serial_println!("boot: entering shell");
    shell::run();
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!(
        "allocation error: size={} align={}",
        layout.size(),
        layout.align()
    );
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    serial_println!("KERNEL PANIC: {}", info);
    println!();
    println!("KERNEL PANIC");
    println!("{}", info);

    loop {
        unsafe {
            core::arch::asm!("cli; hlt", options(nomem, nostack));
        }
    }
}
