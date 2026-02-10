#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

global_asm!(
    r#"
    .section .text
    .code16
    .global _start

_start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00

    mov [boot_drive], dl

    mov bx, 0x8000
    mov si, 1
    mov cx, 32

load_stage2:
    cmp cx, 0
    je stage2_loaded

read_retry:
    push cx
    push si
    push bx

    mov ax, si
    call lba_to_chs

    mov ah, 0x02
    mov al, 0x01
    mov dl, [boot_drive]
    int 0x13
    jnc read_ok

    xor ax, ax
    int 0x13
    pop bx
    pop si
    pop cx
    jmp read_retry

read_ok:
    pop bx
    pop si
    pop cx

    add bx, 512
    inc si
    dec cx
    jmp load_stage2

stage2_loaded:
    mov dl, [boot_drive]
    .byte 0xEA
    .word 0x8000
    .word 0x0000

lba_to_chs:
    xor dx, dx
    mov di, 36
    div di
    mov bp, ax

    mov ax, dx
    xor dx, dx
    mov di, 18
    div di

    mov dh, al
    mov al, dl
    inc al
    mov cl, al

    mov ax, bp
    mov ch, al
    shr ax, 2
    and al, 0xC0
    or cl, al
    ret

boot_drive:
    .byte 0

    .org 510
    .word 0xAA55
"#
);

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
