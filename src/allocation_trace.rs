use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

pub struct CountingAllocator;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationSnapshot {
    pub allocations: u64,
    pub bytes: u64,
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

pub fn configure_from_env() {
    let enabled = std::env::var("CHELOTYPE_ALLOC_TRACE").ok().as_deref() == Some("1");
    ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        reset();
    }
}

pub fn snapshot() -> AllocationSnapshot {
    AllocationSnapshot {
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        bytes: BYTES.load(Ordering::Relaxed),
    }
}

pub fn reset() {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
}
