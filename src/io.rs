use core::arch::asm;

#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let mut value: u8;
    asm!(
        "in al, dx",
        in("dx") port,
        out("al") value,
        options(nomem, nostack, preserves_flags)
    );
    value
}

#[inline]
pub unsafe fn io_wait() {
    asm!(
        "out 0x80, al",
        in("al") 0u8,
        options(nomem, nostack, preserves_flags)
    );
}
