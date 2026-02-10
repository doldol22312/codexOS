use core::arch::global_asm;

global_asm!(
    r#"
    .section .text.boot, "ax"
    .global _start
_start:
    cli
    cld
    lea esp, [stack_top]
    call zero_bss
    call kernel_main
1:
    hlt
    jmp 1b
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
