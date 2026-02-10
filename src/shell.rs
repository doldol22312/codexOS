use core::str;

use crate::{allocator, keyboard, print, println, vga};

const MAX_LINE: usize = 256;

pub fn run() -> ! {
    let mut line = [0u8; MAX_LINE];
    let mut len = 0usize;

    print_prompt();

    loop {
        if let Some(ch) = keyboard::read_char() {
            match ch {
                '\n' => {
                    println!();
                    execute_line(&line[..len]);
                    len = 0;
                    print_prompt();
                }
                '\x08' => {
                    if len > 0 {
                        len -= 1;
                        vga::backspace();
                    }
                }
                _ => {
                    if is_printable(ch) && len < MAX_LINE {
                        line[len] = ch as u8;
                        len += 1;
                        print!("{}", ch);
                    }
                }
            }
        } else {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }
}

fn print_prompt() {
    print!("codexOS> ");
}

fn execute_line(bytes: &[u8]) {
    let line = str::from_utf8(bytes).unwrap_or("");
    let mut parts = line.split_whitespace();
    let Some(command) = parts.next() else {
        return;
    };

    match command {
        "help" => {
            println!("Commands:");
            println!("  help  - show this message");
            println!("  clear - clear screen");
            println!("  echo  - echo arguments");
            println!("  info  - show system info");
            println!("  heap  - show heap usage");
        }
        "clear" => vga::clear_screen(),
        "echo" => {
            let mut first = true;
            for part in parts {
                if !first {
                    print!(" ");
                }
                print!("{}", part);
                first = false;
            }
            println!();
        }
        "info" => {
            println!("codexOS barebones kernel");
            println!("arch: x86 (32-bit)");
            println!("lang: Rust + inline assembly");
            println!("boot: Multiboot/GRUB");
            println!("features: VGA, IDT, IRQ keyboard, shell, heap");
        }
        "heap" => {
            let heap = allocator::stats();
            println!("heap start: {:#010x}", heap.start);
            println!("heap end:   {:#010x}", heap.end);
            println!("heap total: {} bytes", heap.total);
            println!("heap used:  {} bytes", heap.used);
            println!("heap free:  {} bytes", heap.remaining);
        }
        _ => {
            println!("unknown command: {}", command);
        }
    }
}

fn is_printable(ch: char) -> bool {
    ch >= ' ' && ch <= '~'
}
