use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

const PAGE_DIRECTORY_ENTRIES: usize = 1024;
const LARGE_PAGE_SIZE: usize = 4 * 1024 * 1024;
const IDENTITY_MAPPED_BYTES: usize = 128 * 1024 * 1024;
const IDENTITY_PDE_COUNT: usize = IDENTITY_MAPPED_BYTES / LARGE_PAGE_SIZE;

const PDE_PRESENT: u32 = 1 << 0;
const PDE_WRITABLE: u32 = 1 << 1;
const PDE_LARGE_PAGE: u32 = 1 << 7;

const CR0_PAGING_ENABLE: u32 = 1 << 31;
const CR4_PAGE_SIZE_EXTENSIONS: u32 = 1 << 4;

#[repr(C, align(4096))]
struct PageDirectory([u32; PAGE_DIRECTORY_ENTRIES]);

static mut PAGE_DIRECTORY: PageDirectory = PageDirectory([0; PAGE_DIRECTORY_ENTRIES]);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct PagingStats {
    pub enabled: bool,
    pub directory_phys: usize,
    pub mapped_bytes: usize,
    pub mapped_regions: usize,
    pub page_size_bytes: usize,
}

pub fn init() {
    if INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    unsafe {
        let directory = core::ptr::addr_of_mut!(PAGE_DIRECTORY.0) as *mut u32;
        for index in 0..PAGE_DIRECTORY_ENTRIES {
            directory.add(index).write(0);
        }

        for index in 0..IDENTITY_PDE_COUNT {
            let physical_base = (index * LARGE_PAGE_SIZE) as u32;
            directory
                .add(index)
                .write(physical_base | PDE_PRESENT | PDE_WRITABLE | PDE_LARGE_PAGE);
        }

        let directory_phys = core::ptr::addr_of!(PAGE_DIRECTORY.0) as u32;
        write_cr3(directory_phys);

        let cr4 = read_cr4() | CR4_PAGE_SIZE_EXTENSIONS;
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

pub fn stats() -> PagingStats {
    PagingStats {
        enabled: is_enabled(),
        directory_phys: unsafe { core::ptr::addr_of!(PAGE_DIRECTORY.0) as usize },
        mapped_bytes: IDENTITY_MAPPED_BYTES,
        mapped_regions: IDENTITY_PDE_COUNT,
        page_size_bytes: LARGE_PAGE_SIZE,
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
