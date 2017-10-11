use download::{self, DownloadManager};
use futures::future;
#[macro_use]
use futures::prelude::*;
use hyper::{self, Uri};
use slog::Logger;
use std::fs;
use std::path::{PathBuf, Path};
use std::result::Result;
use util;

pub trait Cacheable {
    fn cached_path(&self) -> PathBuf;
    fn uri(&self) -> Result<Uri, hyper::error::UriError>;
}

pub trait Cache<T: Cacheable + 'static> {
    fn is_cached(t: &T) -> bool {
        t.cached_path().exists()
    }

    fn with(t: T, manager: DownloadManager, log: Logger) -> download::BoxFuture<PathBuf>;

    #[async(boxed)]
    fn install_at(t: T,
                  mut location: PathBuf,
                  manager: DownloadManager,
                  log: Logger)
                  -> ::download::Result<()> {
        let cached_path = await!(Self::with(t, manager, log.clone()))?;
        info!(log, "installing item"; "location"=>location.as_path().to_string_lossy().into_owned());

        fs::create_dir_all(&location)?;
        
        cached_path.file_name().map(|n| location.push(n));
        util::symlink(cached_path, location, &log)?;
        Ok(())
    }
}

fn first_file_in_folder<P: AsRef<Path>>(path: P) -> Result<PathBuf,download::Error> {
    Ok(path.as_ref()
        .read_dir()
        .map_err(|_| download::Error::from(download::ErrorKind::CacheError))?
        .next()
        .ok_or_else(|| download::Error::from(download::ErrorKind::CacheError))?
        .unwrap()
        .path())
}

pub struct FolderCache;

impl<T: Cacheable + 'static> ::cache::Cache<T> for FolderCache {
    fn with(t: T, manager: DownloadManager, log: Logger) -> download::BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));

        if Self::is_cached(&t) {
            info!(log, "item was already cached");
            Box::new(future::result(first_file_in_folder(&cached_path)).or_else(move |_|{
                //invalidate cache
                warn!(log,"Removing invalid cache folder {:?}",cached_path);
                future::result(fs::remove_dir(cached_path).map_err(|ioe| download::Error::from(ioe))).and_then(|_|{
                    //FIXME: will retry forever
                    //retry
                    Self::with(t,manager,log)
                })
            }))
        } else {
            info!(log, "item is not cached, downloading now");
            match t.uri() {
                Ok(uri) => {
                    Box::new(manager.download(uri, cached_path.clone(), true, log)
                        .and_then(move |_| first_file_in_folder(cached_path)))
                }
                Err(e) => Box::new(future::err(download::Error::from(e))),
            }
        }
    }
}

pub struct FileCache;

impl<T: Cacheable + 'static> ::cache::Cache<T> for FileCache {
    fn with(t: T, manager: DownloadManager, log: Logger) -> download::BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));

        if Self::is_cached(&t) {
            info!(log, "item was already cached");
            Box::new(future::ok(cached_path))
        } else {
            info!(log, "item is not cached, downloading now");
            match t.uri() {
                Ok(uri) => {
                    Box::new(manager.download(uri, cached_path.clone(), false, log)
                        .map(move |_| cached_path))
                }
                Err(e) => Box::new(future::err(download::Error::from(e))),
            }
        }
    }
}
