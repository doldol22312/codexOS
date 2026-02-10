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
    mov ss, ax
    mov sp, 0x7a00
    mov [boot_drive], dl

    mov ax, 0x1000
    mov es, ax
    xor bx, bx
    mov si, 33
    mov cx, 1024

load_kernel:
    cmp cx, 0
    je kernel_loaded

read_kernel_retry:
    push cx
    push si
    push bx
    push es

    mov ax, si
    call lba_to_chs

    mov ah, 0x02
    mov al, 0x01
    mov dl, [boot_drive]
    int 0x13
    jnc read_kernel_ok

    xor ax, ax
    int 0x13
    pop es
    pop bx
    pop si
    pop cx
    jmp read_kernel_retry

read_kernel_ok:
    pop es
    pop bx
    pop si
    pop cx

    add bx, 512
    jnc no_segment_bump
    mov ax, es
    add ax, 0x1000
    mov es, ax

no_segment_bump:
    inc si
    dec cx
    jmp load_kernel

kernel_loaded:
    call enable_a20

    lgdt [gdt_descriptor]

    mov eax, cr0
    or eax, 0x00000001
    mov cr0, eax
    .byte 0x66, 0xEA
    .long protected_mode_entry
    .word 0x0008

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

enable_a20:
    in al, 0x92
    or al, 0x02
    and al, 0xFE
    out 0x92, al
    ret

    .align 8
gdt:
    .quad 0x0000000000000000
    .quad 0x00CF9A000000FFFF
    .quad 0x00CF92000000FFFF
gdt_end:

gdt_descriptor:
    .word gdt_end - gdt - 1
    .long gdt

boot_drive:
    .byte 0

    .code32
protected_mode_entry:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov esp, 0x0009FC00

    cld
    mov esi, 0x00010000
    mov edi, 0x00100000
    mov ecx, (1024 * 512) / 4
    rep movsd

    mov eax, 0x00100000
    jmp eax
"#
);

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
