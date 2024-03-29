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
    io::{
        AsyncReadExt,
        AsyncWriteExt,
    },
};
use crate::error::prelude::*;
use snafu::Snafu;

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

pub fn uri_to_url(uri: &Uri) -> Result<Url,url::ParseError> {
    Ok(Url::from_str(format!("{}", uri).as_str())?)
}

pub fn url_to_uri(url: &Url) -> Result<Uri, http::uri::InvalidUri> {
    Ok(Uri::from_str(url.as_ref())?)
}

pub async fn save_stream_to_file<S>(
    mut stream: S,
    path: PathBuf,
) -> download::Result<()>
where
    S: Stream<Item = Result<hyper::Chunk,hyper::error::Error>> + Unpin + Send,
{
    let mut file = tokio::fs::File::create(path).await.context(crate::download::error::Io)?;

    while let Some(chunk) = stream.try_next().await.context(crate::download::error::Hyper)?{
        file.write_all(chunk.as_ref()).await.context(crate::download::error::Io)?;
    }
    Ok(())
}

pub fn file_timestamp<P: AsRef<Path>>(path: P) -> std::io::Result<DateTime<Utc>> {
    let metadata = path.as_ref().metadata()?;
    Ok(metadata.modified()?.into())
}

#[derive(Debug, Snafu)]
pub enum SymlinkError {
    #[snafu(display("IO error while symlinking: {}", source))]
    Io{
        source: std::io::Error,
    },
    #[snafu(display("The target of the symlink already exists"))]
    AlreadyExists,
}

use std::sync::atomic::{AtomicBool, Ordering};

static SYMLINKS_BLOCKED: AtomicBool = AtomicBool::new(false);

use std::fmt::Debug;
pub async fn symlink<P: AsRef<Path> + Debug + Unpin + Send + Clone + 'static, Q: AsRef<Path> + Debug + Unpin + Send + Clone + 'static>(
    src: P,
    dst: Q,
    log: &Logger,
) -> Result<(), SymlinkError> {
    info!(log, "symlinking {:?} to {:?}", src, dst);
    if SYMLINKS_BLOCKED.load(Ordering::Acquire) {
        warn!(log, "symlink permission denied, falling back to copy");
        fs_copy(src,dst).await.context(Io)?;
        Ok(())
    } else {
        match symlink_internal(src.clone(), dst.clone()).await {
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
                fs_copy(src,dst).await.context(Io)?;
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(e) => Err(Io.into_error(e)),
        }
    }
}

pub async fn fs_copy<P: AsRef<Path> + Unpin + Send + 'static, Q: AsRef<Path>+ Unpin + Send + 'static>(src: P, dst: Q) -> io::Result<()> {
    let src_open = tokio::fs::File::open(src);
    let dst_open = tokio::fs::File::create(dst);
    let (mut src,mut dst) = futures::try_join!(
        src_open,
        dst_open
    )?;
    src.copy(&mut dst).await?;
    Ok(())
}

#[cfg(windows)]
async fn symlink_internal<P: AsRef<Path> + Unpin + Send + Clone + 'static, Q: AsRef<Path> + Unpin + Send + 'static>(src: P, dst: Q) -> io::Result<()> {
    let metadata = tokio::fs::symlink_metadata(src.clone()).await?;
    if metadata.is_file() {
        tokio::fs::os::windows::symlink_file(src, dst).await
    } else if metadata.is_dir() {
        tokio::fs::os::windows::symlink_dir(src, dst).await
    } else {
        panic!("tried to symlink unknown filetype")
    }
}

#[cfg(unix)]
async fn symlink_internal<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> io::Result<()> {
    tokio::fs::os::unix::symlink(src, dst).await
}
