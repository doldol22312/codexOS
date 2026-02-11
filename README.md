# codexOS

A bare-metal operating system written from scratch in Rust for x86 (32-bit). Boots from a custom two-stage bootloader, runs an interactive shell, and includes drivers for keyboard, mouse, display, disk, and a custom on-disk filesystem -- all with zero external dependencies.

## Features

- **Custom two-stage bootloader** -- 512-byte MBR stage 1 loads a stage 2 that enables A20, switches to 32-bit protected mode, and jumps to the kernel at 1 MB
- **Memory management** -- identity-mapped paging (128 MB via 4 MB large pages) and a free-list heap allocator with block coalescing
- **Interrupt-driven I/O** -- full IDT/GDT/PIC setup with handlers for CPU exceptions and hardware IRQs
- **PS/2 keyboard driver** -- scancode translation, shift/caps lock, arrow/page keys, and a 256-byte circular input buffer
- **PS/2 mouse driver** -- 3-byte packet parsing, absolute position tracking, and button state
- **VGA text mode** -- 80x25 display with color support, hardware cursor, scrolling, and PageUp/PageDown scrollback view
- **Serial port** -- COM1 UART output for debug logging
- **ATA PIO disk driver** -- 28-bit LBA read/write on the primary master drive
- **Custom filesystem (CFS1)** -- superblock + directory table + file storage with create, read, write, delete, list, and format operations
- **PIT timer** -- configurable frequency (default 100 Hz) with uptime tracking
- **CMOS RTC** -- date and time reads with BCD/binary format handling
- **Interactive shell** -- 26 built-in commands, command history, tab completion, line editing, and an in-shell text editor
- **Matrix screensaver** -- because every OS needs one

## Shell Commands

| Command | Description |
|---|---|
| `help` | List available commands |
| `clear` | Clear the screen |
| `echo <text>` | Print text |
| `info` | System information and uptime |
| `disk` | ATA disk info |
| `fsformat` | Format the data disk |
| `fsinfo` | Filesystem status |
| `fsls` | List files |
| `fswrite <name> <text>` | Create/write a file |
| `fscat <name>` | Read a file |
| `fsdelete <name>` | Delete a file |
| `edit <name>` | Open simple line editor for a file |
| `date` | Current date |
| `time` | Current time |
| `rtc` | Full RTC status |
| `paging` | Paging statistics |
| `uptime` | Kernel uptime |
| `heap` | Heap memory stats |
| `memtest [bytes]` | Test heap allocation |
| `hexdump <addr> [len]` | Dump memory contents |
| `mouse` | Mouse position and button state |
| `matrix` | Matrix rain screensaver |
| `color` | Set text colors |
| `reboot` | Reboot the system |
| `shutdown` | Power off |
| `panic` | Trigger a kernel panic (for testing) |

Shell input supports command history (`Up`/`Down`), cursor movement (`Left`/`Right`), tab completion for commands and filenames, and output scrolling with `PageUp`/`PageDown`.

## Building

### Prerequisites

- **Rust nightly** toolchain with the `rust-src` component
- **QEMU** (`qemu-system-i386`) for running the OS
- **GNU Make**

The project pins its toolchain in `rust-toolchain.toml`, so `rustup` will install the right version automatically.

### Build and Run

```bash
# Build everything (bootloader + kernel + disk images)
make build

# Build and run in QEMU with graphical VGA output
make run

# Build and run with serial output on stdio (headless)
make run-serial

# Release build
make PROFILE=release run

# Clean build artifacts
make clean
```

## Project Structure

```
codexOS/
├── src/
│   ├── main.rs            Kernel entry and initialization sequence
│   ├── boot.rs            Assembly entry point, BSS zeroing
│   ├── allocator.rs       Free-list heap allocator (512 KB)
│   ├── ata.rs             ATA PIO disk driver
│   ├── fs.rs              Custom filesystem (CFS1)
│   ├── gdt.rs             Global Descriptor Table
│   ├── idt.rs             Interrupt Descriptor Table
│   ├── interrupts.rs      Exception and IRQ dispatch
│   ├── io.rs              Port I/O primitives (inb/outb/inw/outw)
│   ├── keyboard.rs        PS/2 keyboard driver
│   ├── matrix.rs          Matrix rain screensaver
│   ├── mouse.rs           PS/2 mouse driver
│   ├── paging.rs          Page directory setup (4 MB pages)
│   ├── pic.rs             8259 PIC initialization
│   ├── reboot.rs          System reboot
│   ├── rtc.rs             CMOS real-time clock
│   ├── serial.rs          COM1 UART driver
│   ├── shell.rs           Interactive command shell
│   ├── shutdown.rs        ACPI/APM power off
│   ├── timer.rs           PIT timer (IRQ0)
│   ├── vga.rs             VGA text-mode driver
│   └── bin/
│       ├── boot_stage1.rs MBR bootloader (512 bytes)
│       └── boot_stage2.rs Stage 2: A20, protected mode, kernel load
├── linker.ld              Kernel linker script (loads at 1 MB)
├── stage1.ld              Stage 1 bootloader linker script
├── stage2.ld              Stage 2 bootloader linker script
├── i686-codex_os.json     Custom Rust target specification
├── Cargo.toml             Package manifest
├── Makefile               Build orchestration
└── rust-toolchain.toml    Nightly toolchain pinning
```

## Memory Layout

```
0x00007C00  Stage 1 bootloader (512 bytes)
0x00008000  Stage 2 bootloader
0x00100000  Kernel .text (1 MB)
    ...     .rodata, .data, .bss
    ...     Heap (512 KB)
    ...     Stack (1 MB, grows down)
0x08000000  End of identity-mapped region (128 MB)
```

## Disk Layout

**Floppy image** (`codexos.img`, 1.44 MB):

| Sectors | Contents |
|---|---|
| 0 | Stage 1 bootloader |
| 1--32 | Stage 2 bootloader |
| 33--1056 | Kernel binary |

**Data disk** (`data.img`, 16 MB):

| Sectors | Contents |
|---|---|
| 0 | CFS1 superblock |
| 1--16 | Directory table |
| 17+ | File data |

## License

This project does not currently specify a license.
