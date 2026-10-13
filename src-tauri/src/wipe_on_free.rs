//! A global allocator that wipes every block before handing it back to the system.
//!
//! Why: passwords and Hidden Field values cross Tauri's IPC layer as ordinary `String`s and
//! `serde_json::Value`s before they reach our `Zeroizing` types (and again on the way out), and
//! libraries such as zxcvbn copy what they are given. None of those buffers can be wiped from
//! outside, but all of them are freed through the global allocator, so wiping there covers them
//! in one place. This covers the Rust heap of this process only: not the stack, not memory the
//! WebView or AppKit allocates (ADR-0003).

use std::alloc::{GlobalAlloc, Layout, System};

pub struct WipeOnFree;

// SAFETY: every call is forwarded to `System` unchanged. The only extra work is writing zeros
// into a block the caller is giving up, inside the bounds `layout` describes, before it is freed.
unsafe impl GlobalAlloc for WipeOnFree {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which is `System`'s too.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as for `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` is a live block of `layout.size()` bytes that the caller owns and will
        // not use again; it came from `System` through this allocator.
        unsafe {
            wipe(ptr, layout.size());
            System.dealloc(ptr, layout);
        }
    }

    // `realloc` is deliberately not overridden: the default allocates a new block, copies, and
    // frees the old one through `dealloc` above, so the old copy is wiped. `System.realloc` may
    // move the data and free the old block itself, unwiped.
}

/// Zero `len` bytes at `ptr` with volatile writes, so the compiler cannot drop them as dead
/// stores ahead of the `free` that follows. Whole words where possible: Argon2id's working
/// memory (up to 512 MiB) comes through here.
///
/// # Safety
/// `ptr` must be valid for writes of `len` bytes.
unsafe fn wipe(ptr: *mut u8, len: usize) {
    const WORD: usize = size_of::<usize>();
    let mut at = 0;
    // SAFETY: every write is inside `ptr..ptr + len`, and the word writes are aligned.
    unsafe {
        while at < len && !(ptr.add(at) as usize).is_multiple_of(WORD) {
            ptr.add(at).write_volatile(0);
            at += 1;
        }
        while len - at >= WORD {
            ptr.add(at).cast::<usize>().write_volatile(0);
            at += WORD;
        }
        while at < len {
            ptr.add(at).write_volatile(0);
            at += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::wipe;

    #[test]
    fn wipes_exactly_the_given_range_at_every_alignment() {
        for start in 0..17 {
            for len in 0..70 {
                let mut buf = [0xa5u8; 96];
                // SAFETY: `start + len` is within the 96-byte buffer.
                unsafe { wipe(buf.as_mut_ptr().add(start), len) };
                for (i, byte) in buf.iter().enumerate() {
                    let expected = if (start..start + len).contains(&i) {
                        0
                    } else {
                        0xa5
                    };
                    assert_eq!(*byte, expected, "start {start}, len {len}, byte {i}");
                }
            }
        }
    }
}
