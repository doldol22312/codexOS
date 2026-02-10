TARGET_JSON := i686-codex_os.json
TARGET_TRIPLE := i686-codex_os
PROFILE ?= debug
KERNEL := target/$(TARGET_TRIPLE)/$(PROFILE)/codex_os
ISO_DIR := build/isofiles
ISO_PATH := build/codexos.iso
QEMU := qemu-system-i386

.PHONY: all build iso run run-iso clean

all: build

build:
	cargo +nightly build -Zjson-target-spec -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem --target $(TARGET_JSON)

iso: build
	mkdir -p $(ISO_DIR)/boot/grub
	cp $(KERNEL) $(ISO_DIR)/boot/kernel.elf
	cp grub.cfg $(ISO_DIR)/boot/grub/grub.cfg
	grub-mkrescue -o $(ISO_PATH) $(ISO_DIR)

run: iso
	$(QEMU) -cdrom $(ISO_PATH) -m 128M

run-iso: iso
	$(QEMU) -cdrom $(ISO_PATH) -m 128M

clean:
	rm -rf build
	cargo +nightly clean
