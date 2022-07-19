use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::{env, io};

use anyhow::Result;

/// Build a [`PathBuf`].
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use embuild::path_buf;
/// assert_eq!(path_buf!["/foo", "bar"].as_path(), Path::new("/foo/bar"));
/// ```
#[macro_export]
macro_rules! path_buf {
    ($($e: expr),*) => {{
        use std::path::PathBuf;
        let mut pb = PathBuf::new();
        $(
            pb.push($e);
        )*
        pb
    }}
}

pub trait PathExt: AsRef<Path> {
    /// Pop `times` segments from this path.
    fn pop_times(&self, times: usize) -> PathBuf {
        let mut path: PathBuf = self.as_ref().to_owned();
        for _ in 0..times {
            path.pop();
        }
        path
    }

    /// Make this path absolute relative to `relative_dir` if not already.
    ///
    /// Note: Does not check if the path exists and no normalization takes place.
    fn abspath_relative_to(&self, relative_dir: impl AsRef<Path>) -> PathBuf {
        if self.as_ref().is_absolute() {
            return self.as_ref().to_owned();
        }

        relative_dir.as_ref().join(self)
    }

    /// Make this path absolute relative to [`env::current_dir`] if not already.
    ///
    /// Note: Does not check if the path exists and no normalization takes place.
    fn abspath(&self) -> io::Result<PathBuf> {
        if self.as_ref().is_absolute() {
            return Ok(self.as_ref().to_owned());
        }

        Ok(env::current_dir()?.join(self))
    }
}

impl PathExt for Path {}
impl PathExt for PathBuf {}

/// Error when converting from [`OsStr`] to [`String`] fails.
///
/// The contained [`String`] is is the lossy conversion of the original.
#[derive(Debug, thiserror::Error)]
#[error("failed to convert OsStr '{0}' to String, invalid utf-8")]
pub struct Utf8ConvError(pub String);

pub trait OsStrExt: AsRef<OsStr> {
    /// Try to convert this [`OsStr`] into a string.
    fn try_to_str(&self) -> Result<&str, Utf8ConvError> {
        match self.as_ref().to_str() {
            Some(s) => Ok(s),
            _ => Err(Utf8ConvError(self.as_ref().to_string_lossy().to_string())),
        }
    }
}

impl OsStrExt for OsStr {}
impl OsStrExt for std::ffi::OsString {}
impl OsStrExt for Path {}
impl OsStrExt for PathBuf {}

/// Download the file at `url` to `writer`.
/// 
/// Fails if the response status is not `200` (`OK`).
#[cfg(feature = "ureq")]
pub fn download_file_to(url: &str, writer: &mut impl std::io::Write) -> Result<()> {
    let req = ureq::get(url).call()?;
    if req.status() != 200 {
        anyhow::bail!(
            "Server at url '{}' returned unexpected status {}: {}",
            url,
            req.status(),
            req.status_text()
        );
    }

    let mut reader = req.into_reader();
    std::io::copy(&mut reader, writer)?;
    Ok(())
}
