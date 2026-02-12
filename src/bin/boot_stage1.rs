#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

global_asm!(
    r#"
    .section .text
    .code16
    .global _start

    .equ BOOT_META_ADDR, 0x7E00
    .equ BOOT_META_LBA, 0x0001
    .equ BOOT_META_STAGE2_LBA_OFF, 4
    .equ BOOT_META_STAGE2_SECTORS_OFF, 6

_start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00

    mov [boot_drive], dl

    call read_boot_meta
    cmp word ptr [BOOT_META_ADDR], 0x4443
    jne boot_fail
    cmp word ptr [BOOT_META_ADDR + 2], 0x3158
    jne boot_fail

    mov bx, 0x8000
    mov si, word ptr [BOOT_META_ADDR + BOOT_META_STAGE2_LBA_OFF]
    mov cx, word ptr [BOOT_META_ADDR + BOOT_META_STAGE2_SECTORS_OFF]
    cmp cx, 0
    je boot_fail

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

read_boot_meta:
    mov bx, BOOT_META_ADDR
    mov si, BOOT_META_LBA

read_boot_meta_retry:
    push si
    push bx

    mov ax, si
    call lba_to_chs

    mov ah, 0x02
    mov al, 0x01
    mov dl, [boot_drive]
    int 0x13
    jnc read_boot_meta_ok

    xor ax, ax
    int 0x13
    pop bx
    pop si
    jmp read_boot_meta_retry

read_boot_meta_ok:
    pop bx
    pop si
    ret

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

boot_fail:
    cli
    hlt
    jmp boot_fail

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
