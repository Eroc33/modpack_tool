use download;
use futures::future;
use futures::prelude::*;

use hyper;
use hyper::Uri;
use slog::Logger;
use std::fs::File;
use std::io::{self, Read, Cursor};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::io::copy;

use url::Url;

pub fn uri_to_url(uri: &Uri) -> ::download::Result<Url> {
    Ok(Url::from_str(uri.as_ref())?)
}

pub fn url_to_uri(url: &Url) -> ::download::Result<Uri> {
    Ok(Uri::from_str(url.as_ref())?)
}

pub fn save_stream_to_file<S>(stream: S,
                              path: PathBuf)
                              -> impl Future<Item = (), Error = download::Error>
    where S: Stream<Item = hyper::Chunk>,
          S::Error: Into<download::Error>
{
    future::result(File::create(path))
        .map_err(download::Error::from)
        .and_then(move |file| {
            stream.map_err(Into::into)
                .fold(file, |mut file, chunk| -> Result<File, download::Error> {
                    io::copy(&mut Cursor::new(chunk), &mut file)?;
                    Ok(file)
                })
                .map(|_| ())
        })
}

pub fn save_file<R>(mut reader: R, path: &Path) -> impl Future<Item = u64, Error = io::Error>
    where R: Read
{
    future::result(File::create(path)).and_then(move |mut file| copy(&mut reader, &mut file))
}

pub fn file_timestamp<P: AsRef<Path>>(path: P) -> download::Result<::std::time::SystemTime> {
    let metadata = path.as_ref().metadata()?;
    Ok(metadata.modified()?)
}

use std::sync::atomic::{ATOMIC_BOOL_INIT, AtomicBool, Ordering};

static SYMLINKS_BLOCKED: AtomicBool = ATOMIC_BOOL_INIT;

use std::fmt::Debug;
pub fn symlink<P: AsRef<Path> + Debug, Q: AsRef<Path> + Debug>(src: P,
                                                               dst: Q,
                                                               log: &Logger)
                                                               -> io::Result<()> {
    info!(log, "symlinking {:?} to {:?}", src, dst);
    if SYMLINKS_BLOCKED.load(Ordering::Acquire) {
        warn!(log, "symlink permission denied, falling back to copy");
        ::std::fs::copy(src, dst)?;
        Ok(())
    } else {
        match _symlink(src.as_ref(), dst.as_ref()) {
            // if the symlink failed due to permission denied
            Err(ref e) if e.raw_os_error() == Some(1314) => {
                warn!(log, "Symlink permission denied, falling back to copy");
                SYMLINKS_BLOCKED.store(true, Ordering::Release);
                ::std::fs::copy(src, dst)?;
                Ok(())
            }
            other => other,
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
