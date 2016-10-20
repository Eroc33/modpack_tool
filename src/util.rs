#![allow(redundant_closure)]
use download;
use futures;
use hyper;
use serde_json;

use slog::Logger;
use std::fs::File;
use std::io::{self, Read, copy};
use std::path::{Path, PathBuf};
use time;
use url::{self, Url};
use zip;

error_chain! {
  links {
    download::Error, download::ErrorKind, Download;
  }
  foreign_links{
    io::Error, Io;
    url::ParseError, Url;
    hyper::Error, Hyper;
    zip::result::ZipError,Zip;
    serde_json::error::Error, Json;
  }
  errors {
    UnknownScheme(t: String) {
      description("unknown url scheme")
      display("unknown url scheme: '{}'", t)
    }
  }
}

pub type BoxFuture<I> = futures::BoxFuture<I,Error>;

pub fn create_dir(mut path: PathBuf) -> ::std::io::Result<()> {
    path.set_file_name("");
    ::std::fs::DirBuilder::new().recursive(true)
        .create(path)?;
    Ok(())
}

pub fn save_file<R>(mut reader: R, path: &Path) -> io::Result<u64>
    where R: Read
{
    let mut file = File::create(path)?;
    copy(&mut reader, &mut file)
}

#[deprecated]
pub fn download(url: &Url, path: PathBuf, append_filename: bool, log: Logger) -> impl download::Future<()> {
    download_with(url, path, append_filename, &hyper::Client::new(), log)
}

fn epoch_tm() -> time::Tm {
    time::at_utc(time::Timespec { sec: 0, nsec: 0 })
}

pub fn download_with(url: &Url,
                     path: PathBuf,
                     append_filename: bool,
                     client: &hyper::Client,
                     log: Logger)
                     -> impl download::Future<()> {
    futures::done(_download_with(url, path, append_filename, client, log))
}


pub fn _download_with(url: &Url,
                      mut path: PathBuf,
                      append_filename: bool,
                      client: &hyper::Client,
                      log: Logger)
                      -> download::Result<()> {
    let log = log.new(o!("url"=>url.to_string()));
    info!(log, "Downloading");
    create_dir(path.clone())?;

    let mut headers = hyper::header::Headers::new();

    // FIXME find a way to workout which mod file is which *before* downloading
    if path.exists() && path.is_file() {
        let metadata = path.metadata()?;
        let duration = metadata.modified()?.duration_since(::std::time::UNIX_EPOCH)?;
        let duration = time::Duration::from_std(duration)?;
        let timestamp = epoch_tm() + duration;
        headers.set(hyper::header::IfModifiedSince(hyper::header::HttpDate(timestamp)));
    }
    let res = client.get(url.clone()).headers(headers).send()?;
    if res.status == hyper::status::StatusCode::NotModified {
        info!(log, "not modified, skipping {:?}", path);
        return Ok(());
    }

    {
        let filename = match res.url.path_segments() {
            Some(parts) => {
                url::percent_encoding::percent_decode(parts.last().unwrap().as_bytes())
                    .decode_utf8_lossy()
            }
            None => unreachable!("Couldn't retrive filename as url was not relative"),
        };
        if append_filename {
            path.push(filename.into_owned());
        }
    }
    info!(log,"downloaded as"; "path"=>path.as_path().to_string_lossy().into_owned());
    Ok(save_file(res, path.as_path()).map(|_| ())?)
}

use std::sync::atomic::{ATOMIC_BOOL_INIT, AtomicBool, Ordering};

static SYMLINKS_BLOCKED: AtomicBool = ATOMIC_BOOL_INIT;

use std::fmt::Debug;
pub fn symlink<P: AsRef<Path> + Debug, Q: AsRef<Path> + Debug>(src: P,
                                                               dst: Q,
                                                               log: &Logger)
                                                               -> ::std::io::Result<()> {
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
fn _symlink<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> ::std::io::Result<()> {
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
fn _symlink<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> std::io::Result<()> {
    ::std::os::unix::fs::symlink(src, dst)
}
