extern crate alloc;

use alloc::vec;

use crate::{fs, paging, task};

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELF_CLASS_32: u8 = 1;
const ELF_DATA_LITTLE_ENDIAN: u8 = 1;
const ELF_VERSION_CURRENT: u8 = 1;

const ET_EXEC: u16 = 2;
const EM_386: u16 = 3;
const PT_LOAD: u32 = 1;
const PF_W: u32 = 1 << 1;

const MAX_ELF_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfError {
    Fs(fs::FsError),
    FileNotFound,
    FileTooLarge,
    Truncated,
    InvalidFormat,
    Unsupported,
    AddressSpace,
    SpawnFailed,
}

impl ElfError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fs(error) => error.as_str(),
            Self::FileNotFound => "file not found",
            Self::FileTooLarge => "ELF file too large",
            Self::Truncated => "truncated ELF file",
            Self::InvalidFormat => "invalid ELF format",
            Self::Unsupported => "unsupported ELF",
            Self::AddressSpace => "address space setup failed",
            Self::SpawnFailed => "failed to spawn process",
        }
    }
}

impl From<fs::FsError> for ElfError {
    fn from(value: fs::FsError) -> Self {
        Self::Fs(value)
    }
}

pub fn spawn_from_fs(path: &str) -> Result<task::TaskId, ElfError> {
    let file_size = lookup_file_size(path)?;
    if file_size == 0 || file_size > MAX_ELF_FILE_BYTES {
        return Err(ElfError::FileTooLarge);
    }

    let mut image = vec![0u8; file_size];
    let result = fs::read_file(path, &mut image)?;
    if result.total_size != file_size || result.copied_size != file_size {
        return Err(ElfError::Truncated);
    }

    let loaded = load_elf(&image)?;
    task::spawn_user(loaded.entry_point, loaded.user_stack_top, loaded.address_space)
        .map_err(|_| ElfError::SpawnFailed)
}

struct LoadedElf {
    entry_point: u32,
    user_stack_top: u32,
    address_space: paging::AddressSpace,
}

fn load_elf(image: &[u8]) -> Result<LoadedElf, ElfError> {
    if image.len() < 52 {
        return Err(ElfError::Truncated);
    }

    if image[0..4] != ELF_MAGIC {
        return Err(ElfError::InvalidFormat);
    }
    if image[4] != ELF_CLASS_32 || image[5] != ELF_DATA_LITTLE_ENDIAN || image[6] != ELF_VERSION_CURRENT {
        return Err(ElfError::Unsupported);
    }

    let e_type = read_u16(image, 16)?;
    let e_machine = read_u16(image, 18)?;
    let e_version = read_u32(image, 20)?;
    let e_entry = read_u32(image, 24)?;
    let e_phoff = read_u32(image, 28)? as usize;
    let e_ehsize = read_u16(image, 40)? as usize;
    let e_phentsize = read_u16(image, 42)? as usize;
    let e_phnum = read_u16(image, 44)? as usize;

    if e_type != ET_EXEC || e_machine != EM_386 || e_version != 1 {
        return Err(ElfError::Unsupported);
    }
    if e_ehsize < 52 || e_phentsize < 32 || e_phnum == 0 {
        return Err(ElfError::InvalidFormat);
    }

    let phdr_bytes = e_phentsize
        .checked_mul(e_phnum)
        .ok_or(ElfError::InvalidFormat)?;
    let phdr_end = e_phoff.checked_add(phdr_bytes).ok_or(ElfError::InvalidFormat)?;
    if phdr_end > image.len() {
        return Err(ElfError::Truncated);
    }

    let entry = e_entry as usize;
    if !(paging::USER_SPACE_BASE..paging::USER_SPACE_LIMIT).contains(&entry) {
        return Err(ElfError::InvalidFormat);
    }

    let mut address_space = paging::AddressSpace::new_user().map_err(|_| ElfError::AddressSpace)?;
    let mut loaded_segment = false;

    for index in 0..e_phnum {
        let offset = e_phoff + index * e_phentsize;
        let p_type = read_u32(image, offset)?;
        if p_type != PT_LOAD {
            continue;
        }

        loaded_segment = true;

        let p_offset = read_u32(image, offset + 4)? as usize;
        let p_vaddr = read_u32(image, offset + 8)? as usize;
        let p_filesz = read_u32(image, offset + 16)? as usize;
        let p_memsz = read_u32(image, offset + 20)? as usize;
        let p_flags = read_u32(image, offset + 24)?;

        if p_memsz < p_filesz {
            return Err(ElfError::InvalidFormat);
        }
        if p_memsz == 0 {
            continue;
        }

        let segment_end = p_vaddr.checked_add(p_memsz).ok_or(ElfError::InvalidFormat)?;
        if p_vaddr < paging::USER_SPACE_BASE || segment_end > paging::USER_SPACE_LIMIT {
            return Err(ElfError::InvalidFormat);
        }

        address_space
            .map_user_region(p_vaddr, p_memsz, (p_flags & PF_W) != 0)
            .map_err(|_| ElfError::AddressSpace)?;

        if p_filesz > 0 {
            let file_end = p_offset.checked_add(p_filesz).ok_or(ElfError::InvalidFormat)?;
            if file_end > image.len() {
                return Err(ElfError::Truncated);
            }

            address_space
                .copy_into_user(p_vaddr, &image[p_offset..file_end])
                .map_err(|_| ElfError::AddressSpace)?;
        }
    }

    if !loaded_segment {
        return Err(ElfError::InvalidFormat);
    }

    address_space
        .map_user_stack(paging::USER_DEFAULT_STACK_TOP, paging::USER_DEFAULT_STACK_BYTES)
        .map_err(|_| ElfError::AddressSpace)?;

    Ok(LoadedElf {
        entry_point: e_entry,
        user_stack_top: paging::USER_DEFAULT_STACK_TOP as u32,
        address_space,
    })
}

fn lookup_file_size(path: &str) -> Result<usize, ElfError> {
    let mut entries = [fs::FileInfo::empty(); MAX_DIRECTORY_ENTRIES];
    let count = fs::list(&mut entries)?;
    for entry in entries.iter().take(count) {
        if entry.name_str() == path {
            return Ok(entry.size_bytes as usize);
        }
    }
    Err(ElfError::FileNotFound)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ElfError> {
    let end = offset.checked_add(2).ok_or(ElfError::InvalidFormat)?;
    if end > bytes.len() {
        return Err(ElfError::Truncated);
    }
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ElfError> {
    let end = offset.checked_add(4).ok_or(ElfError::InvalidFormat)?;
    if end > bytes.len() {
        return Err(ElfError::Truncated);
    }
    Ok(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}
