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
mod pic;
mod shell;
pub mod vga;

use core::alloc::Layout;
use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    vga::clear_screen();
    println!("codexOS booting...");
    gdt::init();
    idt::init();
    pic::init();
    keyboard::init();

    unsafe {
        core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
    }

    println!("Interrupts online. Starting shell.");
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
    println!();
    println!("KERNEL PANIC");
    println!("{}", info);

    loop {
        unsafe {
            core::arch::asm!("cli; hlt", options(nomem, nostack));
        }
    }
}
