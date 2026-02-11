use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::io::{inb, inw, io_wait, outb, outw};

const ATA_PRIMARY_IO: u16 = 0x1F0;
const ATA_PRIMARY_CTRL: u16 = 0x3F6;

const REG_DATA: u16 = 0x00;
const REG_ERROR: u16 = 0x01;
const REG_FEATURES: u16 = 0x01;
const REG_SECCOUNT0: u16 = 0x02;
const REG_LBA0: u16 = 0x03;
const REG_LBA1: u16 = 0x04;
const REG_LBA2: u16 = 0x05;
const REG_HDDEVSEL: u16 = 0x06;
const REG_COMMAND: u16 = 0x07;
const REG_STATUS: u16 = 0x07;

const REG_ALT_STATUS: u16 = 0x00;

const CMD_IDENTIFY: u8 = 0xEC;
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_CACHE_FLUSH: u8 = 0xE7;

const STATUS_ERR: u8 = 1 << 0;
const STATUS_DRQ: u8 = 1 << 3;
const STATUS_DF: u8 = 1 << 5;
const STATUS_DRDY: u8 = 1 << 6;
const STATUS_BSY: u8 = 1 << 7;

const DRIVE_MASTER: u8 = 0xE0;

const POLL_LIMIT: usize = 1_000_000;
const SECTOR_SIZE: usize = 512;

static PRESENT: AtomicBool = AtomicBool::new(false);
static SECTORS: AtomicU32 = AtomicU32::new(0);
static MODEL: [core::sync::atomic::AtomicU8; 40] = [const { core::sync::atomic::AtomicU8::new(0) }; 40];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtaError {
    NotInitialized,
    OutOfRange,
    Timeout,
    DeviceFault,
    DeviceError,
}

#[derive(Clone, Copy)]
pub struct DiskInfo {
    pub present: bool,
    pub sectors: u32,
    pub sector_size: usize,
    pub model: [u8; 40],
}

pub fn init() {
    let Some((sectors, model)) = identify_primary_master() else {
        PRESENT.store(false, Ordering::Relaxed);
        SECTORS.store(0, Ordering::Relaxed);
        clear_model();
        return;
    };

    SECTORS.store(sectors, Ordering::Relaxed);
    for (index, byte) in model.iter().copied().enumerate() {
        MODEL[index].store(byte, Ordering::Relaxed);
    }
    PRESENT.store(true, Ordering::Relaxed);
}

pub fn is_present() -> bool {
    PRESENT.load(Ordering::Relaxed)
}

pub fn info() -> Option<DiskInfo> {
    if !is_present() {
        return None;
    }

    let mut model = [0u8; 40];
    for (index, byte) in model.iter_mut().enumerate() {
        *byte = MODEL[index].load(Ordering::Relaxed);
    }

    Some(DiskInfo {
        present: true,
        sectors: SECTORS.load(Ordering::Relaxed),
        sector_size: SECTOR_SIZE,
        model,
    })
}

pub fn read_sector(lba: u32, buffer: &mut [u8; SECTOR_SIZE]) -> Result<(), AtaError> {
    if !is_present() {
        return Err(AtaError::NotInitialized);
    }

    if lba >= SECTORS.load(Ordering::Relaxed) {
        return Err(AtaError::OutOfRange);
    }

    unsafe {
        select_drive_and_lba(lba);
        outb(ATA_PRIMARY_IO + REG_FEATURES, 0);
        outb(ATA_PRIMARY_IO + REG_SECCOUNT0, 1);
        outb(ATA_PRIMARY_IO + REG_LBA0, (lba & 0xFF) as u8);
        outb(ATA_PRIMARY_IO + REG_LBA1, ((lba >> 8) & 0xFF) as u8);
        outb(ATA_PRIMARY_IO + REG_LBA2, ((lba >> 16) & 0xFF) as u8);
        outb(ATA_PRIMARY_IO + REG_COMMAND, CMD_READ_SECTORS);
    }

    poll_data_ready()?;

    for index in 0..(SECTOR_SIZE / 2) {
        let word = unsafe { inw(ATA_PRIMARY_IO + REG_DATA) };
        buffer[index * 2] = (word & 0x00FF) as u8;
        buffer[index * 2 + 1] = ((word >> 8) & 0x00FF) as u8;
    }

    delay_400ns();
    Ok(())
}

pub fn write_sector(lba: u32, buffer: &[u8; SECTOR_SIZE]) -> Result<(), AtaError> {
    if !is_present() {
        return Err(AtaError::NotInitialized);
    }

    if lba >= SECTORS.load(Ordering::Relaxed) {
        return Err(AtaError::OutOfRange);
    }

    unsafe {
        select_drive_and_lba(lba);
        outb(ATA_PRIMARY_IO + REG_FEATURES, 0);
        outb(ATA_PRIMARY_IO + REG_SECCOUNT0, 1);
        outb(ATA_PRIMARY_IO + REG_LBA0, (lba & 0xFF) as u8);
        outb(ATA_PRIMARY_IO + REG_LBA1, ((lba >> 8) & 0xFF) as u8);
        outb(ATA_PRIMARY_IO + REG_LBA2, ((lba >> 16) & 0xFF) as u8);
        outb(ATA_PRIMARY_IO + REG_COMMAND, CMD_WRITE_SECTORS);
    }

    poll_data_ready()?;

    for index in 0..(SECTOR_SIZE / 2) {
        let low = buffer[index * 2] as u16;
        let high = (buffer[index * 2 + 1] as u16) << 8;
        unsafe {
            outw(ATA_PRIMARY_IO + REG_DATA, low | high);
        }
    }

    unsafe {
        outb(ATA_PRIMARY_IO + REG_COMMAND, CMD_CACHE_FLUSH);
    }

    wait_not_busy()?;
    delay_400ns();
    Ok(())
}

fn identify_primary_master() -> Option<(u32, [u8; 40])> {
    unsafe {
        outb(ATA_PRIMARY_IO + REG_HDDEVSEL, DRIVE_MASTER);
    }
    delay_400ns();

    unsafe {
        outb(ATA_PRIMARY_IO + REG_SECCOUNT0, 0);
        outb(ATA_PRIMARY_IO + REG_LBA0, 0);
        outb(ATA_PRIMARY_IO + REG_LBA1, 0);
        outb(ATA_PRIMARY_IO + REG_LBA2, 0);
        outb(ATA_PRIMARY_IO + REG_COMMAND, CMD_IDENTIFY);
    }

    let status = unsafe { inb(ATA_PRIMARY_IO + REG_STATUS) };
    if status == 0 {
        return None;
    }

    for _ in 0..POLL_LIMIT {
        let status = unsafe { inb(ATA_PRIMARY_IO + REG_STATUS) };
        if (status & STATUS_ERR) != 0 || (status & STATUS_DF) != 0 {
            return None;
        }
        if (status & STATUS_BSY) == 0 && (status & STATUS_DRQ) != 0 {
            let mut identify_words = [0u16; 256];
            for word in identify_words.iter_mut() {
                *word = unsafe { inw(ATA_PRIMARY_IO + REG_DATA) };
            }

            let mut model = [0u8; 40];
            for index in 0..20 {
                let value = identify_words[27 + index];
                model[index * 2] = (value >> 8) as u8;
                model[index * 2 + 1] = (value & 0xFF) as u8;
            }

            trim_ascii_right(&mut model);

            let lba28 = ((identify_words[61] as u32) << 16) | identify_words[60] as u32;
            let lba48 = ((identify_words[103] as u64) << 48)
                | ((identify_words[102] as u64) << 32)
                | ((identify_words[101] as u64) << 16)
                | (identify_words[100] as u64);

            let sectors = if lba48 != 0 { lba48 } else { lba28 as u64 };
            if sectors == 0 {
                return None;
            }

            return Some((sectors.min(u32::MAX as u64) as u32, model));
        }
    }

    None
}

fn clear_model() {
    for byte in &MODEL {
        byte.store(0, Ordering::Relaxed);
    }
}

fn trim_ascii_right(model: &mut [u8; 40]) {
    let mut end = model.len();
    while end > 0 {
        let ch = model[end - 1];
        if ch == b' ' || ch == 0 {
            end -= 1;
        } else {
            break;
        }
    }

    for byte in model.iter_mut().skip(end) {
        *byte = 0;
    }
}

unsafe fn select_drive_and_lba(lba: u32) {
    outb(
        ATA_PRIMARY_IO + REG_HDDEVSEL,
        DRIVE_MASTER | (((lba >> 24) & 0x0F) as u8),
    );
    delay_400ns();
}

fn poll_data_ready() -> Result<(), AtaError> {
    wait_not_busy()?;

    for _ in 0..POLL_LIMIT {
        let status = unsafe { inb(ATA_PRIMARY_IO + REG_STATUS) };
        if (status & STATUS_ERR) != 0 {
            let _ = unsafe { inb(ATA_PRIMARY_IO + REG_ERROR) };
            return Err(AtaError::DeviceError);
        }
        if (status & STATUS_DF) != 0 {
            return Err(AtaError::DeviceFault);
        }
        if (status & STATUS_DRQ) != 0 {
            return Ok(());
        }
    }

    Err(AtaError::Timeout)
}

fn wait_not_busy() -> Result<(), AtaError> {
    for _ in 0..POLL_LIMIT {
        let status = unsafe { inb(ATA_PRIMARY_IO + REG_STATUS) };
        if (status & STATUS_BSY) == 0 {
            if (status & STATUS_ERR) != 0 {
                return Err(AtaError::DeviceError);
            }
            if (status & STATUS_DF) != 0 {
                return Err(AtaError::DeviceFault);
            }
            if (status & STATUS_DRDY) != 0 || (status & STATUS_DRQ) != 0 {
                return Ok(());
            }
        }
    }

    Err(AtaError::Timeout)
}

fn delay_400ns() {
    unsafe {
        let _ = inb(ATA_PRIMARY_CTRL + REG_ALT_STATUS);
        let _ = inb(ATA_PRIMARY_CTRL + REG_ALT_STATUS);
        let _ = inb(ATA_PRIMARY_CTRL + REG_ALT_STATUS);
        let _ = inb(ATA_PRIMARY_CTRL + REG_ALT_STATUS);
        io_wait();
    }
}
