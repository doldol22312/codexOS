use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::null_mut;

pub struct BumpAllocator {
    next: UnsafeCell<usize>,
}

unsafe impl Sync for BumpAllocator {}

impl BumpAllocator {
    pub const fn new() -> Self {
        Self {
            next: UnsafeCell::new(0),
        }
    }
}

unsafe extern "C" {
    static __heap_start: u8;
    static __heap_end: u8;
}

#[global_allocator]
static GLOBAL_ALLOCATOR: BumpAllocator = BumpAllocator::new();

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let heap_start = core::ptr::addr_of!(__heap_start) as usize;
        let heap_end = core::ptr::addr_of!(__heap_end) as usize;
        let size = layout.size().max(1);
        let align = layout.align();

        let next_ptr = self.next.get();
        let current = if *next_ptr == 0 { heap_start } else { *next_ptr };
        let alloc_start = align_up(current, align);
        let Some(alloc_end) = alloc_start.checked_add(size) else {
            return null_mut();
        };

        if alloc_end > heap_end {
            return null_mut();
        }

        *next_ptr = alloc_end;
        alloc_start as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[derive(Clone, Copy)]
pub struct HeapStats {
    pub start: usize,
    pub end: usize,
    pub used: usize,
    pub total: usize,
    pub remaining: usize,
}

#[derive(Clone, Copy)]
pub struct MemTestResult {
    pub start: usize,
    pub tested: usize,
    pub failures: u32,
    pub first_failure_addr: Option<usize>,
}

pub fn stats() -> HeapStats {
    unsafe {
        let heap_start = core::ptr::addr_of!(__heap_start) as usize;
        let heap_end = core::ptr::addr_of!(__heap_end) as usize;
        let next = *GLOBAL_ALLOCATOR.next.get();
        let current = if next == 0 { heap_start } else { next };

        HeapStats {
            start: heap_start,
            end: heap_end,
            used: current.saturating_sub(heap_start),
            total: heap_end.saturating_sub(heap_start),
            remaining: heap_end.saturating_sub(current),
        }
    }
}

pub fn memtest(requested_bytes: usize) -> MemTestResult {
    const PATTERN_A: u32 = 0xAA55_AA55;
    const PATTERN_B: u32 = 0x55AA_55AA;

    let heap = stats();
    let start = heap.start + heap.used;
    let tested = requested_bytes.min(heap.remaining);
    let words = tested / 4;
    let tail = tested % 4;

    let mut failures = 0u32;
    let mut first_failure_addr = None;

    unsafe {
        for index in 0..words {
            let pointer = (start + index * 4) as *mut u32;
            pointer.write_volatile(PATTERN_A);
        }

        for index in 0..words {
            let pointer = (start + index * 4) as *const u32;
            if pointer.read_volatile() != PATTERN_A {
                failures = failures.saturating_add(1);
                if first_failure_addr.is_none() {
                    first_failure_addr = Some(start + index * 4);
                }
            }
        }

        for index in 0..words {
            let pointer = (start + index * 4) as *mut u32;
            pointer.write_volatile(PATTERN_B);
        }

        for index in 0..words {
            let pointer = (start + index * 4) as *const u32;
            if pointer.read_volatile() != PATTERN_B {
                failures = failures.saturating_add(1);
                if first_failure_addr.is_none() {
                    first_failure_addr = Some(start + index * 4);
                }
            }
        }

        for offset in 0..tail {
            let pointer = (start + words * 4 + offset) as *mut u8;
            pointer.write_volatile(0xA5);
            if (pointer as *const u8).read_volatile() != 0xA5 {
                failures = failures.saturating_add(1);
                if first_failure_addr.is_none() {
                    first_failure_addr = Some(start + words * 4 + offset);
                }
            }
        }
    }

    MemTestResult {
        start,
        tested,
        failures,
        first_failure_addr,
    }
}

const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}
