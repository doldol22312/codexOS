use core::arch::global_asm;

global_asm!(
    r#"
    .set MB_MAGIC, 0x1BADB002
    .set MB_FLAGS, 0x00000003
    .set MB_CHECKSUM, -(MB_MAGIC + MB_FLAGS)

    .section .multiboot, "a"
    .align 4
    .long MB_MAGIC
    .long MB_FLAGS
    .long MB_CHECKSUM

    .section .text
    .global _start
_start:
    cli
    lea esp, [stack_top]
    call zero_bss
    call kernel_main
1:
    hlt
    jmp 1b

    .section .boot_stack, "aw", @nobits
    .align 16
stack_bottom:
    .skip 16384
stack_top:
"#
);

unsafe extern "C" {
    static mut __bss_start: u8;
    static mut __bss_end: u8;
}

#[no_mangle]
pub extern "C" fn zero_bss() {
    unsafe {
        let mut current = core::ptr::addr_of_mut!(__bss_start) as usize;
        let end = core::ptr::addr_of_mut!(__bss_end) as usize;

        while current < end {
            (current as *mut u8).write_volatile(0);
            current += 1;
        }
    }
}
