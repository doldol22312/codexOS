TARGET_JSON := i686-codex_os.json
TARGET_TRIPLE := i686-codex_os
PROFILE ?= debug
QEMU := qemu-system-i386

BUILD_DIR := build
BUILD_STAMP := $(BUILD_DIR)/.dir
IMG_PATH := $(BUILD_DIR)/codexos.img
DATA_IMG_PATH := $(BUILD_DIR)/data.img

KERNEL_ELF := target/$(TARGET_TRIPLE)/$(PROFILE)/codex_os
STAGE1_ELF := target/$(TARGET_TRIPLE)/$(PROFILE)/boot_stage1
STAGE2_ELF := target/$(TARGET_TRIPLE)/$(PROFILE)/boot_stage2

KERNEL_BIN := $(BUILD_DIR)/kernel.bin
STAGE1_BIN := $(BUILD_DIR)/boot_stage1.bin
STAGE2_BIN := $(BUILD_DIR)/boot_stage2.bin

FLOPPY_SECTORS := 2880
DATA_DISK_SECTORS := 32768
STAGE2_SECTORS := 32
KERNEL_SECTORS := 1100
KERNEL_LBA := 33

CARGO_FLAGS := -Zjson-target-spec -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem --target $(TARGET_JSON)

PROFILE_FLAG :=
ifeq ($(PROFILE),release)
PROFILE_FLAG := --release
endif

.PHONY: all build kernel stage1 stage2 image data-image run run-serial clean

all: build

build: image

$(BUILD_STAMP):
	mkdir -p $(BUILD_DIR)
	touch $(BUILD_STAMP)

kernel: | $(BUILD_STAMP)
	cargo +nightly rustc $(CARGO_FLAGS) $(PROFILE_FLAG) --bin codex_os

stage1: | $(BUILD_STAMP)
	cargo +nightly rustc $(CARGO_FLAGS) $(PROFILE_FLAG) --features bootloader-stage1 --bin boot_stage1 -- -C link-arg=-Tstage1.ld -C link-arg=--no-eh-frame-hdr

stage2: | $(BUILD_STAMP)
	cargo +nightly rustc $(CARGO_FLAGS) $(PROFILE_FLAG) --features bootloader-stage2 --bin boot_stage2 -- -C link-arg=-Tstage2.ld

$(STAGE1_BIN): stage1 | $(BUILD_STAMP)
	objcopy -O binary $(STAGE1_ELF) $(STAGE1_BIN)
	@if [ $$(stat -c%s $(STAGE1_BIN)) -ne 512 ]; then \
		echo "boot_stage1 must be exactly 512 bytes"; \
		exit 1; \
	fi

$(STAGE2_BIN): stage2 | $(BUILD_STAMP)
	objcopy -O binary $(STAGE2_ELF) $(STAGE2_BIN)
	@if [ $$(stat -c%s $(STAGE2_BIN)) -gt $$(( $(STAGE2_SECTORS) * 512 )) ]; then \
		echo "boot_stage2 is too large (max $(STAGE2_SECTORS) sectors)"; \
		exit 1; \
	fi

$(KERNEL_BIN): kernel | $(BUILD_STAMP)
	objcopy -O binary $(KERNEL_ELF) $(KERNEL_BIN)
	@if [ $$(stat -c%s $(KERNEL_BIN)) -gt $$(( $(KERNEL_SECTORS) * 512 )) ]; then \
		echo "kernel.bin is too large (max $(KERNEL_SECTORS) sectors)"; \
		exit 1; \
	fi

data-image: | $(BUILD_STAMP)
	@if [ ! -f $(DATA_IMG_PATH) ]; then \
		dd if=/dev/zero of=$(DATA_IMG_PATH) bs=512 count=$(DATA_DISK_SECTORS) status=none; \
		echo "Created data disk image: $(DATA_IMG_PATH)"; \
	fi

image: $(STAGE1_BIN) $(STAGE2_BIN) $(KERNEL_BIN) data-image | $(BUILD_STAMP)
	dd if=/dev/zero of=$(IMG_PATH) bs=512 count=$(FLOPPY_SECTORS) status=none
	dd if=$(STAGE1_BIN) of=$(IMG_PATH) bs=512 seek=0 conv=notrunc status=none
	dd if=$(STAGE2_BIN) of=$(IMG_PATH) bs=512 seek=1 conv=notrunc status=none
	dd if=$(KERNEL_BIN) of=$(IMG_PATH) bs=512 seek=$(KERNEL_LBA) conv=notrunc status=none
	@echo "Built boot image: $(IMG_PATH)"

run: image
	$(QEMU) -drive format=raw,file=$(IMG_PATH),if=floppy -drive format=raw,file=$(DATA_IMG_PATH),if=ide,index=0,media=disk -boot a -m 128M

run-serial: image
	$(QEMU) -drive format=raw,file=$(IMG_PATH),if=floppy -drive format=raw,file=$(DATA_IMG_PATH),if=ide,index=0,media=disk -boot a -m 128M -display none -serial stdio -monitor none -no-reboot -no-shutdown

clean:
	rm -rf $(BUILD_DIR)
	cargo +nightly clean
