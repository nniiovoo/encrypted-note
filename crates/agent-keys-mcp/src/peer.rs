//! Who runs the other end of the socket, by the kernel's word. The only `unsafe` in this crate.

use std::os::unix::net::UnixStream;

/// The effective uid of the process at the other end of `stream`, or `None` if the kernel
/// can't say (or, off the Mac, isn't asked).
#[cfg(target_os = "macos")]
pub fn server_uid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut uid: libc::uid_t = u32::MAX;
    let mut gid: libc::gid_t = u32::MAX;
    // SAFETY: the descriptor is open for as long as `stream` is borrowed, and both out pointers
    // are valid for writes.
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    (rc == 0 && uid != u32::MAX).then_some(uid)
}

#[cfg(not(target_os = "macos"))]
pub fn server_uid(_stream: &UnixStream) -> Option<u32> {
    None
}

/// This process's own effective uid (for tests and the dev setup).
#[cfg(unix)]
pub fn own_uid() -> u32 {
    // SAFETY: geteuid has no arguments and cannot fail.
    unsafe { libc::geteuid() }
}
