use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::bootinfo;

const PAGE_SIZE: usize = 4096;
const PAGE_TABLE_ENTRIES: usize = 1024;
const PAGE_DIRECTORY_ENTRIES: usize = 1024;

const IDENTITY_MAPPED_BYTES: usize = 256 * 1024 * 1024;
const IDENTITY_PAGE_TABLE_COUNT: usize =
    IDENTITY_MAPPED_BYTES / (PAGE_SIZE * PAGE_TABLE_ENTRIES);

const FRAMEBUFFER_VIRT_BASE: usize = 0xE000_0000;
const FRAMEBUFFER_PDE_BASE: usize = FRAMEBUFFER_VIRT_BASE >> 22;
const MAX_FRAMEBUFFER_MAPPED_BYTES: usize = 64 * 1024 * 1024;
const FRAMEBUFFER_PAGE_TABLE_COUNT: usize =
    MAX_FRAMEBUFFER_MAPPED_BYTES / (PAGE_SIZE * PAGE_TABLE_ENTRIES);

const PDE_PRESENT: u32 = 1 << 0;
const PDE_WRITABLE: u32 = 1 << 1;
const PDE_CACHE_DISABLE: u32 = 1 << 4;

const PTE_PRESENT: u32 = 1 << 0;
const PTE_WRITABLE: u32 = 1 << 1;
const PTE_CACHE_DISABLE: u32 = 1 << 4;

const CR0_PAGING_ENABLE: u32 = 1 << 31;
const CR4_PAGE_SIZE_EXTENSIONS: u32 = 1 << 4;

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageDirectory([u32; PAGE_DIRECTORY_ENTRIES]);

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageTable([u32; PAGE_TABLE_ENTRIES]);

#[derive(Clone, Copy)]
pub struct FramebufferMapping {
    pub physical_base: usize,
    pub virtual_base: usize,
    pub bytes: usize,
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: usize,
    pub font_phys: usize,
    pub font_bytes: usize,
    pub font_height: usize,
}

#[derive(Clone, Copy)]
pub struct PagingStats {
    pub enabled: bool,
    pub directory_phys: usize,
    pub mapped_bytes: usize,
    pub mapped_regions: usize,
    pub page_size_bytes: usize,
    pub framebuffer_mapped: bool,
    pub framebuffer_virtual: usize,
    pub framebuffer_bytes: usize,
}

static mut PAGE_DIRECTORY: PageDirectory = PageDirectory([0; PAGE_DIRECTORY_ENTRIES]);
static mut IDENTITY_PAGE_TABLES: [PageTable; IDENTITY_PAGE_TABLE_COUNT] =
    [PageTable([0; PAGE_TABLE_ENTRIES]); IDENTITY_PAGE_TABLE_COUNT];
static mut FRAMEBUFFER_PAGE_TABLES: [PageTable; FRAMEBUFFER_PAGE_TABLE_COUNT] =
    [PageTable([0; PAGE_TABLE_ENTRIES]); FRAMEBUFFER_PAGE_TABLE_COUNT];

static mut FRAMEBUFFER_MAP: Option<FramebufferMapping> = None;
static INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    if INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    unsafe {
        clear_tables();
        map_identity();
        FRAMEBUFFER_MAP = map_framebuffer();

        let directory_phys = core::ptr::addr_of!(PAGE_DIRECTORY.0) as u32;
        write_cr3(directory_phys);

        // We now use 4 KiB pages instead of 4 MiB pages.
        let cr4 = read_cr4() & !CR4_PAGE_SIZE_EXTENSIONS;
        write_cr4(cr4);

        let cr0 = read_cr0() | CR0_PAGING_ENABLE;
        write_cr0(cr0);
    }

    INITIALIZED.store(true, Ordering::Release);
}

pub fn is_enabled() -> bool {
    unsafe { (read_cr0() & CR0_PAGING_ENABLE) != 0 }
}

pub fn page_fault_address() -> usize {
    unsafe { read_cr2() as usize }
}

pub fn framebuffer_mapping() -> Option<FramebufferMapping> {
    unsafe { FRAMEBUFFER_MAP }
}

pub fn stats() -> PagingStats {
    let framebuffer = framebuffer_mapping();
    let mapped_regions = (IDENTITY_MAPPED_BYTES / PAGE_SIZE)
        + framebuffer.map(|mapping| div_ceil(mapping.bytes, PAGE_SIZE)).unwrap_or(0);
    let mapped_bytes = IDENTITY_MAPPED_BYTES + framebuffer.map(|mapping| mapping.bytes).unwrap_or(0);

    PagingStats {
        enabled: is_enabled(),
        directory_phys: unsafe { core::ptr::addr_of!(PAGE_DIRECTORY.0) as usize },
        mapped_bytes,
        mapped_regions,
        page_size_bytes: PAGE_SIZE,
        framebuffer_mapped: framebuffer.is_some(),
        framebuffer_virtual: framebuffer.map(|mapping| mapping.virtual_base).unwrap_or(0),
        framebuffer_bytes: framebuffer.map(|mapping| mapping.bytes).unwrap_or(0),
    }
}

unsafe fn clear_tables() {
    let directory = core::ptr::addr_of_mut!(PAGE_DIRECTORY.0) as *mut u32;
    for index in 0..PAGE_DIRECTORY_ENTRIES {
        directory.add(index).write(0);
    }

    for table in 0..IDENTITY_PAGE_TABLE_COUNT {
        let entries = core::ptr::addr_of_mut!(IDENTITY_PAGE_TABLES[table].0) as *mut u32;
        for entry in 0..PAGE_TABLE_ENTRIES {
            entries.add(entry).write(0);
        }
    }

    for table in 0..FRAMEBUFFER_PAGE_TABLE_COUNT {
        let entries = core::ptr::addr_of_mut!(FRAMEBUFFER_PAGE_TABLES[table].0) as *mut u32;
        for entry in 0..PAGE_TABLE_ENTRIES {
            entries.add(entry).write(0);
        }
    }
}

unsafe fn map_identity() {
    let directory = core::ptr::addr_of_mut!(PAGE_DIRECTORY.0) as *mut u32;

    for table in 0..IDENTITY_PAGE_TABLE_COUNT {
        let table_phys = core::ptr::addr_of!(IDENTITY_PAGE_TABLES[table].0) as u32;
        directory
            .add(table)
            .write(table_phys | PDE_PRESENT | PDE_WRITABLE);

        let entries = core::ptr::addr_of_mut!(IDENTITY_PAGE_TABLES[table].0) as *mut u32;
        for entry in 0..PAGE_TABLE_ENTRIES {
            let page_index = table * PAGE_TABLE_ENTRIES + entry;
            let physical = (page_index * PAGE_SIZE) as u32;
            entries
                .add(entry)
                .write(physical | PTE_PRESENT | PTE_WRITABLE);
        }
    }
}

unsafe fn map_framebuffer() -> Option<FramebufferMapping> {
    let info = bootinfo::video_info()?;
    if !info.vbe_active() {
        return None;
    }

    if info.framebuffer_phys == 0 || info.width == 0 || info.height == 0 || info.pitch == 0 {
        return None;
    }

    let bytes = info.pitch.checked_mul(info.height)?;
    if bytes == 0 || bytes > MAX_FRAMEBUFFER_MAPPED_BYTES {
        return None;
    }

    let aligned_phys = align_down(info.framebuffer_phys, PAGE_SIZE);
    let offset = info.framebuffer_phys - aligned_phys;
    let required = bytes.checked_add(offset)?;
    let page_count = div_ceil(required, PAGE_SIZE);
    let table_count = div_ceil(page_count, PAGE_TABLE_ENTRIES);
    if table_count == 0 || table_count > FRAMEBUFFER_PAGE_TABLE_COUNT {
        return None;
    }

    let directory = core::ptr::addr_of_mut!(PAGE_DIRECTORY.0) as *mut u32;

    for table in 0..table_count {
        let pde_index = FRAMEBUFFER_PDE_BASE + table;
        if pde_index >= PAGE_DIRECTORY_ENTRIES {
            return None;
        }

        let table_phys = core::ptr::addr_of!(FRAMEBUFFER_PAGE_TABLES[table].0) as u32;
        directory
            .add(pde_index)
            .write(table_phys | PDE_PRESENT | PDE_WRITABLE | PDE_CACHE_DISABLE);

        let entries = core::ptr::addr_of_mut!(FRAMEBUFFER_PAGE_TABLES[table].0) as *mut u32;
        for entry in 0..PAGE_TABLE_ENTRIES {
            let page_index = table * PAGE_TABLE_ENTRIES + entry;
            if page_index >= page_count {
                entries.add(entry).write(0);
                continue;
            }

            let physical = aligned_phys + page_index * PAGE_SIZE;
            entries
                .add(entry)
                .write((physical as u32) | PTE_PRESENT | PTE_WRITABLE | PTE_CACHE_DISABLE);
        }
    }

    Some(FramebufferMapping {
        physical_base: info.framebuffer_phys,
        virtual_base: FRAMEBUFFER_VIRT_BASE + offset,
        bytes,
        width: info.width,
        height: info.height,
        pitch: info.pitch,
        bpp: info.bpp,
        font_phys: info.font_phys,
        font_bytes: info.font_bytes,
        font_height: info.font_height.max(1),
    })
}

#[inline]
const fn align_down(value: usize, align: usize) -> usize {
    value & !(align - 1)
}

#[inline]
const fn div_ceil(value: usize, divisor: usize) -> usize {
    if value == 0 { 0 } else { (value - 1) / divisor + 1 }
}

#[inline]
unsafe fn read_cr0() -> u32 {
    let value: u32;
    asm!("mov {}, cr0", out(reg) value, options(nomem, nostack, preserves_flags));
    value
}

#[inline]
unsafe fn write_cr0(value: u32) {
    asm!("mov cr0, {}", in(reg) value, options(nomem, nostack, preserves_flags));
}

#[inline]
unsafe fn read_cr2() -> u32 {
    let value: u32;
    asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
    value
}

#[inline]
unsafe fn write_cr3(value: u32) {
    asm!("mov cr3, {}", in(reg) value, options(nomem, nostack, preserves_flags));
}

#[inline]
unsafe fn read_cr4() -> u32 {
    let value: u32;
    asm!("mov {}, cr4", out(reg) value, options(nomem, nostack, preserves_flags));
    value
}

#[inline]
unsafe fn write_cr4(value: u32) {
    asm!("mov cr4, {}", in(reg) value, options(nomem, nostack, preserves_flags));
}
