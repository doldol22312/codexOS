TARGET_JSON := i686-codex_os.json
TARGET_TRIPLE := i686-codex_os
PROFILE ?= release
QEMU := qemu-system-i386
QEMU_NET_ARGS := -netdev user,id=n0 -device ne2k_pci,netdev=n0

BUILD_DIR := build
BUILD_STAMP := $(BUILD_DIR)/.dir
IMG_PATH := $(BUILD_DIR)/codexos.img
DATA_IMG_PATH := $(BUILD_DIR)/data.img
BOOT_META_BIN := $(BUILD_DIR)/boot_meta.bin
BOOT_LAYOUT := $(BUILD_DIR)/boot_layout.env

KERNEL_ELF := target/$(TARGET_TRIPLE)/$(PROFILE)/codex_os
STAGE1_ELF := target/$(TARGET_TRIPLE)/$(PROFILE)/boot_stage1
STAGE2_ELF := target/$(TARGET_TRIPLE)/$(PROFILE)/boot_stage2

KERNEL_BIN := $(BUILD_DIR)/kernel.bin
STAGE1_BIN := $(BUILD_DIR)/boot_stage1.bin
STAGE2_BIN := $(BUILD_DIR)/boot_stage2.bin
USER_HELLO_OBJ := $(BUILD_DIR)/hello_user.o
USER_HELLO_ELF := $(BUILD_DIR)/hello.elf
USER_HELLO_FS_NAME := hello.elf
USER_STRESS_OBJ := $(BUILD_DIR)/stress_user.o
USER_STRESS_ELF := $(BUILD_DIR)/stress.elf
USER_STRESS_FS_NAME := stress.elf

FLOPPY_SECTORS := 2880
DATA_DISK_SECTORS := 32768
BOOT_META_LBA := 1
STAGE2_LBA := 2

CARGO_FLAGS := -Zjson-target-spec -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem --target $(TARGET_JSON)

PROFILE_FLAG :=
ifeq ($(PROFILE),release)
PROFILE_FLAG := --release
endif

.PHONY: all build kernel stage1 stage2 image data-image user-hello user-stress inject-user-hello inject-user-stress inject-user-samples run-user-hello run-user-stress run run-serial discord-bridge clean

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

$(KERNEL_BIN): kernel | $(BUILD_STAMP)
	objcopy -O binary $(KERNEL_ELF) $(KERNEL_BIN)

$(USER_HELLO_OBJ): userspace/hello_user.S | $(BUILD_STAMP)
	as --32 -o $(USER_HELLO_OBJ) userspace/hello_user.S

$(USER_HELLO_ELF): $(USER_HELLO_OBJ) | $(BUILD_STAMP)
	ld -m elf_i386 -nostdlib -Ttext 0x40010000 -e _start -o $(USER_HELLO_ELF) $(USER_HELLO_OBJ)

user-hello: $(USER_HELLO_ELF)

$(USER_STRESS_OBJ): userspace/stress_user.S | $(BUILD_STAMP)
	as --32 -o $(USER_STRESS_OBJ) userspace/stress_user.S

$(USER_STRESS_ELF): $(USER_STRESS_OBJ) | $(BUILD_STAMP)
	ld -m elf_i386 -nostdlib -Ttext 0x40012000 -e _start -o $(USER_STRESS_ELF) $(USER_STRESS_OBJ)

user-stress: $(USER_STRESS_ELF)

inject-user-hello: data-image user-hello
	python3 tools/inject_cfs.py --image $(DATA_IMG_PATH) --host $(USER_HELLO_ELF) --name $(USER_HELLO_FS_NAME) --format-if-needed

inject-user-stress: data-image user-stress
	python3 tools/inject_cfs.py --image $(DATA_IMG_PATH) --host $(USER_STRESS_ELF) --name $(USER_STRESS_FS_NAME) --format-if-needed

inject-user-samples: inject-user-hello inject-user-stress

$(BOOT_LAYOUT): $(STAGE2_BIN) $(KERNEL_BIN) | $(BUILD_STAMP)
	@stage2_bytes=$$(stat -c%s $(STAGE2_BIN)); \
	kernel_bytes=$$(stat -c%s $(KERNEL_BIN)); \
	stage2_sectors=$$(( (stage2_bytes + 511) / 512 )); \
	kernel_sectors=$$(( (kernel_bytes + 511) / 512 )); \
	kernel_lba=$$(( $(STAGE2_LBA) + stage2_sectors )); \
	end_lba=$$(( kernel_lba + kernel_sectors )); \
	if [ $$stage2_sectors -eq 0 ]; then \
		echo "boot_stage2 produced an empty binary"; \
		exit 1; \
	fi; \
	if [ $$kernel_sectors -eq 0 ]; then \
		echo "kernel produced an empty binary"; \
		exit 1; \
	fi; \
	if [ $$end_lba -gt $(FLOPPY_SECTORS) ]; then \
		echo "kernel image too large for floppy layout (end sector $$end_lba, max $(FLOPPY_SECTORS))"; \
		exit 1; \
	fi; \
	printf 'STAGE2_LBA=%s\nSTAGE2_SECTORS=%s\nKERNEL_LBA=%s\nKERNEL_SECTORS=%s\nKERNEL_BYTES=%s\n' \
		$(STAGE2_LBA) $$stage2_sectors $$kernel_lba $$kernel_sectors $$kernel_bytes > $(BOOT_LAYOUT)

$(BOOT_META_BIN): $(BOOT_LAYOUT) | $(BUILD_STAMP)
	@. $(BOOT_LAYOUT); \
	perl -e 'print "CDX1"; print pack("vvvvV", @ARGV); print "\0" x (512 - 16);' \
		$$STAGE2_LBA $$STAGE2_SECTORS $$KERNEL_LBA $$KERNEL_SECTORS $$KERNEL_BYTES > $(BOOT_META_BIN)

data-image: | $(BUILD_STAMP)
	@if [ ! -f $(DATA_IMG_PATH) ]; then \
		dd if=/dev/zero of=$(DATA_IMG_PATH) bs=512 count=$(DATA_DISK_SECTORS) status=none; \
		echo "Created data disk image: $(DATA_IMG_PATH)"; \
	fi

image: $(STAGE1_BIN) $(STAGE2_BIN) $(KERNEL_BIN) $(BOOT_META_BIN) data-image | $(BUILD_STAMP)
	@. $(BOOT_LAYOUT); \
	dd if=/dev/zero of=$(IMG_PATH) bs=512 count=$(FLOPPY_SECTORS) status=none; \
	dd if=$(STAGE1_BIN) of=$(IMG_PATH) bs=512 seek=0 conv=notrunc status=none; \
	dd if=$(BOOT_META_BIN) of=$(IMG_PATH) bs=512 seek=$(BOOT_META_LBA) conv=notrunc status=none; \
	dd if=$(STAGE2_BIN) of=$(IMG_PATH) bs=512 seek=$$STAGE2_LBA conv=notrunc status=none; \
	dd if=$(KERNEL_BIN) of=$(IMG_PATH) bs=512 seek=$$KERNEL_LBA conv=notrunc status=none; \
	echo "Built boot image: $(IMG_PATH) (stage2 $$STAGE2_SECTORS sectors, kernel $$KERNEL_SECTORS sectors)"

run: image
	$(QEMU) -drive format=raw,file=$(IMG_PATH),if=floppy -drive format=raw,file=$(DATA_IMG_PATH),if=ide,index=0,media=disk -boot a -m 128M $(QEMU_NET_ARGS)

run-serial: image
	$(QEMU) -drive format=raw,file=$(IMG_PATH),if=floppy -drive format=raw,file=$(DATA_IMG_PATH),if=ide,index=0,media=disk -boot a -m 128M $(QEMU_NET_ARGS) -display none -serial stdio -monitor none -no-reboot -no-shutdown

run-user-hello: image inject-user-hello
	$(QEMU) -drive format=raw,file=$(IMG_PATH),if=floppy -drive format=raw,file=$(DATA_IMG_PATH),if=ide,index=0,media=disk -boot a -m 128M $(QEMU_NET_ARGS) -display none -serial stdio -monitor none -no-reboot -no-shutdown

run-user-stress: image inject-user-stress
	$(QEMU) -drive format=raw,file=$(IMG_PATH),if=floppy -drive format=raw,file=$(DATA_IMG_PATH),if=ide,index=0,media=disk -boot a -m 128M $(QEMU_NET_ARGS) -display none -serial stdio -monitor none -no-reboot -no-shutdown

discord-bridge:
	python3 tools/discord_bridge.py --bind 0.0.0.0 --port 4242

clean:
	rm -rf $(BUILD_DIR)
	cargo +nightly clean
