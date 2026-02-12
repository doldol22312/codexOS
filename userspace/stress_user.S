# Userspace scheduler/syscall stress test for codexOS.
# Syscalls:
#   3 -> write(EBX=ptr, ECX=len)
#   1 -> yield()
#   2 -> sleep(EBX=ticks)
#   0 -> exit()

.section .text
.global _start

_start:
    mov $8, %edi                # iterations

stress_loop:
    mov $3, %eax                # write
    mov $msg_tick, %ebx
    mov $msg_tick_len, %ecx
    int $0x80

    mov $1, %eax                # yield
    int $0x80

    mov $2, %eax                # sleep(3 ticks)
    mov $3, %ebx
    int $0x80

    dec %edi
    jnz stress_loop

    mov $3, %eax
    mov $msg_done, %ebx
    mov $msg_done_len, %ecx
    int $0x80

    mov $0, %eax                # exit
    int $0x80

halt_forever:
    jmp halt_forever

.section .rodata
msg_tick:
    .ascii "stress tick\n"
.set msg_tick_len, . - msg_tick

msg_done:
    .ascii "stress complete\n"
.set msg_done_len, . - msg_done
