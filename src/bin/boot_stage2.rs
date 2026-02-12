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
    .equ BOOT_META_KERNEL_LBA_OFF, 8
    .equ BOOT_META_KERNEL_SECTORS_OFF, 10
    .equ BOUNCE_SEG, 0x0900
    .equ BOUNCE_OFF, 0x0000
    .equ BOUNCE_ADDR, 0x00009000
    .equ KERNEL_LOAD_ADDR, 0x00100000

_start:
    cli
    xor ax, ax
    mov ds, ax
    mov ss, ax
    mov sp, 0x7a00
    mov [boot_drive], dl

    cmp word ptr [BOOT_META_ADDR], 0x4443
    jne boot_fail
    cmp word ptr [BOOT_META_ADDR + 2], 0x3158
    jne boot_fail
    mov cx, word ptr [BOOT_META_ADDR + BOOT_META_KERNEL_SECTORS_OFF]
    cmp cx, 0
    je boot_fail

    call init_boot_video_info
    call cache_font_8x16
    call set_vbe_mode
    call enable_a20
    call enable_unreal_fs

after_unreal_mode:
    xor ax, ax
    mov ds, ax

    mov dword ptr [kernel_dst_ptr], KERNEL_LOAD_ADDR

    mov ax, BOUNCE_SEG
    mov es, ax
    mov bx, BOUNCE_OFF
    mov si, word ptr [BOOT_META_ADDR + BOOT_META_KERNEL_LBA_OFF]
    mov cx, word ptr [BOOT_META_ADDR + BOOT_META_KERNEL_SECTORS_OFF]

load_kernel:
    cmp cx, 0
    je kernel_loaded

read_kernel_retry:
    xor ax, ax
    mov ds, ax

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
    xor ax, ax
    mov ds, ax

    pop es
    pop bx
    pop si
    pop cx

    call enable_unreal_fs
    xor ax, ax
    mov ds, ax

    push cx
    push si
    mov esi, BOUNCE_ADDR
    mov edi, dword ptr [kernel_dst_ptr]
    mov ecx, 128

copy_sector_to_kernel:
    mov eax, dword ptr [esi]
    mov dword ptr fs:[edi], eax
    add esi, 4
    add edi, 4
    dec ecx
    jnz copy_sector_to_kernel

    pop si
    pop cx
    mov dword ptr [kernel_dst_ptr], edi
    inc si
    dec cx
    jmp load_kernel

kernel_loaded:
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

enable_unreal_fs:
    lgdt [gdt_descriptor]
    mov eax, cr0
    or eax, 0x00000001
    mov cr0, eax
    .byte 0x66, 0xEA
    .long unreal_mode_pm
    .word 0x0018

unreal_mode_pm:
    mov ax, 0x10
    mov fs, ax
    mov eax, cr0
    and eax, 0xFFFFFFFE
    mov cr0, eax
    .byte 0xEA
    .word unreal_mode_rm
    .word 0x0000

unreal_mode_rm:
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
    .quad 0x00009A000000FFFF
gdt_end:

gdt_descriptor:
    .word gdt_end - gdt - 1
    .long gdt

boot_drive:
    .byte 0
kernel_dst_ptr:
    .long 0

boot_fail:
    cli
    hlt
    jmp boot_fail

    .code32
protected_mode_entry:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov esp, 0x0009FC00

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
