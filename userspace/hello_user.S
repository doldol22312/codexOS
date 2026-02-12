# Minimal ring-3 userspace test program for codexOS.
# Syscall ABI (EAX = number):
#   3 -> write(EBX=ptr, ECX=len)
#   0 -> exit()

.section .text
.global _start

_start:
    mov $3, %eax
    mov $message, %ebx
    mov $message_len, %ecx
    int $0x80

    mov $0, %eax
    int $0x80

halt_forever:
    jmp halt_forever

.section .rodata
message:
    .ascii "hello from user mode\n"
.set message_len, . - message
