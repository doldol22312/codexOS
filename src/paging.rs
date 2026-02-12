extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::bootinfo;

pub const PAGE_SIZE: usize = 4096;
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

pub const USER_SPACE_BASE: usize = 0x4000_0000;
pub const USER_SPACE_LIMIT: usize = 0x8000_0000;
pub const USER_DEFAULT_STACK_BYTES: usize = 64 * 1024;
pub const USER_DEFAULT_STACK_TOP: usize = USER_SPACE_LIMIT;

const PDE_PRESENT: u32 = 1 << 0;
const PDE_WRITABLE: u32 = 1 << 1;
const PDE_USER: u32 = 1 << 2;
const PDE_CACHE_DISABLE: u32 = 1 << 4;

const PTE_PRESENT: u32 = 1 << 0;
const PTE_WRITABLE: u32 = 1 << 1;
const PTE_USER: u32 = 1 << 2;
const PTE_CACHE_DISABLE: u32 = 1 << 4;

const PAGE_ENTRY_ADDR_MASK: u32 = 0xFFFF_F000;

const CR0_PAGING_ENABLE: u32 = 1 << 31;
const CR4_PAGE_SIZE_EXTENSIONS: u32 = 1 << 4;

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageDirectory([u32; PAGE_DIRECTORY_ENTRIES]);

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageTable([u32; PAGE_TABLE_ENTRIES]);

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageFrame([u8; PAGE_SIZE]);

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressSpaceError {
    PagingUnavailable,
    InvalidRange,
    Overflow,
}

struct OwnedPageTable {
    pde_index: usize,
    table: Box<PageTable>,
}

pub struct AddressSpace {
    directory: Box<PageDirectory>,
    user_tables: Vec<OwnedPageTable>,
    user_frames: Vec<Box<PageFrame>>,
}

impl AddressSpace {
    pub fn new_user() -> Result<Self, AddressSpaceError> {
        if !INITIALIZED.load(Ordering::Acquire) {
            return Err(AddressSpaceError::PagingUnavailable);
        }

        let mut directory = Box::new(PageDirectory([0; PAGE_DIRECTORY_ENTRIES]));
        unsafe {
            core::ptr::copy_nonoverlapping(
                core::ptr::addr_of!(KERNEL_PAGE_DIRECTORY.0) as *const u32,
                directory.0.as_mut_ptr(),
                PAGE_DIRECTORY_ENTRIES,
            );
        }

        Ok(Self {
            directory,
            user_tables: Vec::new(),
            user_frames: Vec::new(),
        })
    }

    pub fn cr3(&self) -> u32 {
        self.directory.0.as_ptr() as u32
    }

    pub fn map_user_region(
        &mut self,
        virtual_base: usize,
        bytes: usize,
        writable: bool,
    ) -> Result<(), AddressSpaceError> {
        if bytes == 0 {
            return Ok(());
        }

        let end = virtual_base
            .checked_add(bytes)
            .ok_or(AddressSpaceError::Overflow)?;
        if !is_user_range(virtual_base, end) {
            return Err(AddressSpaceError::InvalidRange);
        }

        let page_start = align_down(virtual_base, PAGE_SIZE);
        let page_end = align_up(end, PAGE_SIZE).ok_or(AddressSpaceError::Overflow)?;
        let mut page = page_start;
        while page < page_end {
            self.map_user_page(page, writable)?;
            page = page
                .checked_add(PAGE_SIZE)
                .ok_or(AddressSpaceError::Overflow)?;
        }

        Ok(())
    }

    pub fn map_user_stack(
        &mut self,
        stack_top: usize,
        bytes: usize,
    ) -> Result<(), AddressSpaceError> {
        if bytes == 0 {
            return Ok(());
        }

        let stack_bottom = stack_top
            .checked_sub(bytes)
            .ok_or(AddressSpaceError::Overflow)?;
        let aligned_bottom = align_down(stack_bottom, PAGE_SIZE);
        let aligned_top = align_up(stack_top, PAGE_SIZE).ok_or(AddressSpaceError::Overflow)?;
        if !is_user_range(aligned_bottom, aligned_top) {
            return Err(AddressSpaceError::InvalidRange);
        }

        self.map_user_region(aligned_bottom, aligned_top - aligned_bottom, true)
    }

    pub fn copy_into_user(
        &self,
        virtual_base: usize,
        data: &[u8],
    ) -> Result<(), AddressSpaceError> {
        if data.is_empty() {
            return Ok(());
        }

        let end = virtual_base
            .checked_add(data.len())
            .ok_or(AddressSpaceError::Overflow)?;
        if !is_user_range(virtual_base, end) {
            return Err(AddressSpaceError::InvalidRange);
        }

        let mut copied = 0usize;
        while copied < data.len() {
            let addr = virtual_base + copied;
            let page_offset = addr & (PAGE_SIZE - 1);
            let chunk = (PAGE_SIZE - page_offset).min(data.len() - copied);

            let dst = self
                .translate_user_ptr(addr)
                .ok_or(AddressSpaceError::InvalidRange)?;
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr().add(copied), dst, chunk);
            }

            copied += chunk;
        }

        Ok(())
    }

    fn map_user_page(&mut self, virtual_page: usize, writable: bool) -> Result<(), AddressSpaceError> {
        if (virtual_page & (PAGE_SIZE - 1)) != 0 {
            return Err(AddressSpaceError::InvalidRange);
        }
        let page_end = virtual_page
            .checked_add(PAGE_SIZE)
            .ok_or(AddressSpaceError::Overflow)?;
        if !is_user_range(virtual_page, page_end) {
            return Err(AddressSpaceError::InvalidRange);
        }

        let pde_index = virtual_page >> 22;
        let pte_index = (virtual_page >> 12) & 0x3FF;

        let table = self.ensure_user_table(pde_index)?;
        if (table.0[pte_index] & PTE_PRESENT) != 0 {
            if (table.0[pte_index] & PTE_USER) == 0 {
                return Err(AddressSpaceError::InvalidRange);
            }
            if writable {
                table.0[pte_index] |= PTE_WRITABLE;
            }
            return Ok(());
        }

        let frame = Box::new(PageFrame([0; PAGE_SIZE]));
        let frame_phys = frame.0.as_ptr() as u32;
        let mut flags = PTE_PRESENT | PTE_USER;
        if writable {
            flags |= PTE_WRITABLE;
        }

        table.0[pte_index] = frame_phys | flags;
        self.user_frames.push(frame);
        Ok(())
    }

    fn ensure_user_table(&mut self, pde_index: usize) -> Result<&mut PageTable, AddressSpaceError> {
        if pde_index >= PAGE_DIRECTORY_ENTRIES {
            return Err(AddressSpaceError::InvalidRange);
        }

        if let Some(existing_index) = self
            .user_tables
            .iter()
            .position(|table| table.pde_index == pde_index)
        {
            return Ok(self.user_tables[existing_index].table.as_mut());
        }

        // User spaces can only add mappings where the kernel template has no entry.
        if (self.directory.0[pde_index] & PDE_PRESENT) != 0 {
            return Err(AddressSpaceError::InvalidRange);
        }

        let table = Box::new(PageTable([0; PAGE_TABLE_ENTRIES]));
        let table_phys = table.0.as_ptr() as u32;
        self.directory.0[pde_index] = table_phys | PDE_PRESENT | PDE_WRITABLE | PDE_USER;

        self.user_tables.push(OwnedPageTable { pde_index, table });
        let table_ref = self
            .user_tables
            .last_mut()
            .expect("user page table vector just pushed");
        Ok(table_ref.table.as_mut())
    }

    fn translate_user_ptr(&self, virtual_addr: usize) -> Option<*mut u8> {
        let end = virtual_addr.checked_add(1)?;
        if !is_user_range(virtual_addr, end) {
            return None;
        }

        let pde_index = virtual_addr >> 22;
        let pte_index = (virtual_addr >> 12) & 0x3FF;
        let page_offset = virtual_addr & (PAGE_SIZE - 1);

        let pde = self.directory.0.get(pde_index).copied()?;
        if (pde & (PDE_PRESENT | PDE_USER)) != (PDE_PRESENT | PDE_USER) {
            return None;
        }

        let table_ptr = (pde & PAGE_ENTRY_ADDR_MASK) as *const u32;
        let pte = unsafe { table_ptr.add(pte_index).read() };
        if (pte & (PTE_PRESENT | PTE_USER)) != (PTE_PRESENT | PTE_USER) {
            return None;
        }

        let physical = (pte & PAGE_ENTRY_ADDR_MASK) as usize + page_offset;
        Some(physical as *mut u8)
    }
}

static mut KERNEL_PAGE_DIRECTORY: PageDirectory = PageDirectory([0; PAGE_DIRECTORY_ENTRIES]);
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

        let directory_phys = core::ptr::addr_of!(KERNEL_PAGE_DIRECTORY.0) as u32;
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

pub fn kernel_directory_phys() -> usize {
    unsafe { core::ptr::addr_of!(KERNEL_PAGE_DIRECTORY.0) as usize }
}

pub fn switch_address_space(cr3: u32) {
    unsafe {
        if read_cr3() != cr3 {
            write_cr3(cr3);
        }
    }
}

pub fn use_kernel_address_space() {
    switch_address_space(kernel_directory_phys() as u32);
}

pub fn stats() -> PagingStats {
    let framebuffer = framebuffer_mapping();
    let mapped_regions = (IDENTITY_MAPPED_BYTES / PAGE_SIZE)
        + framebuffer
            .map(|mapping| div_ceil(mapping.bytes, PAGE_SIZE))
            .unwrap_or(0);
    let mapped_bytes = IDENTITY_MAPPED_BYTES + framebuffer.map(|mapping| mapping.bytes).unwrap_or(0);

    PagingStats {
        enabled: is_enabled(),
        directory_phys: kernel_directory_phys(),
        mapped_bytes,
        mapped_regions,
        page_size_bytes: PAGE_SIZE,
        framebuffer_mapped: framebuffer.is_some(),
        framebuffer_virtual: framebuffer.map(|mapping| mapping.virtual_base).unwrap_or(0),
        framebuffer_bytes: framebuffer.map(|mapping| mapping.bytes).unwrap_or(0),
    }
}

unsafe fn clear_tables() {
    let directory = core::ptr::addr_of_mut!(KERNEL_PAGE_DIRECTORY.0) as *mut u32;
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
    let directory = core::ptr::addr_of_mut!(KERNEL_PAGE_DIRECTORY.0) as *mut u32;

    for table in 0..IDENTITY_PAGE_TABLE_COUNT {
        let table_phys = core::ptr::addr_of!(IDENTITY_PAGE_TABLES[table].0) as u32;
        directory.add(table).write(table_phys | PDE_PRESENT | PDE_WRITABLE);

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

    let directory = core::ptr::addr_of_mut!(KERNEL_PAGE_DIRECTORY.0) as *mut u32;

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
const fn is_user_range(start: usize, end: usize) -> bool {
    start < end && start >= USER_SPACE_BASE && end <= USER_SPACE_LIMIT
}

#[inline]
const fn align_down(value: usize, align: usize) -> usize {
    value & !(align - 1)
}

#[inline]
fn align_up(value: usize, align: usize) -> Option<usize> {
    if value == 0 {
        return Some(0);
    }
    let add = align.checked_sub(1)?;
    let rounded = value.checked_add(add)?;
    Some(align_down(rounded, align))
}

#[inline]
const fn div_ceil(value: usize, divisor: usize) -> usize {
    if value == 0 {
        0
    } else {
        (value - 1) / divisor + 1
    }
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
unsafe fn read_cr3() -> u32 {
    let value: u32;
    asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
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
