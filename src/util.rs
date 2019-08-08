use crate::download;
use futures::prelude::*;

use hyper;
use http::Uri;
use slog::Logger;
use std;
use std::io::{self};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use chrono::DateTime;
use chrono::offset::Utc;
use tokio::{
    self,
    prelude::AsyncRead,
    io::{
        AsyncReadExt,
        AsyncWriteExt,
    },
};

use url::Url;

//TODO: make this an extension method?
pub fn remove_unc_prefix<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref().to_str().unwrap();
    let path = path.trim_start_matches(r#"\\?\"#); //actually remove UNC prefix
    path.into()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use super::remove_unc_prefix;
    #[test]
    fn test_remove_unc_prefix() {
        assert_eq!(
            remove_unc_prefix(r#"C:\foo\bar\baz"#),
            PathBuf::from(r#"C:\foo\bar\baz"#)
        );
        assert_eq!(
            remove_unc_prefix(r#"\\?\C:\foo\bar\baz"#),
            PathBuf::from(r#"C:\foo\bar\baz"#)
        );
    }
}

pub fn uri_to_url(uri: &Uri) -> crate::download::Result<Url> {
    Ok(Url::from_str(format!("{}", uri).as_str())?)
}

pub fn url_to_uri(url: &Url) -> crate::download::Result<Uri> {
    Ok(Uri::from_str(url.as_ref())?)
}

pub async fn save_stream_to_file<S,E>(
    mut stream: S,
    path: PathBuf,
) -> download::Result<()>
where
    S: Stream<Item = Result<hyper::Chunk,E>> + Unpin + Send,
    download::Error: From<E>,
{
    let mut file = tokio::fs::File::create(path).await?;

    while let Some(chunk) = stream.try_next().await?{
        file.write_all(chunk.as_ref()).await?;
    }
    Ok(())
}

pub async fn save_file<R>(
    mut reader: R,
    path: PathBuf,
) -> download::Result<u64>
where
    R: AsyncRead + Send + Unpin,
{
    let mut file = tokio::fs::File::create(path).await?;

    Ok(reader.copy(&mut file).await?)
}

pub fn file_timestamp<P: AsRef<Path>>(path: P) -> download::Result<DateTime<Utc>> {
    let metadata = path.as_ref().metadata()?;
    Ok(metadata.modified()?.into())
}

#[derive(Debug, Fail)]
pub enum SymlinkError {
    #[fail(display = "{}", _0)]
    Io(#[cause] std::io::Error),
    #[fail(display = "The target of the symlink already exists")]
    AlreadyExists,
}

impl From<std::io::Error> for SymlinkError {
    fn from(err: std::io::Error) -> Self {
        SymlinkError::Io(err)
    }
}

use std::sync::atomic::{AtomicBool, Ordering};

static SYMLINKS_BLOCKED: AtomicBool = AtomicBool::new(false);

use std::fmt::Debug;
pub fn symlink<P: AsRef<Path> + Debug, Q: AsRef<Path> + Debug>(
    src: P,
    dst: Q,
    log: &Logger,
) -> Result<(), SymlinkError> {
    info!(log, "symlinking {:?} to {:?}", src, dst);
    if SYMLINKS_BLOCKED.load(Ordering::Acquire) {
        warn!(log, "symlink permission denied, falling back to copy");
        ::std::fs::copy(src, dst)?;
        Ok(())
    } else {
        match _symlink(src.as_ref(), dst.as_ref()) {
            //if the file already exists
            #[cfg(windows)]
            Err(ref e) if e.raw_os_error() == Some(183) =>
            {
                Err(SymlinkError::AlreadyExists)
            }
            // if the symlink failed due to permission denied
            #[cfg(windows)]
            Err(ref e) if e.raw_os_error() == Some(1314) =>
            {
                warn!(log, "Symlink permission denied, falling back to copy");
                SYMLINKS_BLOCKED.store(true, Ordering::Release);
                ::std::fs::copy(src, dst)?;
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(windows)]
fn _symlink<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> io::Result<()> {
    let metadata = ::std::fs::symlink_metadata(src.as_ref())?;
    if metadata.is_file() {
        ::std::os::windows::fs::symlink_file(src, dst)
    } else if metadata.is_dir() {
        ::std::os::windows::fs::symlink_dir(src, dst)
    } else {
        panic!("tried to symlink unknown filetype")
    }
}

#[cfg(unix)]
fn _symlink<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> io::Result<()> {
    ::std::os::unix::fs::symlink(src, dst)
}
