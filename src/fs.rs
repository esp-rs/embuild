//! Filesystem utilities.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use anyhow::Result;

/// Copy `src_file` to `dest_file_or_dir` if `src_file` is different or the destination
/// file doesn't exist.
///
/// ### Panics
/// If `src_file` is not a file this function will panic.
pub fn copy_file_if_different(
    src_file: impl AsRef<Path>,
    dest_file_or_dir: impl AsRef<Path>,
) -> Result<()> {
    let src_file: &Path = src_file.as_ref();
    let dest_file_or_dir: &Path = dest_file_or_dir.as_ref();

    assert!(src_file.is_file());

    let src_fd = fs::File::open(src_file)?;

    let (dest_fd, dest_file) = if dest_file_or_dir.exists() {
        if dest_file_or_dir.is_dir() {
            let dest_file = dest_file_or_dir.join(src_file.file_name().unwrap());
            if dest_file.exists() {
                (Some(fs::File::open(&dest_file)?), dest_file)
            } else {
                (None, dest_file)
            }
        } else {
            (
                Some(fs::File::open(dest_file_or_dir)?),
                dest_file_or_dir.to_owned(),
            )
        }
    } else {
        (None, dest_file_or_dir.to_owned())
    };

    if let Some(dest_fd) = dest_fd {
        if !is_file_eq(&src_fd, &dest_fd)? {
            drop(dest_fd);
            drop(src_fd);
            copy_with_metadata(src_file, dest_file)?;
        }
    } else {
        copy_with_metadata(src_file, dest_file)?;
    }
    Ok(())
}

/// Whether the file type and contents of `file` are equal to `other`.
pub fn is_file_eq(file: &File, other: &File) -> Result<bool> {
    let file_meta = file.metadata()?;
    let other_meta = other.metadata()?;

    if file_meta.file_type() == other_meta.file_type()
        && file_meta.len() == other_meta.len()
        && file_meta.modified()? == other_meta.modified()?
    {
        let mut file_bytes = io::BufReader::new(&*file).bytes();
        let mut other_bytes = io::BufReader::new(&*other).bytes();

        // TODO: check performance
        loop {
            match (file_bytes.next(), other_bytes.next()) {
                (Some(Ok(b0)), Some(Ok(b1))) => {
                    if b0 != b1 {
                        break Ok(false);
                    }
                }
                (None, None) => break Ok(true),
                (None, Some(_)) | (Some(_), None) => break Ok(false),
                (Some(Err(e)), _) | (_, Some(Err(e))) => return Err(e.into()),
            }
        }
    } else {
        Ok(false)
    }
}

/// Wrap [`fs::copy`] to also copy metadata such as permissions and file times.
///
/// This function is required because Cargo's fingerprinting uses mtime which is not perpetuated by
/// [`fs::copy`]. If Cargo detects a fingerprint mismatch, it will rebuild the crate with the
/// mismatch. This could lead Cargo to keep rebuilding the same crate over and over because the
/// comparison will always yield a new fingerprint since [`fs::copy`] doesn't take mtime into
/// account.
pub fn copy_with_metadata(src_file: impl AsRef<Path>, dest_file: impl AsRef<Path>) -> Result<()> {
    fs::copy(&src_file, &dest_file)?;
    let src_fd = fs::File::open(&src_file)?;
    let dest_fd = fs::File::open(&dest_file)?;
    let src_file_meta = src_fd.metadata()?;

    let src_atime = filetime::FileTime::from_last_access_time(&src_file_meta);
    let src_mtime = filetime::FileTime::from_last_modification_time(&src_file_meta);

    dest_fd.set_permissions(src_file_meta.permissions())?;
    filetime::set_file_times(dest_file, src_atime, src_mtime)?;

    Ok(())
}
