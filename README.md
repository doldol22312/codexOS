# codexOS

A bare-metal operating system written from scratch in Rust for x86 (32-bit). Boots from a custom two-stage bootloader, runs a preemptive multitasking kernel with a windowed desktop environment, and includes drivers for keyboard, mouse, display, disk, and a custom on-disk filesystem -- all with zero external dependencies.

~14,000 lines of Rust. No libc, no libunwind, no runtime.

## Features

### Boot & Core
- **Custom two-stage bootloader** -- 512-byte MBR stage 1 reads a build-generated metadata sector, loads stage 2 by metadata, then stage 2 streams kernel sectors through a 0x9000 bounce buffer directly to high memory (1 MB+) before entering 32-bit protected mode
- **Memory management** -- 4 KiB paging with identity mapping (256 MB) plus framebuffer virtual mapping, and a free-list heap allocator with block coalescing (8 MB heap)
- **Interrupt-driven I/O** -- full IDT/GDT/PIC setup with handlers for all 32 CPU exceptions and hardware IRQs (0--47)
- **Preemptive multitasking** -- timer-driven round-robin scheduler with up to 16 tasks, 64 KB stacks, task states (ready/running/sleeping/exited), and syscalls for `yield` and `sleep`
- **Synchronization primitives** -- spinlock-based `Mutex` with RAII guards and counting `Semaphore` with atomic permits

### Drivers
- **PS/2 keyboard** -- scancode translation (including extended 0xE0 prefix), shift/caps lock state, arrow/page keys, and key press/release event generation
- **PS/2 mouse** -- 3-byte packet parsing, absolute position tracking, button events, and a framebuffer cursor sprite with clipped redraw-safe save/restore
- **ATA PIO disk** -- 28-bit LBA read/write on the primary master drive with IDENTIFY, timeout, and error recovery
- **PCI + NE2000 networking** -- PCI bus scan, RTL8029/NE2000-compatible NIC probe, ARP/IPv4, DNS resolution, TCP handshake, and UDP request/response exchange
- **Serial port** -- COM1 UART output for debug logging
- **PIT timer** -- configurable frequency (default 100 Hz) with tick counting and uptime tracking
- **CMOS RTC** -- date and time reads with BCD/binary format handling and mid-update avoidance

### Graphics & UI
- **Framebuffer text console** -- dynamic text grid from VBE mode, bitmap glyph rendering to double buffer, dirty-rect flush optimization, batched redraw support, and blinking shell cursor (with VGA text-mode fallback)
- **Unified input system** -- single 512-slot ring-buffer event queue for keyboard + mouse (`KeyPress`, `KeyRelease`, `MouseMove`, `MouseDown`, `MouseUp`, `MouseClick`) with hit-region testing and focus tracking
- **Widget toolkit** -- 14 framebuffer widgets: `Panel`, `Label`, `Button`, `TextBox`, `TextArea`, `Checkbox`, `RadioButton`, `Dropdown`, `ComboBox`, `Scrollbar`, `ListView`, `TreeView`, `ProgressBar`, `PopupMenu`
- **Window compositor** -- up to 16 windows with drag, resize, minimize, maximize, z-ordering, title bars, close buttons, and redraw-safe composition paths

### Shell & Desktop
- **Interactive shell** -- built-in commands for storage, diagnostics, graphics demos, and Discord bridge checks; command history (32 entries), tab completion for commands and filenames, cursor-based line editing, and an in-shell text editor
- **Desktop environment** -- start menu launcher, app registry, taskbar with open-window buttons and clock, and desktop background layering behind compositor windows
- **Desktop apps** -- functional Terminal (shell session), File Browser (CFS1 directory listing), System Monitor (heap/uptime/task metrics), Notes (text editor), Pixel Paint (color palette + canvas), and Discord bridge client
- **Custom filesystem (CFS1)** -- superblock + directory table + file storage with create, read, write, delete, list, and format operations (16 MB data disk, up to 256 files)
- **Demos** -- graphical multitasking demo (`multdemo`) with parallel worker tasks and benchmark mode, graphics primitives demo, widget showcases, window compositor demo, and Matrix screensaver

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
| `elfrun <name>` | Load and run an ELF32 user process from CFS1 (foreground) |
| `date` | Current date |
| `time` | Current time |
| `rtc` | Full RTC status |
| `paging` | Paging statistics |
| `uptime` | Kernel uptime |
| `heap` | Heap memory stats |
| `memtest [bytes]` | Test heap allocation |
| `hexdump <addr> [len]` | Dump memory contents |
| `mouse` | Mouse position and button state |
| `netinfo` | PCI + network stack diagnostics |
| `discordcfg` | Validate and inspect `discord.cfg` |
| `discorddiag` | Live Discord bridge diagnostics and sync check |
| `matrix` | Matrix rain screensaver |
| `multdemo [bench [iterations]]` | Graphical multitasking windows demo or benchmark mode |
| `gfxdemo` | Framebuffer primitives demo |
| `uidemo` | UI dispatcher + widget demo |
| `uidemo2` | Advanced widget showcase (forms, lists, tree, popup menu) |
| `windemo` | Multi-window compositor demo |
| `desktop` | Desktop environment shell demo (taskbar, launcher, app registry) |
| `color` | Set text colors |
| `reboot` | Reboot the system |
| `shutdown` | Power off |
| `panic` | Trigger a kernel panic (for testing) |

Shell input supports command history (`Up`/`Down`), cursor movement (`Left`/`Right`), tab completion for commands and filenames, and output scrolling with `PageUp`/`PageDown`.

## Desktop Environment

The `desktop` command launches a windowed desktop environment with a taskbar, start-menu launcher, and six built-in applications:

| App | Description |
|---|---|
| **Terminal** | Shell session with command input, history, and output scrollback |
| **Files** | CFS1 file browser with directory listing and file details |
| **Monitor** | Live system metrics -- heap usage, uptime, task count, tick rate |
| **Notes** | Multi-line text editor with cursor navigation and word wrap |
| **Paint** | Pixel canvas with 8-color palette, brush tool, and clear button |
| **Discord** | Bridge-backed Discord client with guild/channel/message panes and message composer |

Each app runs in its own compositor window with drag, resize, minimize, and close support. The taskbar shows buttons for open windows and a real-time clock.

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

# Run host-side Discord UDP bridge (required for live Discord traffic)
make discord-bridge

# Build sample user ELF, inject into CFS1, and boot (serial)
make run-user-hello

# Build scheduler/syscall stress user ELF, inject into CFS1, and boot (serial)
make run-user-stress

# Debug build override
make PROFILE=debug run

# Clean build artifacts
make clean
```

### Userspace ELF Smoke Test

`make run-user-hello` compiles `userspace/hello_user.S`, injects it as `hello.elf` into `build/data.img` (auto-formatting CFS1 if needed), and boots the kernel in serial mode.

At the `codexOS>` prompt:

```text
fsls
elfrun hello.elf
```

Expected output includes:

```text
elfrun: started hello.elf as task <id>
hello from user mode
codexOS>
```

### Userspace Scheduler Stress Test

`make run-user-stress` compiles `userspace/stress_user.S`, injects it as `stress.elf` into `build/data.img`, and boots in serial mode.

At the `codexOS>` prompt:

```text
elfrun stress.elf
```

Expected output includes repeated ticks plus completion:

```text
stress tick
...
stress complete
```

## Discord Bridge Setup

The kernel does not implement TLS/WebSocket directly; it uses a lightweight UDP bridge on the host that performs authenticated Discord REST calls.

1. Start the bridge on the host:

```bash
make discord-bridge
```

2. In codexOS, create `discord.cfg`:

```text
bot_token=<your discord bot token>
default_guild_id=<optional numeric guild id>
default_channel_id=<optional numeric channel id>
bridge_ip=10.0.2.2
bridge_port=4242
poll_ticks=120
```

3. Write it from shell (example):

```text
fswrite discord.cfg bot_token=YOUR_TOKEN default_channel_id=123456789012345678 bridge_ip=10.0.2.2 bridge_port=4242 poll_ticks=120
```

4. Verify in codexOS:

```text
netinfo
discordcfg
discorddiag
desktop
```

If `discorddiag` reports `transport=connected`, the Discord desktop app can send and receive messages via the bridge.

## Project Structure

```
codexOS/
├── src/
│   ├── main.rs            Kernel entry and initialization sequence
│   ├── boot.rs            Assembly entry point, BSS zeroing
│   ├── allocator.rs       Free-list heap allocator (8 MB)
│   ├── ata.rs             ATA PIO disk driver
│   ├── bootinfo.rs        Stage2 -> kernel boot video metadata
│   ├── elf.rs             ELF32 userspace loader
│   ├── discord/           Discord bridge client/config parser
│   ├── fs.rs              Custom filesystem (CFS1)
│   ├── gdt.rs             Global Descriptor Table
│   ├── idt.rs             Interrupt Descriptor Table
│   ├── interrupts.rs      Exception and IRQ dispatch
│   ├── input.rs           Unified keyboard/mouse event queue + hit testing
│   ├── io.rs              Port I/O primitives (inb/outb/inw/outw)
│   ├── keyboard.rs        PS/2 keyboard driver
│   ├── matrix.rs          Matrix rain screensaver
│   ├── mouse.rs           PS/2 mouse driver
│   ├── net/               NE2000 NIC driver + IPv4/UDP/TCP primitives
│   ├── paging.rs          4 KiB page tables + framebuffer mapping
│   ├── pci.rs             PCI bus scan and device enumeration
│   ├── pic.rs             8259 PIC initialization
│   ├── reboot.rs          System reboot
│   ├── rtc.rs             CMOS real-time clock
│   ├── serial.rs          COM1 UART driver
│   ├── shell/
│   │   ├── mod.rs         Core REPL loop, history, and tab completion
│   │   ├── editor.rs      Built-in line editor command implementation
│   │   ├── demos.rs       Graphics/UI/desktop demo command implementations
│   │   └── commands/      Non-demo command groups
│   ├── shutdown.rs        ACPI/APM power off
│   ├── sync.rs            Mutex and Semaphore primitives
│   ├── task.rs            Preemptive task scheduler (round-robin)
│   ├── timer.rs           PIT timer (IRQ0)
│   ├── vga.rs             Bitmap-font framebuffer text console
│   ├── ui/
│   │   ├── mod.rs         UI module re-exports and shared types
│   │   ├── dispatcher.rs  Event dispatcher with hit-region routing
│   │   ├── widgets.rs     14 framebuffer widget types
│   │   └── window.rs      Window compositor (drag/resize/z-order)
│   └── bin/
│       ├── boot_stage1.rs MBR bootloader (512 bytes)
│       └── boot_stage2.rs Stage 2: VBE mode set, A20, protected mode, kernel load
├── userspace/
│   ├── hello_user.S       Minimal ring-3 syscall test program
│   └── stress_user.S      Ring-3 yield/sleep/write stress test program
├── tools/
│   ├── inject_cfs.py      Host-side CFS1 file injector
│   └── discord_bridge.py  Host-side Discord REST <-> UDP bridge
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
