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

    mov ax, 0x0900
    mov es, ax
    xor bx, bx
    mov si, 33
    mov cx, 1200

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
    call init_boot_video_info
    call cache_font_8x16
    call set_vbe_mode
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

init_boot_video_info:
    mov di, 0x5000
    mov dword ptr [di + 0], 0x32454256
    mov dword ptr [di + 4], 0
    mov dword ptr [di + 8], 0
    mov dword ptr [di + 12], 0
    mov dword ptr [di + 16], 0
    mov dword ptr [di + 20], 0
    mov dword ptr [di + 24], 0
    mov dword ptr [di + 28], 0
    mov dword ptr [di + 32], 0
    mov dword ptr [di + 36], 0
    mov dword ptr [di + 40], 0
    ret

cache_font_8x16:
    mov ax, 0x1130
    mov bh, 0x06
    int 0x10

    mov si, bp
    mov di, 0x5200
    mov cx, 4096
copy_font_loop:
    mov al, es:[si]
    mov [di], al
    inc si
    inc di
    loop copy_font_loop

    mov di, 0x5000
    mov dword ptr [di + 32], 0x00005200
    mov dword ptr [di + 36], 4096
    mov dword ptr [di + 40], 16
    mov eax, dword ptr [di + 4]
    or eax, 0x00000002
    mov dword ptr [di + 4], eax
    ret

set_vbe_mode:
    mov ax, 0x4F02
    mov bx, 0x118
    push bx
    mov ax, 0x4F01
    xor cx, cx
    mov cx, bx
    xor dx, dx
    mov es, dx
    mov di, 0x6000
    int 0x10
    cmp ax, 0x004F
    jne try_mode_114

    pop bx
    push bx
    mov ax, 0x4F02
    or bx, 0x4000
    int 0x10
    cmp ax, 0x004F
    jne try_mode_114
    pop bx
    call store_vbe_mode_info
    ret

try_mode_114:
    pop bx
    mov bx, 0x114
    push bx
    mov ax, 0x4F01
    xor cx, cx
    mov cx, bx
    xor dx, dx
    mov es, dx
    mov di, 0x6000
    int 0x10
    cmp ax, 0x004F
    jne vbe_fallback_text

    pop bx
    push bx
    mov ax, 0x4F02
    or bx, 0x4000
    int 0x10
    cmp ax, 0x004F
    jne vbe_fallback_text
    pop bx
    call store_vbe_mode_info
    ret

vbe_fallback_text:
    pop bx
    mov ax, 0x0003
    int 0x10
    ret

store_vbe_mode_info:
    mov si, 0x6000
    mov di, 0x5000

    xor eax, eax
    mov ax, bx
    mov dword ptr [di + 8], eax

    xor eax, eax
    mov ax, word ptr [si + 0x12]
    mov dword ptr [di + 12], eax

    xor eax, eax
    mov ax, word ptr [si + 0x14]
    mov dword ptr [di + 16], eax

    xor eax, eax
    mov ax, word ptr [si + 0x10]
    mov dword ptr [di + 20], eax

    xor eax, eax
    mov al, byte ptr [si + 0x19]
    mov dword ptr [di + 24], eax

    mov eax, dword ptr [si + 0x28]
    mov dword ptr [di + 28], eax

    mov eax, dword ptr [di + 4]
    or eax, 0x00000001
    mov dword ptr [di + 4], eax
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
    mov esi, 0x00009000
    mov edi, 0x00100000
    mov ecx, (1200 * 512) / 4
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
