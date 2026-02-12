# codexOS

A bare-metal operating system written from scratch in Rust for x86 (32-bit). Boots from a custom two-stage bootloader, runs an interactive shell, and includes drivers for keyboard, mouse, display, disk, and a custom on-disk filesystem -- all with zero external dependencies.

## Features

- **Custom two-stage bootloader** -- 512-byte MBR stage 1 reads a build-generated metadata sector, loads stage 2 by metadata, then stage 2 streams kernel sectors through a 0x9000 bounce buffer directly to high memory (1 MB+) before entering 32-bit protected mode
- **Memory management** -- 4 KiB paging with identity mapping (256 MB) plus framebuffer virtual mapping, and a free-list heap allocator with block coalescing
- **Interrupt-driven I/O** -- full IDT/GDT/PIC setup with handlers for CPU exceptions and hardware IRQs
- **Unified input system** -- single ring-buffer event queue for keyboard + mouse (`KeyPress`, `KeyRelease`, `MouseMove`, `MouseDown`, `MouseUp`, `MouseClick`) with simple hit-testing helpers
- **UI/event layer** -- central event dispatcher with hit-region routing, focus tracking, framebuffer widgets (`Panel`, `Label`, `Button`, `TextBox`, `TextArea`, `Checkbox`, `RadioButton`, `Dropdown`, `ComboBox`, `Scrollbar`, `ListView`, `TreeView`, `ProgressBar`, `PopupMenu`), and a window compositor with drag/resize/minimize/maximize
- **PS/2 keyboard driver** -- scancode translation, shift/caps lock, arrow/page keys, and key press/release event generation
- **PS/2 mouse driver** -- 3-byte packet parsing, absolute position tracking, button events, and a hardware-independent framebuffer cursor sprite with clipped redraw-safe save/restore
- **Framebuffer text console** -- dynamic text grid from VBE mode, bitmap glyph rendering to double buffer, dirty-rect flush optimization, batched redraw support, and blinking shell cursor (with VGA fallback)
- **Graphical multdemo** -- multi-window multitasking demo (`multdemo`) with parallel worker tasks (clock/gears/wave), interactive compositor controls, and a `multdemo bench` mode for scheduler/semaphore stress
- **Cursor/compositor stability** -- stabilized cursor and window composition behavior with clipped cursor updates and redraw-safe composition paths during drag/resize and focus changes
- **Serial port** -- COM1 UART output for debug logging
- **ATA PIO disk driver** -- 28-bit LBA read/write on the primary master drive
- **Custom filesystem (CFS1)** -- superblock + directory table + file storage with create, read, write, delete, list, and format operations
- **PIT timer** -- configurable frequency (default 100 Hz) with uptime tracking
- **CMOS RTC** -- date and time reads with BCD/binary format handling
- **Interactive shell** -- 31 built-in commands, command history, tab completion, line editing, and an in-shell text editor
- **Matrix screensaver** -- because every OS needs one (press any key to exit)

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
| `multdemo [bench [iterations]]` | Graphical multitasking windows demo or benchmark mode |
| `gfxdemo` | Framebuffer primitives demo |
| `uidemo` | UI dispatcher + widget demo |
| `uidemo2` | Advanced widget showcase (forms, lists, tree, popup menu) |
| `windemo` | Multi-window compositor demo |
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

# Debug build override
make PROFILE=debug run

# Clean build artifacts
make clean
```

## Project Structure

```
codexOS/
├── src/
│   ├── main.rs            Kernel entry and initialization sequence
│   ├── boot.rs            Assembly entry point, BSS zeroing
│   ├── allocator.rs       Free-list heap allocator (8 MB)
│   ├── ata.rs             ATA PIO disk driver
│   ├── bootinfo.rs        Stage2 -> kernel boot video metadata
│   ├── fs.rs              Custom filesystem (CFS1)
│   ├── gdt.rs             Global Descriptor Table
│   ├── idt.rs             Interrupt Descriptor Table
│   ├── interrupts.rs      Exception and IRQ dispatch
│   ├── input.rs           Unified keyboard/mouse event queue + hit testing
│   ├── io.rs              Port I/O primitives (inb/outb/inw/outw)
│   ├── keyboard.rs        PS/2 keyboard driver
│   ├── matrix.rs          Matrix rain screensaver
│   ├── mouse.rs           PS/2 mouse driver
│   ├── paging.rs          4 KiB page tables + framebuffer mapping
│   ├── pic.rs             8259 PIC initialization
│   ├── reboot.rs          System reboot
│   ├── rtc.rs             CMOS real-time clock
│   ├── serial.rs          COM1 UART driver
│   ├── shell.rs           Interactive command shell
│   ├── shutdown.rs        ACPI/APM power off
│   ├── timer.rs           PIT timer (IRQ0)
│   ├── ui.rs              UI dispatcher + framebuffer widget toolkit
│   ├── vga.rs             Bitmap-font framebuffer text console
│   └── bin/
│       ├── boot_stage1.rs MBR bootloader (512 bytes)
│       └── boot_stage2.rs Stage 2: VBE mode set, A20, protected mode, kernel load
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
    ...     Heap (8 MB)
    ...     Stack (1 MB, grows down)
0x10000000  End of identity-mapped region (256 MB)
```

## Disk Layout

**Floppy image** (`codexos.img`, 1.44 MB):

| Sectors | Contents |
|---|---|
| 0 | Stage 1 bootloader |
| 1 | Boot metadata (`stage2_lba`, `stage2_sectors`, `kernel_lba`, `kernel_sectors`, `kernel_bytes`) |
| 2.. | Stage 2 bootloader (size-derived) |
| after stage 2 | Kernel binary (size-derived) |

Boot metadata sector format (`CDX1`, little-endian):

| Offset | Size | Field |
|---|---|---|
| 0x00 | 4 | Magic (`CDX1`) |
| 0x04 | 2 | `stage2_lba` |
| 0x06 | 2 | `stage2_sectors` |
| 0x08 | 2 | `kernel_lba` |
| 0x0A | 2 | `kernel_sectors` |
| 0x0C | 4 | `kernel_bytes` |

**Data disk** (`data.img`, 16 MB):

| Sectors | Contents |
|---|---|
| 0 | CFS1 superblock |
| 1--16 | Directory table |
| 17+ | File data |

## License

This project does not currently specify a license.
