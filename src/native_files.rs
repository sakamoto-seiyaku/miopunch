//! Platform-specific publication of already-flushed native private files.
//! Windows has no Unix directory-fsync equivalent here: use write-through
//! MoveFileExW for publication, and FlushFileBuffers (File::sync_all) for data.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};

fn create_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn write_private_contents(mut file: File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_private_contents(create_private_file(path)?, bytes)
}

/// Same-directory replacement of a fully flushed private file. Only a file
/// successfully created by this operation is eligible for temporary cleanup.
pub(crate) fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".mio-{:032x}", rand::random::<u128>()));
    let file = create_private_file(&temporary)?;
    let result = (|| {
        // Drop the file before MoveFileExW on Windows.
        write_private_contents(file, bytes)?;
        replace_file(&temporary, path)?;
        sync_directory(parent)
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

#[cfg(not(windows))]
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(windows)]
pub(crate) fn sync_directory(_path: &Path) -> io::Result<()> {
    // Publication uses MOVEFILE_WRITE_THROUGH below. Opening a directory as a
    // regular File fails with ERROR_ACCESS_DENIED on Windows. We do not claim
    // physical-power-loss guarantees for a filesystem not runtime-qualified.
    Ok(())
}

pub(crate) fn publish_file(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    return move_windows(source, target, false);
    // Android SELinux can deny hard links even in a writable shell directory.
    // renameat2 preserves atomic, no-overwrite publication without link permission.
    #[cfg(target_os = "android")]
    return rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        target,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into);
    #[cfg(not(any(windows, target_os = "android")))]
    std::fs::hard_link(source, target)
}

pub(crate) fn replace_file(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    return move_windows(source, target, true);
    #[cfg(not(windows))]
    std::fs::rename(source, target)
}

#[cfg(windows)]
pub(crate) fn move_windows(source: &Path, target: &Path, replace: bool) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        value.push(0);
        Ok(value)
    }
    let source = wide(source)?;
    let target = wide(target)?;
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // Both pointers own NUL-terminated UTF-16 until the synchronous call ends.
    // No COPY_ALLOWED: publication must remain a same-volume rename.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
