//! Process hardening applied before any secret exists in memory.

/// Disable core dumps so a crash can't write process memory (keys, Notes) to disk.
#[cfg(unix)]
pub fn apply() {
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: setrlimit with a valid, fully-initialised struct; failure is harmless (we keep going
    // with the OS default, which on macOS is already 0).
    #[allow(unsafe_code)]
    unsafe {
        libc::setrlimit(libc::RLIMIT_CORE, &limit);
    }
}

#[cfg(not(unix))]
pub fn apply() {}
