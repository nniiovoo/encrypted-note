//! Local Quick Unlock: a biometric-gated Secure Enclave key wraps the Vault Key.
//! No Keychain item, account, provisioning profile, or network is involved.
//! Native work blocks: callers must release the core mutex and serialize enroll/remove.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use zeroize::Zeroizing;

pub const MAX_BLOB_LEN: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Availability {
    Available,
    UnsupportedPlatform,
    BiometryUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Unavailable(Availability),
    Cancelled,
    Missing,
    AlreadyExists,
    InvalidPayload,
    Failed,
    CleanupFailed,
}

/// A fresh id per operation, shared across wrapping and protected enrollment read-back.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<CancellationState>);

struct CancellationState {
    cancelled: AtomicBool,
    id: u64,
}

impl Default for CancellationState {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |id| id.checked_add(1))
            .unwrap_or(0);
        Self {
            cancelled: AtomicBool::new(id == 0),
            id,
        }
    }
}

impl Cancellation {
    pub fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::SeqCst) {
            #[cfg(target_os = "macos")]
            mac::cancel(self.0.id);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.id == 0 || self.0.cancelled.load(Ordering::SeqCst)
    }

    #[cfg(any(target_os = "macos", test))]
    fn check(&self) -> Result<(), Error> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

impl Drop for CancellationState {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if self.id != 0 {
            mac::release(self.id);
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::{availability, enroll, read, remove};

#[cfg(not(target_os = "macos"))]
pub fn availability() -> Availability {
    Availability::UnsupportedPlatform
}

#[cfg(not(target_os = "macos"))]
pub fn enroll(
    _path: &Path,
    _payload: Zeroizing<Vec<u8>>,
    _cancel: &Cancellation,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    Err(Error::Unavailable(Availability::UnsupportedPlatform))
}

#[cfg(not(target_os = "macos"))]
pub fn read(_path: &Path, _cancel: &Cancellation) -> Result<Zeroizing<Vec<u8>>, Error> {
    Err(Error::Unavailable(Availability::UnsupportedPlatform))
}

#[cfg(not(target_os = "macos"))]
pub fn remove(_path: &Path) -> Result<(), Error> {
    Err(Error::Unavailable(Availability::UnsupportedPlatform))
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::{
        ffi::{CString, OsStr},
        fs::{File, Metadata, OpenOptions},
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{
                ffi::OsStrExt,
                fs::{MetadataExt, OpenOptionsExt},
            },
        },
        path::Component,
    };

    const PAYLOAD_LEN: usize = vault_format::QUICK_UNLOCK_PAYLOAD_LEN;

    // These functions are compiled from native/biometry.swift. Lengths are Swift Int /
    // Rust isize. Every output is caller-owned and zeroed by Rust when it leaves scope.
    unsafe extern "C" {
        fn enote_biometry_available() -> i32;
        fn enote_biometry_cancel(id: u64);
        fn enote_biometry_release(id: u64);
        fn enote_biometry_wrap(
            id: u64,
            payload: *const u8,
            length: isize,
            output: *mut u8,
            capacity: isize,
            written: *mut isize,
        ) -> i32;
        fn enote_biometry_open(
            id: u64,
            blob: *const u8,
            length: isize,
            output: *mut u8,
            capacity: isize,
        ) -> i32;
        fn enote_biometry_validate(blob: *const u8, length: isize) -> i32;
    }

    pub(super) fn cancel(id: u64) {
        // SAFETY: an opaque request id; the synchronized Swift registry owns its contexts.
        unsafe { enote_biometry_cancel(id) };
    }

    pub(super) fn release(id: u64) {
        // SAFETY: the final Arc drop means no operation can still borrow this request.
        unsafe { enote_biometry_release(id) };
    }

    pub fn availability() -> Availability {
        // SAFETY: availability checks hardware/policy without generating keys or prompting.
        if unsafe { enote_biometry_available() } == 1 {
            Availability::Available
        } else {
            Availability::BiometryUnavailable
        }
    }

    fn native_status(result: i32) -> Result<(), Error> {
        match result {
            0 => Ok(()),
            1 => Err(Error::Cancelled),
            2 => Err(Error::Unavailable(Availability::BiometryUnavailable)),
            3 => Err(Error::InvalidPayload),
            _ => Err(Error::Failed),
        }
    }

    fn io_error(error: std::io::Error) -> Error {
        match error.kind() {
            std::io::ErrorKind::NotFound => Error::Missing,
            std::io::ErrorKind::AlreadyExists => Error::AlreadyExists,
            _ => Error::Failed,
        }
    }

    fn c_string(value: &OsStr) -> Result<CString, Error> {
        CString::new(value.as_bytes()).map_err(|_| Error::Failed)
    }

    struct Location {
        directory: File,
        name: CString,
    }

    impl Location {
        fn new(path: &Path) -> Result<Self, Error> {
            if !path.is_absolute() {
                return Err(Error::Failed);
            }
            let parent = path.parent().ok_or(Error::Failed)?;
            let name = c_string(path.file_name().ok_or(Error::Failed)?)?;
            if parent
                .components()
                .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
            {
                return Err(Error::Failed);
            }
            // Reject every symlink without reading ancestors outside the app sandbox.
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW_ANY | libc::O_CLOEXEC)
                .open(parent)
                .map_err(io_error)?;
            let metadata = directory.metadata().map_err(io_error)?;
            // SAFETY: geteuid reads the current process's effective user id.
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(Error::Failed);
            }
            Ok(Self { directory, name })
        }

        fn open(&self) -> Result<File, Error> {
            // SAFETY: owned directory fd and exact single filename. NONBLOCK prevents a
            // substituted FIFO from hanging before the regular-file metadata check.
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    self.name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
            // SAFETY: a fresh successful openat transfers sole fd ownership.
            let file = unsafe { File::from_raw_fd(fd) };
            validate_file(&file.metadata().map_err(io_error)?)?;
            Ok(file)
        }

        fn unlink(&self, name: &CString) -> Result<(), Error> {
            // SAFETY: an owned directory and exact filename; flags zero never follows links.
            if unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) } == 0 {
                Ok(())
            } else {
                Err(io_error(std::io::Error::last_os_error()))
            }
        }
    }

    fn validate_file(metadata: &Metadata) -> Result<(), Error> {
        // SAFETY: geteuid only reads the current process identity.
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o7777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(Error::InvalidPayload);
        }
        Ok(())
    }

    fn read_blob(path: &Path) -> Result<Vec<u8>, Error> {
        let location = Location::new(path)?;
        let file = location.open()?;
        let size = file.metadata().map_err(io_error)?.len();
        if size == 0 || size > MAX_BLOB_LEN as u64 {
            return Err(Error::InvalidPayload);
        }
        let mut blob = Vec::with_capacity(size as usize);
        file.take(MAX_BLOB_LEN as u64 + 1)
            .read_to_end(&mut blob)
            .map_err(io_error)?;
        if blob.is_empty() || blob.len() > MAX_BLOB_LEN {
            return Err(Error::InvalidPayload);
        }
        Ok(blob)
    }

    fn write_blob(path: &Path, blob: &[u8], id: u64) -> Result<(), Error> {
        if blob.is_empty() || blob.len() > MAX_BLOB_LEN {
            return Err(Error::InvalidPayload);
        }
        let location = Location::new(path)?;
        // RENAME_EXCL below is the atomic authority; this check avoids needless temporary work.
        match location.open() {
            Ok(_) => return Err(Error::AlreadyExists),
            Err(Error::Missing) => {}
            Err(error) => return Err(error),
        }
        let mut random = [0u8; 16];
        // SAFETY: a writable local byte array; arc4random_buf fills its exact length.
        unsafe { libc::arc4random_buf(random.as_mut_ptr().cast(), random.len()) };
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let temporary = temporary_name(id, &suffix)?;
        // SAFETY: O_EXCL creates a fresh regular file under the held private directory;
        // exact 0600 permissions and nofollow protect against pre-existing names/links.
        let fd = unsafe {
            libc::openat(
                location.directory.as_raw_fd(),
                temporary.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        // SAFETY: a successful openat returns a new sole-owned fd.
        let mut file = unsafe { File::from_raw_fd(fd) };
        let publish = (|| {
            validate_file(&file.metadata().map_err(io_error)?)?;
            file.write_all(blob).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
            // SAFETY: two exact names in one held directory; EXCL atomically refuses any
            // existing destination instead of replacing an unexpected registration/file.
            if unsafe {
                libc::renameatx_np(
                    location.directory.as_raw_fd(),
                    temporary.as_ptr(),
                    location.directory.as_raw_fd(),
                    location.name.as_ptr(),
                    libc::RENAME_EXCL,
                )
            } != 0
            {
                return Err(io_error(std::io::Error::last_os_error()));
            }
            Ok(())
        })();
        if let Err(error) = publish {
            let _ = location.unlink(&temporary);
            return Err(error);
        }
        if location.directory.sync_all().is_err() {
            let _ = location.unlink(&location.name);
            let _ = location.directory.sync_all();
            return Err(Error::Failed);
        }
        Ok(())
    }

    fn temporary_name(id: u64, suffix: &str) -> Result<CString, Error> {
        // VaultDir removes .tmp-* under its startup lock after an interrupted publication.
        CString::new(format!(".tmp-enote-quick-{id}-{suffix}")).map_err(|_| Error::Failed)
    }

    pub fn read(path: &Path, cancellation: &Cancellation) -> Result<Zeroizing<Vec<u8>>, Error> {
        cancellation.check()?;
        let blob = read_blob(path)?;
        // SAFETY: a bounded live byte slice. Validation performs no Secure Enclave operation.
        native_status(unsafe { enote_biometry_validate(blob.as_ptr(), blob.len() as isize) })?;
        cancellation.check()?;
        let mut payload = Zeroizing::new(vec![0; PAYLOAD_LEN]);
        // SAFETY: immutable bounded input and an exact, writable caller-owned output. Swift
        // writes output only after the actual enclave ECDH operation and AEAD verification.
        let outcome = unsafe {
            enote_biometry_open(
                cancellation.0.id,
                blob.as_ptr(),
                blob.len() as isize,
                payload.as_mut_ptr(),
                PAYLOAD_LEN as isize,
            )
        };
        cancellation.check()?;
        native_status(outcome)?;
        Ok(payload)
    }

    pub fn enroll(
        path: &Path,
        payload: Zeroizing<Vec<u8>>,
        cancellation: &Cancellation,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        if payload.len() != PAYLOAD_LEN {
            return Err(Error::InvalidPayload);
        }
        cancellation.check()?;
        let mut blob = Zeroizing::new(vec![0; MAX_BLOB_LEN]);
        let mut written = 0isize;
        // SAFETY: exact 80-byte input, bounded writable output, and a live output-length slot.
        // Swift creates only an encrypted device-bound key blob, never a Keychain item.
        let outcome = unsafe {
            enote_biometry_wrap(
                cancellation.0.id,
                payload.as_ptr(),
                PAYLOAD_LEN as isize,
                blob.as_mut_ptr(),
                MAX_BLOB_LEN as isize,
                &mut written,
            )
        };
        cancellation.check()?;
        native_status(outcome)?;
        let written = usize::try_from(written).map_err(|_| Error::InvalidPayload)?;
        if written == 0 || written > MAX_BLOB_LEN {
            return Err(Error::InvalidPayload);
        }
        cancellation.check()?;
        write_blob(path, &blob[..written], cancellation.0.id)?;
        // Enrollment commits only after a fresh biometric-gated private ECDH read-back.
        let verified = read(path, cancellation).and_then(|returned| {
            let mismatch = returned
                .iter()
                .zip(payload.iter())
                .fold(0u8, |different, (a, b)| different | (a ^ b));
            cancellation.check()?;
            if mismatch == 0 {
                Ok(returned)
            } else {
                Err(Error::InvalidPayload)
            }
        });
        match verified {
            Ok(returned) => Ok(returned),
            Err(error) => {
                if remove(path).is_err() {
                    return Err(Error::CleanupFailed);
                }
                Err(error)
            }
        }
    }

    pub fn remove(path: &Path) -> Result<(), Error> {
        let location = Location::new(path)?;
        match location.open() {
            Ok(file) => {
                drop(file);
                location.unlink(&location.name)?;
                location.directory.sync_all().map_err(io_error)
            }
            // Flush an earlier unlink/rename whose durability may have been uncertain.
            Err(Error::Missing) => location.directory.sync_all().map_err(io_error),
            Err(error) => Err(error),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{PermissionsExt, symlink};

        fn private_location() -> (tempfile::TempDir, std::path::PathBuf) {
            let directory = tempfile::Builder::new()
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir()
                .expect("fake private directory");
            let canonical = directory
                .path()
                .canonicalize()
                .expect("physical temporary path");
            let path = canonical.join("fake-quick-unlock.bin");
            (directory, path)
        }

        #[test]
        fn private_location_works_when_ancestor_listing_is_denied() {
            const CHILD_PATH: &str = "ENOTE_TEST_SANDBOX_LOCATION";
            const CHILD_OK: &str = "ENOTE_TEST_SANDBOX_LOCATION_OK";
            if let Some(path) = std::env::var_os(CHILD_PATH) {
                let path = std::path::PathBuf::from(path);
                let ancestor = path.parent().unwrap().parent().unwrap();
                assert_eq!(
                    std::fs::File::open(ancestor)
                        .err()
                        .expect("fixture ancestor listing must be denied")
                        .kind(),
                    std::io::ErrorKind::PermissionDenied
                );
                write_blob(&path, b"obviously-fake-encrypted-blob", 99)
                    .expect("publish in the allowed private child");
                assert_eq!(
                    read_blob(&path).expect("read in the allowed private child"),
                    b"obviously-fake-encrypted-blob"
                );
                remove(&path).expect("remove in the allowed private child");
                remove(&path).expect("missing removal still flushes the private child");
                println!("{CHILD_OK}");
                return;
            }

            let (_fixture, original_path) = private_location();
            let ancestor = original_path.parent().unwrap();
            let parent = ancestor.join("fake-private-child");
            std::fs::create_dir(&parent).expect("fake private child");
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
                .expect("private child permissions");
            let path = parent.join("fake-quick-unlock.bin");
            let mut parameter = std::ffi::OsString::from("ENOTE_TEST_DENIED_ANCESTOR=");
            parameter.push(ancestor);
            let output = std::process::Command::new("/usr/bin/sandbox-exec")
                .arg("-D")
                .arg(parameter)
                .args([
                    "-p",
                    r#"(version 1) (allow default) (deny file-read-data (literal (param "ENOTE_TEST_DENIED_ANCESTOR")))"#,
                ])
                .arg(std::env::current_exe().expect("test executable"))
                .args([
                    "--exact",
                    "biometry::mac::tests::private_location_works_when_ancestor_listing_is_denied",
                    "--nocapture",
                ])
                .env(CHILD_PATH, &path)
                .output()
                .expect("run the sandboxed private-location fixture");
            assert!(
                output.status.success(),
                "sandboxed private-location helper failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains(CHILD_OK));
        }

        #[test]
        fn parent_symlinks_and_traversal_are_refused_without_changing_fake_blobs() {
            let (_fixture, root_path) = private_location();
            let root = root_path.parent().expect("fixture root");
            let physical_parent = root.join("fake-private-parent");
            let physical_child = physical_parent.join("fake-private-child");
            for directory in [&physical_parent, &physical_child] {
                std::fs::create_dir(directory).expect("fake private directory");
                std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                    .expect("private directory permissions");
            }
            let path = physical_child.join("fake-quick-unlock.bin");
            write_blob(&path, b"obviously-fake-original-blob", 21).expect("original fake blob");
            let final_parent_link = root.join("fake-final-parent-link");
            let intermediate_link = root.join("fake-intermediate-link");
            symlink(&physical_child, &final_parent_link).expect("fake final parent link");
            symlink(&physical_parent, &intermediate_link).expect("fake intermediate link");

            for candidate in [
                final_parent_link.join("fake-quick-unlock.bin"),
                intermediate_link.join("fake-private-child/fake-quick-unlock.bin"),
                physical_child.join("../fake-private-child/fake-quick-unlock.bin"),
            ] {
                assert_eq!(read_blob(&candidate), Err(Error::Failed));
                assert_eq!(remove(&candidate), Err(Error::Failed));
                assert_eq!(
                    write_blob(
                        &candidate.with_file_name("fake-new-blob"),
                        b"fake-new-blob",
                        22
                    ),
                    Err(Error::Failed)
                );
                assert_eq!(
                    read_blob(&path).expect("original fake blob survives each refused operation"),
                    b"obviously-fake-original-blob"
                );
                assert!(!physical_child.join("fake-new-blob").exists());
            }
        }

        #[test]
        fn publication_is_private_durable_and_never_overwrites() {
            let (_directory, path) = private_location();
            write_blob(&path, b"obviously-fake-encrypted-blob", 1).expect("create fake blob");
            assert_eq!(
                read_blob(&path).expect("read fake blob"),
                b"obviously-fake-encrypted-blob"
            );
            assert_eq!(
                write_blob(&path, b"replacement", 2),
                Err(Error::AlreadyExists)
            );
            assert_eq!(
                std::fs::metadata(&path).expect("mode").mode() & 0o777,
                0o600
            );
            remove(&path).expect("remove exact fake blob");
            remove(&path).expect("missing removal is idempotent");
        }

        #[test]
        fn nonprivate_parent_is_refused_without_changing_the_registration() {
            let (_directory, path) = private_location();
            write_blob(&path, b"obviously-fake-private-registration", 9)
                .expect("private registration");
            let parent = path.parent().expect("parent");
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755))
                .expect("simulate nonprivate parent");
            assert_eq!(read_blob(&path), Err(Error::Failed));
            assert_eq!(remove(&path), Err(Error::Failed));
            assert_eq!(
                write_blob(&path.with_file_name("fake-new-blob"), b"fake", 10),
                Err(Error::Failed)
            );
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .expect("restore private parent");
            assert_eq!(
                read_blob(&path).expect("untouched registration"),
                b"obviously-fake-private-registration"
            );
        }

        #[test]
        fn startup_cleanup_removes_interrupted_wrapped_blob_but_keeps_published_registration() {
            let (_directory, path) = private_location();
            write_blob(&path, b"obviously-fake-published-registration", 7)
                .expect("published fake blob");
            let temporary =
                temporary_name(8, "obviously-fake-encrypted-blob").expect("temporary name");
            let orphan = path
                .parent()
                .expect("private directory")
                .join(OsStr::from_bytes(temporary.as_bytes()));
            std::fs::write(&orphan, b"obviously-fake-interrupted-wrapped-blob")
                .expect("crash fixture");
            std::fs::set_permissions(&orphan, std::fs::Permissions::from_mode(0o600))
                .expect("private fixture");
            let _locked = store::VaultDir::open(path.parent().expect("directory"))
                .expect("locked startup cleanup");
            assert!(!orphan.exists());
            assert_eq!(
                read_blob(&path).expect("published registration survives"),
                b"obviously-fake-published-registration"
            );
        }

        #[test]
        fn refuses_symlinks_hardlinks_special_files_and_nonprivate_modes() {
            let (_directory, path) = private_location();
            let target = path.with_file_name("unrelated-fake-file");
            std::fs::write(&target, b"leave this fake file alone").expect("target");
            symlink(&target, &path).expect("fake symlink");
            assert!(read_blob(&path).is_err());
            assert!(remove(&path).is_err());
            assert!(write_blob(&path, b"fake", 3).is_err());
            std::fs::remove_file(&path).expect("remove symlink");
            write_blob(&path, b"fake", 4).expect("private fake file");
            std::fs::hard_link(&path, path.with_file_name("fake-hardlink")).expect("hardlink");
            assert!(read_blob(&path).is_err());
            std::fs::remove_file(path.with_file_name("fake-hardlink")).expect("remove hardlink");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
            assert!(read_blob(&path).is_err());
            assert!(remove(&path).is_err());
            assert_eq!(
                std::fs::read(&target).expect("untouched target"),
                b"leave this fake file alone"
            );
            std::fs::remove_file(&path).expect("remove nonprivate fake file");
            let fifo = c_string(path.as_os_str()).expect("fake FIFO path");
            // SAFETY: an exact temporary filename under the fake private directory.
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            assert_eq!(read_blob(&path), Err(Error::InvalidPayload));
            assert!(remove(&path).is_err());
        }

        #[test]
        fn bounded_reader_and_native_schema_reject_malformed_data_without_authentication() {
            let (_directory, path) = private_location();
            write_blob(&path, &[0; 32], 5).expect("fake malformed blob");
            let cancel = Cancellation::default();
            assert_eq!(read(&path, &cancel), Err(Error::InvalidPayload));
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("fake file");
            file.set_len(MAX_BLOB_LEN as u64 + 1)
                .expect("oversized fake file");
            assert_eq!(read_blob(&path), Err(Error::InvalidPayload));
            assert_eq!(
                write_blob(&path, &[0; MAX_BLOB_LEN + 1], 6),
                Err(Error::InvalidPayload)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_shared_and_irreversible() {
        let first = Cancellation::default();
        let second = first.clone();
        assert!(!first.is_cancelled());
        second.cancel();
        assert!(first.is_cancelled());
        assert!(second.check().is_err());
    }
}
