use crate::io::{inb, io_wait, outb};

pub fn reboot() -> ! {
    unsafe {
        for _ in 0..100_000 {
            if (inb(0x64) & 0x02) == 0 {
                outb(0x64, 0xFE);
                break;
            }
            io_wait();
        }
    }

    loop {
        unsafe {
            core::arch::asm!("cli; hlt", options(nomem, nostack));
        }
    }
}
