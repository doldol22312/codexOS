use crate::ata::{self, AtaError};

const SECTOR_SIZE: usize = 512;
const SUPERBLOCK_LBA: u32 = 0;
const DIRECTORY_START_LBA: u32 = 1;
const DEFAULT_DIRECTORY_SECTORS: u16 = 16;
const ENTRY_SIZE: usize = 64;
const ENTRIES_PER_SECTOR: usize = SECTOR_SIZE / ENTRY_SIZE;
const MAX_FILENAME_LEN: usize = 48;

const FS_MAGIC: [u8; 4] = *b"CFS1";
const FS_VERSION: u16 = 1;

#[derive(Clone, Copy)]
struct FsState {
    mounted: bool,
    total_sectors: u32,
    directory_sectors: u16,
    next_free_lba: u32,
    file_count: u32,
}

impl FsState {
    const fn unmounted() -> Self {
        Self {
            mounted: false,
            total_sectors: 0,
            directory_sectors: 0,
            next_free_lba: 0,
            file_count: 0,
        }
    }
}

static mut STATE: FsState = FsState::unmounted();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    DiskUnavailable,
    NotMounted,
    InvalidSuperblock,
    NameEmpty,
    NameTooLong,
    NameInvalid,
    NotFound,
    DirectoryFull,
    NoSpace,
    BufferTooSmall,
    Corrupt,
    Io(AtaError),
}

impl FsError {
    pub fn as_str(self) -> &'static str {
        match self {
            FsError::DiskUnavailable => "disk unavailable",
            FsError::NotMounted => "filesystem not mounted",
            FsError::InvalidSuperblock => "invalid superblock",
            FsError::NameEmpty => "empty filename",
            FsError::NameTooLong => "filename too long",
            FsError::NameInvalid => "invalid filename",
            FsError::NotFound => "not found",
            FsError::DirectoryFull => "directory full",
            FsError::NoSpace => "not enough disk space",
            FsError::BufferTooSmall => "buffer too small",
            FsError::Corrupt => "filesystem metadata corrupt",
            FsError::Io(error) => match error {
                AtaError::NotInitialized => "ata not initialized",
                AtaError::OutOfRange => "ata out of range",
                AtaError::Timeout => "ata timeout",
                AtaError::DeviceFault => "ata device fault",
                AtaError::DeviceError => "ata device error",
            },
        }
    }
}

impl From<AtaError> for FsError {
    fn from(value: AtaError) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Copy)]
pub struct FsInfo {
    pub disk_present: bool,
    pub mounted: bool,
    pub total_sectors: u32,
    pub directory_sectors: u16,
    pub next_free_lba: u32,
    pub file_count: u32,
    pub free_sectors: u32,
}

#[derive(Clone, Copy)]
pub struct FileInfo {
    pub name: [u8; MAX_FILENAME_LEN],
    pub name_len: u8,
    pub size_bytes: u32,
}

impl FileInfo {
    pub const fn empty() -> Self {
        Self {
            name: [0; MAX_FILENAME_LEN],
            name_len: 0,
            size_bytes: 0,
        }
    }

    pub fn name_str(&self) -> &str {
        let len = (self.name_len as usize).min(MAX_FILENAME_LEN);
        core::str::from_utf8(&self.name[..len]).unwrap_or("?")
    }
}

#[derive(Clone, Copy)]
pub struct ReadResult {
    pub total_size: usize,
    pub copied_size: usize,
}

#[derive(Clone, Copy)]
struct DirectoryEntry {
    used: bool,
    name_len: u8,
    start_lba: u32,
    size_bytes: u32,
    allocated_sectors: u32,
    name: [u8; MAX_FILENAME_LEN],
}

impl DirectoryEntry {
    const fn empty() -> Self {
        Self {
            used: false,
            name_len: 0,
            start_lba: 0,
            size_bytes: 0,
            allocated_sectors: 0,
            name: [0; MAX_FILENAME_LEN],
        }
    }
}

#[derive(Clone, Copy)]
struct EntryLocation {
    sector_lba: u32,
    slot_index: usize,
    entry: DirectoryEntry,
}

#[derive(Clone, Copy)]
struct SlotLocation {
    sector_lba: u32,
    slot_index: usize,
}

pub fn init() {
    if !ata::is_present() {
        unsafe {
            STATE = FsState::unmounted();
        }
        return;
    }

    match load_state_from_disk() {
        Ok(state) => unsafe {
            STATE = state;
        },
        Err(_) => unsafe {
            STATE = FsState::unmounted();
        },
    }
}

pub fn is_mounted() -> bool {
    unsafe { STATE.mounted }
}

pub fn info() -> FsInfo {
    let disk_present = ata::is_present();

    unsafe {
        let state = STATE;
        let free_sectors = if state.mounted && state.total_sectors >= state.next_free_lba {
            state.total_sectors - state.next_free_lba
        } else {
            0
        };

        FsInfo {
            disk_present,
            mounted: state.mounted,
            total_sectors: state.total_sectors,
            directory_sectors: state.directory_sectors,
            next_free_lba: state.next_free_lba,
            file_count: state.file_count,
            free_sectors,
        }
    }
}

pub fn format() -> Result<(), FsError> {
    let Some(disk) = ata::info() else {
        return Err(FsError::DiskUnavailable);
    };

    let total_sectors = disk.sectors;
    if total_sectors <= DIRECTORY_START_LBA + 1 {
        return Err(FsError::NoSpace);
    }

    let mut directory_sectors = DEFAULT_DIRECTORY_SECTORS;
    let max_directory = total_sectors.saturating_sub(DIRECTORY_START_LBA + 1) as u16;
    if directory_sectors > max_directory {
        directory_sectors = max_directory.max(1);
    }

    let data_start_lba = DIRECTORY_START_LBA + directory_sectors as u32;
    if data_start_lba >= total_sectors {
        return Err(FsError::NoSpace);
    }

    let state = FsState {
        mounted: true,
        total_sectors,
        directory_sectors,
        next_free_lba: data_start_lba,
        file_count: 0,
    };

    write_superblock(state)?;

    let zero_sector = [0u8; SECTOR_SIZE];
    for index in 0..directory_sectors {
        ata::write_sector(DIRECTORY_START_LBA + index as u32, &zero_sector)?;
    }

    unsafe {
        STATE = state;
    }

    Ok(())
}

pub fn list(output: &mut [FileInfo]) -> Result<usize, FsError> {
    let state = require_mounted_state()?;

    for file in output.iter_mut() {
        *file = FileInfo::empty();
    }

    let mut found = 0usize;
    let mut sector = [0u8; SECTOR_SIZE];

    for sector_index in 0..state.directory_sectors as u32 {
        ata::read_sector(DIRECTORY_START_LBA + sector_index, &mut sector)?;

        for slot_index in 0..ENTRIES_PER_SECTOR {
            let offset = slot_index * ENTRY_SIZE;
            let entry = decode_entry(&sector[offset..offset + ENTRY_SIZE]);
            if !entry.used {
                continue;
            }

            if found < output.len() {
                output[found] = FileInfo {
                    name: entry.name,
                    name_len: entry.name_len,
                    size_bytes: entry.size_bytes,
                };
            }
            found += 1;
        }
    }

    Ok(found.min(output.len()))
}

pub fn read_file(name: &str, output: &mut [u8]) -> Result<ReadResult, FsError> {
    let state = require_mounted_state()?;
    let (name_bytes, name_len) = normalize_name(name)?;

    let Some(found) = find_entry(state, &name_bytes, name_len)? else {
        return Err(FsError::NotFound);
    };

    let total_size = found.entry.size_bytes as usize;
    if output.len() < total_size {
        return Err(FsError::BufferTooSmall);
    }

    if total_size == 0 {
        return Ok(ReadResult {
            total_size: 0,
            copied_size: 0,
        });
    }

    if found.entry.allocated_sectors == 0 {
        return Err(FsError::Corrupt);
    }

    let mut bytes_remaining = total_size;
    let mut copied = 0usize;
    let mut sector_buffer = [0u8; SECTOR_SIZE];

    for sector_offset in 0..found.entry.allocated_sectors {
        if bytes_remaining == 0 {
            break;
        }

        ata::read_sector(found.entry.start_lba + sector_offset, &mut sector_buffer)?;
        let chunk = bytes_remaining.min(SECTOR_SIZE);
        output[copied..copied + chunk].copy_from_slice(&sector_buffer[..chunk]);

        copied += chunk;
        bytes_remaining -= chunk;
    }

    Ok(ReadResult {
        total_size,
        copied_size: copied,
    })
}

pub fn write_file(name: &str, data: &[u8]) -> Result<(), FsError> {
    let mut state = require_mounted_state()?;
    let (name_bytes, name_len) = normalize_name(name)?;

    let required_sectors = sectors_for_bytes(data.len())?;
    let (existing, free_slot) = scan_directory(state, &name_bytes, name_len)?;

    if existing.is_none() && free_slot.is_none() {
        return Err(FsError::DirectoryFull);
    }

    if let Some(existing) = existing {
        let mut updated = existing.entry;

        if required_sectors > 0 {
            if required_sectors <= existing.entry.allocated_sectors {
                write_extent(existing.entry.start_lba, data)?;
            } else {
                let start_lba = allocate_extent(&mut state, required_sectors)?;
                write_extent(start_lba, data)?;
                updated.start_lba = start_lba;
                updated.allocated_sectors = required_sectors;
            }
        }

        if required_sectors == 0 {
            updated.start_lba = existing.entry.start_lba;
        }

        updated.used = true;
        updated.name_len = name_len;
        updated.name = name_bytes;
        updated.size_bytes = data.len() as u32;
        persist_entry(existing.sector_lba, existing.slot_index, updated)?;
        persist_state(state)?;
        return Ok(());
    }

    let slot = free_slot.ok_or(FsError::DirectoryFull)?;

    let (start_lba, allocated_sectors) = if required_sectors == 0 {
        (0u32, 0u32)
    } else {
        let start = allocate_extent(&mut state, required_sectors)?;
        write_extent(start, data)?;
        (start, required_sectors)
    };

    let new_entry = DirectoryEntry {
        used: true,
        name_len,
        start_lba,
        size_bytes: data.len() as u32,
        allocated_sectors,
        name: name_bytes,
    };

    persist_entry(slot.sector_lba, slot.slot_index, new_entry)?;
    state.file_count = state.file_count.saturating_add(1);
    persist_state(state)?;

    Ok(())
}

fn load_state_from_disk() -> Result<FsState, FsError> {
    let Some(disk) = ata::info() else {
        return Err(FsError::DiskUnavailable);
    };

    let mut sector = [0u8; SECTOR_SIZE];
    ata::read_sector(SUPERBLOCK_LBA, &mut sector)?;

    if sector[0..4] != FS_MAGIC {
        return Err(FsError::InvalidSuperblock);
    }

    let version = read_u16_le(&sector[4..6]);
    if version != FS_VERSION {
        return Err(FsError::InvalidSuperblock);
    }

    let directory_sectors = read_u16_le(&sector[6..8]);
    if directory_sectors == 0 {
        return Err(FsError::InvalidSuperblock);
    }

    let total_sectors = read_u32_le(&sector[8..12]);
    if total_sectors == 0 || total_sectors > disk.sectors {
        return Err(FsError::InvalidSuperblock);
    }

    let next_free_lba = read_u32_le(&sector[12..16]);
    let file_count = read_u32_le(&sector[16..20]);
    let data_start_lba = DIRECTORY_START_LBA + directory_sectors as u32;

    if data_start_lba >= total_sectors {
        return Err(FsError::InvalidSuperblock);
    }

    if next_free_lba < data_start_lba || next_free_lba > total_sectors {
        return Err(FsError::InvalidSuperblock);
    }

    Ok(FsState {
        mounted: true,
        total_sectors,
        directory_sectors,
        next_free_lba,
        file_count,
    })
}

fn require_mounted_state() -> Result<FsState, FsError> {
    let state = unsafe { STATE };
    if !state.mounted {
        return Err(FsError::NotMounted);
    }
    Ok(state)
}

fn persist_state(state: FsState) -> Result<(), FsError> {
    write_superblock(state)?;
    unsafe {
        STATE = state;
    }
    Ok(())
}

fn write_superblock(state: FsState) -> Result<(), FsError> {
    let mut sector = [0u8; SECTOR_SIZE];
    sector[0..4].copy_from_slice(&FS_MAGIC);
    write_u16_le(&mut sector[4..6], FS_VERSION);
    write_u16_le(&mut sector[6..8], state.directory_sectors);
    write_u32_le(&mut sector[8..12], state.total_sectors);
    write_u32_le(&mut sector[12..16], state.next_free_lba);
    write_u32_le(&mut sector[16..20], state.file_count);
    ata::write_sector(SUPERBLOCK_LBA, &sector)?;
    Ok(())
}

fn scan_directory(
    state: FsState,
    target_name: &[u8; MAX_FILENAME_LEN],
    target_len: u8,
) -> Result<(Option<EntryLocation>, Option<SlotLocation>), FsError> {
    let mut found = None;
    let mut free = None;

    let mut sector = [0u8; SECTOR_SIZE];

    for sector_index in 0..state.directory_sectors as u32 {
        let lba = DIRECTORY_START_LBA + sector_index;
        ata::read_sector(lba, &mut sector)?;

        for slot_index in 0..ENTRIES_PER_SECTOR {
            let offset = slot_index * ENTRY_SIZE;
            let entry = decode_entry(&sector[offset..offset + ENTRY_SIZE]);

            if entry.used {
                if names_equal(&entry.name, entry.name_len, target_name, target_len) {
                    found = Some(EntryLocation {
                        sector_lba: lba,
                        slot_index,
                        entry,
                    });
                    return Ok((found, free));
                }
            } else if free.is_none() {
                free = Some(SlotLocation {
                    sector_lba: lba,
                    slot_index,
                });
            }
        }
    }

    Ok((found, free))
}

fn find_entry(
    state: FsState,
    target_name: &[u8; MAX_FILENAME_LEN],
    target_len: u8,
) -> Result<Option<EntryLocation>, FsError> {
    let (found, _) = scan_directory(state, target_name, target_len)?;
    Ok(found)
}

fn persist_entry(sector_lba: u32, slot_index: usize, entry: DirectoryEntry) -> Result<(), FsError> {
    let mut sector = [0u8; SECTOR_SIZE];
    ata::read_sector(sector_lba, &mut sector)?;

    let offset = slot_index * ENTRY_SIZE;
    encode_entry(entry, &mut sector[offset..offset + ENTRY_SIZE]);

    ata::write_sector(sector_lba, &sector)?;
    Ok(())
}

fn allocate_extent(state: &mut FsState, sectors: u32) -> Result<u32, FsError> {
    if sectors == 0 {
        return Ok(0);
    }

    let start = state.next_free_lba;
    let end = start.checked_add(sectors).ok_or(FsError::NoSpace)?;

    if end > state.total_sectors {
        return Err(FsError::NoSpace);
    }

    state.next_free_lba = end;
    Ok(start)
}

fn write_extent(start_lba: u32, data: &[u8]) -> Result<(), FsError> {
    if data.is_empty() {
        return Ok(());
    }

    let sectors = sectors_for_bytes(data.len())?;
    let mut sector_buffer = [0u8; SECTOR_SIZE];

    for sector_index in 0..sectors {
        sector_buffer.fill(0);

        let byte_offset = sector_index as usize * SECTOR_SIZE;
        let remaining = data.len().saturating_sub(byte_offset);
        let chunk = remaining.min(SECTOR_SIZE);

        if chunk > 0 {
            sector_buffer[..chunk].copy_from_slice(&data[byte_offset..byte_offset + chunk]);
        }

        ata::write_sector(start_lba + sector_index, &sector_buffer)?;
    }

    Ok(())
}

fn sectors_for_bytes(len: usize) -> Result<u32, FsError> {
    if len == 0 {
        return Ok(0);
    }

    let sectors = (len + (SECTOR_SIZE - 1)) / SECTOR_SIZE;
    if sectors > u32::MAX as usize {
        return Err(FsError::NoSpace);
    }

    Ok(sectors as u32)
}

fn normalize_name(name: &str) -> Result<([u8; MAX_FILENAME_LEN], u8), FsError> {
    if name.is_empty() {
        return Err(FsError::NameEmpty);
    }

    let bytes = name.as_bytes();
    if bytes.len() > MAX_FILENAME_LEN {
        return Err(FsError::NameTooLong);
    }

    let mut encoded = [0u8; MAX_FILENAME_LEN];
    for (index, byte) in bytes.iter().copied().enumerate() {
        let valid = byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-');
        if !valid {
            return Err(FsError::NameInvalid);
        }
        encoded[index] = byte;
    }

    Ok((encoded, bytes.len() as u8))
}

fn names_equal(
    lhs_name: &[u8; MAX_FILENAME_LEN],
    lhs_len: u8,
    rhs_name: &[u8; MAX_FILENAME_LEN],
    rhs_len: u8,
) -> bool {
    if lhs_len != rhs_len {
        return false;
    }

    let len = lhs_len as usize;
    lhs_name[..len] == rhs_name[..len]
}

fn decode_entry(bytes: &[u8]) -> DirectoryEntry {
    let used = (bytes[0] & 0x01) != 0;
    let name_len = bytes[1].min(MAX_FILENAME_LEN as u8);
    let start_lba = read_u32_le(&bytes[4..8]);
    let size_bytes = read_u32_le(&bytes[8..12]);
    let allocated_sectors = read_u32_le(&bytes[12..16]);

    let mut name = [0u8; MAX_FILENAME_LEN];
    name.copy_from_slice(&bytes[16..64]);

    if !used {
        return DirectoryEntry::empty();
    }

    DirectoryEntry {
        used,
        name_len,
        start_lba,
        size_bytes,
        allocated_sectors,
        name,
    }
}

fn encode_entry(entry: DirectoryEntry, out: &mut [u8]) {
    out.fill(0);

    if !entry.used {
        return;
    }

    out[0] = 0x01;
    out[1] = entry.name_len.min(MAX_FILENAME_LEN as u8);
    write_u32_le(&mut out[4..8], entry.start_lba);
    write_u32_le(&mut out[8..12], entry.size_bytes);
    write_u32_le(&mut out[12..16], entry.allocated_sectors);
    out[16..64].copy_from_slice(&entry.name);
}

fn read_u16_le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn write_u16_le(out: &mut [u8], value: u16) {
    out.copy_from_slice(&value.to_le_bytes());
}

fn write_u32_le(out: &mut [u8], value: u32) {
    out.copy_from_slice(&value.to_le_bytes());
}
