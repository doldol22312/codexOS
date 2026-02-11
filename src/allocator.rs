use core::alloc::{GlobalAlloc, Layout};
use core::arch::asm;
use core::cell::UnsafeCell;
use core::ptr::null_mut;

pub struct FreeListAllocator {
    head: UnsafeCell<*mut FreeBlock>,
}

unsafe impl Sync for FreeListAllocator {}

impl FreeListAllocator {
    pub const fn new() -> Self {
        Self {
            head: UnsafeCell::new(null_mut()),
        }
    }

    unsafe fn ensure_initialized(&self) {
        if !(*self.head.get()).is_null() {
            return;
        }

        let heap_start = core::ptr::addr_of!(__heap_start) as usize;
        let heap_end = core::ptr::addr_of!(__heap_end) as usize;

        let aligned_start = align_up(heap_start, core::mem::align_of::<FreeBlock>());
        if heap_end <= aligned_start + MIN_FREE_BLOCK_SIZE {
            return;
        }

        let head = aligned_start as *mut FreeBlock;
        (*head).size = heap_end - aligned_start;
        (*head).next = null_mut();
        *self.head.get() = head;
    }

    unsafe fn alloc_internal(&self, layout: Layout) -> *mut u8 {
        self.ensure_initialized();

        let mut previous = null_mut::<FreeBlock>();
        let mut current = *self.head.get();

        let requested_size = layout.size().max(1);
        let requested_align = layout.align().max(core::mem::align_of::<usize>());

        while !current.is_null() {
            let block_start = current as usize;
            let block_size = (*current).size;
            let Some(block_end) = block_start.checked_add(block_size) else {
                return null_mut();
            };

            let Some(candidate) = block_start.checked_add(core::mem::size_of::<AllocationHeader>())
            else {
                return null_mut();
            };
            let payload_start = align_up(candidate, requested_align);
            let header_addr = payload_start - core::mem::size_of::<AllocationHeader>();
            let Some(allocation_end) = payload_start.checked_add(requested_size) else {
                previous = current;
                current = (*current).next;
                continue;
            };

            if allocation_end > block_end {
                previous = current;
                current = (*current).next;
                continue;
            }

            let next = (*current).next;
            let trailing_size = block_end - allocation_end;
            let mut reserved_end = allocation_end;

            if trailing_size >= MIN_FREE_BLOCK_SIZE {
                let trailing_block = allocation_end as *mut FreeBlock;
                (*trailing_block).size = trailing_size;
                (*trailing_block).next = next;

                if previous.is_null() {
                    *self.head.get() = trailing_block;
                } else {
                    (*previous).next = trailing_block;
                }
            } else {
                reserved_end = block_end;
                if previous.is_null() {
                    *self.head.get() = next;
                } else {
                    (*previous).next = next;
                }
            }

            let header = header_addr as *mut AllocationHeader;
            (*header).block_start = block_start;
            (*header).block_size = reserved_end - block_start;

            return payload_start as *mut u8;
        }

        null_mut()
    }

    unsafe fn dealloc_internal(&self, ptr: *mut u8) {
        let pointer = ptr as usize;
        let Some(header_addr) = pointer.checked_sub(core::mem::size_of::<AllocationHeader>()) else {
            return;
        };

        let header = header_addr as *const AllocationHeader;
        let block_start = (*header).block_start;
        let block_size = (*header).block_size;

        if block_size < MIN_FREE_BLOCK_SIZE {
            return;
        }

        self.insert_and_coalesce(block_start, block_size);
    }

    unsafe fn insert_and_coalesce(&self, block_start: usize, block_size: usize) {
        self.ensure_initialized();

        let mut previous = null_mut::<FreeBlock>();
        let mut current = *self.head.get();

        while !current.is_null() && (current as usize) < block_start {
            previous = current;
            current = (*current).next;
        }

        let inserted = block_start as *mut FreeBlock;
        (*inserted).size = block_size;
        (*inserted).next = current;

        if previous.is_null() {
            *self.head.get() = inserted;
        } else {
            (*previous).next = inserted;
        }

        coalesce_with_next(inserted);
        if !previous.is_null() {
            coalesce_with_next(previous);
        }
    }

    unsafe fn free_bytes(&self) -> usize {
        self.ensure_initialized();

        let mut total_free = 0usize;
        let mut current = *self.head.get();

        while !current.is_null() {
            total_free = total_free.saturating_add((*current).size);
            current = (*current).next;
        }

        total_free
    }
}

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

#[repr(C)]
struct AllocationHeader {
    block_start: usize,
    block_size: usize,
}

const MIN_FREE_BLOCK_SIZE: usize = core::mem::size_of::<FreeBlock>();

unsafe fn coalesce_with_next(block: *mut FreeBlock) {
    let next = (*block).next;
    if next.is_null() {
        return;
    }

    let block_end = (block as usize).saturating_add((*block).size);
    if block_end == next as usize {
        (*block).size = (*block).size.saturating_add((*next).size);
        (*block).next = (*next).next;
    }
}

struct IrqGuard {
    was_enabled: bool,
}

impl IrqGuard {
    unsafe fn new() -> Self {
        let flags: u32;
        asm!("pushfd", "pop {}", out(reg) flags, options(preserves_flags));

        asm!("cli", options(nomem, nostack));

        Self {
            was_enabled: (flags & (1 << 9)) != 0,
        }
    }
}

impl Drop for IrqGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            unsafe {
                asm!("sti", options(nomem, nostack));
            }
        }
    }
}

unsafe extern "C" {
    static __heap_start: u8;
    static __heap_end: u8;
}

#[global_allocator]
static GLOBAL_ALLOCATOR: FreeListAllocator = FreeListAllocator::new();

unsafe impl GlobalAlloc for FreeListAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = IrqGuard::new();
        self.alloc_internal(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }

        let _guard = IrqGuard::new();
        self.dealloc_internal(ptr);
    }
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
        let _guard = IrqGuard::new();

        let heap_start = core::ptr::addr_of!(__heap_start) as usize;
        let heap_end = core::ptr::addr_of!(__heap_end) as usize;
        let total = heap_end.saturating_sub(heap_start);
        let free = GLOBAL_ALLOCATOR.free_bytes();

        HeapStats {
            start: heap_start,
            end: heap_end,
            used: total.saturating_sub(free),
            total,
            remaining: free,
        }
    }
}

pub fn memtest(requested_bytes: usize) -> MemTestResult {
    const PATTERN_A: u32 = 0xAA55_AA55;
    const PATTERN_B: u32 = 0x55AA_55AA;

    let heap = stats();
    let mut tested = requested_bytes.max(1);
    tested = tested.min(heap.total.saturating_sub(core::mem::size_of::<AllocationHeader>()));

    if tested == 0 {
        return MemTestResult {
            start: heap.start,
            tested: 0,
            failures: 0,
            first_failure_addr: None,
        };
    }

    let pointer = loop {
        let layout = match Layout::from_size_align(tested, 1) {
            Ok(value) => value,
            Err(_) => {
                return MemTestResult {
                    start: heap.start,
                    tested: 0,
                    failures: 0,
                    first_failure_addr: None,
                };
            }
        };

        let ptr = unsafe { GLOBAL_ALLOCATOR.alloc(layout) };
        if !ptr.is_null() {
            break ptr;
        }

        tested /= 2;
        if tested == 0 {
            return MemTestResult {
                start: heap.start,
                tested: 0,
                failures: 0,
                first_failure_addr: None,
            };
        }
    };

    let start = pointer as usize;
    let words = tested / 4;
    let tail = tested % 4;

    let mut failures = 0u32;
    let mut first_failure_addr = None;

    unsafe {
        for index in 0..words {
            let location = pointer.add(index * 4) as *mut u32;
            location.write_volatile(PATTERN_A);
        }

        for index in 0..words {
            let location = pointer.add(index * 4) as *const u32;
            if location.read_volatile() != PATTERN_A {
                failures = failures.saturating_add(1);
                if first_failure_addr.is_none() {
                    first_failure_addr = Some(start + index * 4);
                }
            }
        }

        for index in 0..words {
            let location = pointer.add(index * 4) as *mut u32;
            location.write_volatile(PATTERN_B);
        }

        for index in 0..words {
            let location = pointer.add(index * 4) as *const u32;
            if location.read_volatile() != PATTERN_B {
                failures = failures.saturating_add(1);
                if first_failure_addr.is_none() {
                    first_failure_addr = Some(start + index * 4);
                }
            }
        }

        for offset in 0..tail {
            let location = pointer.add(words * 4 + offset);
            location.write_volatile(0xA5);
            if location.read_volatile() != 0xA5 {
                failures = failures.saturating_add(1);
                if first_failure_addr.is_none() {
                    first_failure_addr = Some(start + words * 4 + offset);
                }
            }
        }

        let layout = Layout::from_size_align_unchecked(tested, 1);
        GLOBAL_ALLOCATOR.dealloc(pointer, layout);
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
