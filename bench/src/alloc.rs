//! A global allocator that tracks the heap in use and its peak, so that
//! the export scenario can report memory growth without a container.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static IN_USE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// The system allocator with two counters.
pub struct Counting;

// SAFETY: every method forwards to the system allocator with the layout it
// was given; the counters do not affect the memory handed out.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = IN_USE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        IN_USE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let out = unsafe { System.realloc(ptr, layout, new_size) };
        if !out.is_null() {
            if new_size >= layout.size() {
                let grown = new_size - layout.size();
                let now = IN_USE.fetch_add(grown, Ordering::Relaxed) + grown;
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                IN_USE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        out
    }
}

/// The heap in use now, in bytes.
pub fn in_use() -> usize {
    IN_USE.load(Ordering::Relaxed)
}

/// Starts a measurement: the peak becomes the heap in use now. Returns that
/// value.
pub fn reset_peak() -> usize {
    let now = in_use();
    PEAK.store(now, Ordering::Relaxed);
    now
}

/// The peak since the last [`reset_peak`], in bytes.
pub fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}
