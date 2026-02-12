#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

mod allocator;
mod ata;
mod boot;
mod bootinfo;
mod discord;
mod elf;
mod fs;
mod gdt;
mod idt;
mod input;
mod interrupts;
mod io;
mod keyboard;
mod matrix;
mod mouse;
mod net;
mod paging;
mod pci;
mod pic;
mod reboot;
mod rtc;
mod serial;
mod shell;
mod shutdown;
mod sync;
mod task;
mod timer;
mod ui;
pub mod vga;

use core::alloc::Layout;
use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    serial::init();
    serial_println!("serial: initialized");

    gdt::init();
    serial_println!("boot: gdt ready");
    idt::init();
    serial_println!("boot: idt ready");
    paging::init();
    serial_println!(
        "boot: paging ready ({} MiB mapped, {} KiB pages)",
        paging::stats().mapped_bytes / (1024 * 1024),
        paging::stats().page_size_bytes / 1024
    );
    if let Some(mapping) = paging::framebuffer_mapping() {
        serial_println!(
            "boot: framebuffer {}x{} {}bpp mapped at {:#010x}",
            mapping.width,
            mapping.height,
            mapping.bpp,
            mapping.virtual_base
        );
        serial_println!(
            "boot: fb phys={:#010x} bytes={}",
            mapping.physical_base,
            mapping.bytes,
        );
    } else {
        serial_println!("boot: fb mapping unavailable");
    }

    vga::init();
    vga::set_color(0x0F, 0x00);
    vga::clear_screen();
    println!("codexOS booting...");
    serial_println!("boot: video ready");
    pic::init();
    serial_println!("boot: pic ready");
    timer::init(100);
    serial_println!("boot: pit ready ({}hz)", timer::frequency_hz());
    ata::init();
    if let Some(disk) = ata::info() {
        serial_println!(
            "boot: ata ready ({} sectors, {} bytes)",
            disk.sectors,
            disk.sectors as u64 * disk.sector_size as u64
        );
    } else {
        serial_println!("boot: ata unavailable");
    }
    fs::init();
    serial_println!(
        "boot: fs {}",
        if fs::is_mounted() {
            "mounted"
        } else {
            "unmounted"
        }
    );
    let rtc_ready = rtc::init();
    serial_println!(
        "boot: rtc {}",
        if rtc_ready { "ready" } else { "unavailable" }
    );
    input::init();
    serial_println!("boot: input queue ready");
    keyboard::init();
    serial_println!("boot: keyboard ready");
    mouse::init();
    serial_println!("boot: mouse ready");
    pci::init();
    serial_println!("boot: pci scan complete ({} devices)", pci::device_count());
    match net::init() {
        Ok(()) => {
            serial_println!("boot: net ready ({})", net::stats().nic_name);
        }
        Err(error) => {
            serial_println!("boot: net unavailable ({:?})", error);
        }
    }
    match task::init() {
        Ok(()) => {
            serial_println!("boot: scheduler ready");
        }
        Err(reason) => {
            serial_println!("boot: scheduler disabled ({})", reason);
        }
    }

    unsafe {
        core::arch::asm!("sti", options(nomem, nostack));
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
