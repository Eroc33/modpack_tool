use download::{self, DownloadManager};
use futures::future;
use futures::prelude::*;
use slog::Logger;
use std::fs;
use std::path::{Path, PathBuf};
use std::result::Result;
use util;
use http::Uri;

pub trait Cacheable: Send + Sized + 'static {
    type Cache: Cache<Self>;
    fn cached_path(&self) -> PathBuf;
    fn uri(&self) -> Result<Uri, download::Error>;
    fn reader(self, manager: DownloadManager, log: Logger) -> download::BoxFuture<fs::File> {
        Box::new(
            Self::Cache::with(self, manager, log).and_then(move |path| Ok(fs::File::open(path)?)),
        )
    }
    fn install_at(
        self,
        location: &Path,
        manager: DownloadManager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        Self::Cache::install_at(self, location.to_owned(), manager, log)
    }
}

pub trait Cache<T: Cacheable + Send + 'static> {
    fn is_cached(t: &T) -> bool {
        t.cached_path().exists()
    }

    fn with(t: T, manager: DownloadManager, log: Logger) -> download::BoxFuture<PathBuf>;

    #[async(boxed_send)]
    fn install_at(
        t: T,
        mut location: PathBuf,
        manager: DownloadManager,
        log: Logger,
    ) -> ::download::Result<()> {
        let cached_path = self::await!(Self::with(t, manager, log.clone()))?;
        info!(log, "installing item"; "location"=>location.as_path().to_string_lossy().into_owned());

        fs::create_dir_all(&location)?;

        if let Some(name) = cached_path.file_name() {
            location.push(name);
        }
        match util::symlink(cached_path, location, &log) {
            Err(util::SymlinkError::Io(ioe)) => return Err(ioe.into()),
            Err(util::SymlinkError::AlreadyExists) => {
                //TODO: verify the file, and replace/redownload it if needed
                warn!(log, "File already exist, assuming content is correct");
            }
            Ok(_) => {}
        }
        Ok(())
    }
}

fn first_file_in_folder<P: AsRef<Path>>(path: P) -> Result<PathBuf, download::Error> {
    Ok(path.as_ref()
        .read_dir()
        .map_err(|_| download::Error::CacheError)?
        .next()
        .ok_or_else(|| download::Error::CacheError)?
        .unwrap()
        .path())
}

pub struct FolderCache;

impl<T: Cacheable + Send + 'static> ::cache::Cache<T> for FolderCache {
    fn with(t: T, manager: DownloadManager, log: Logger) -> download::BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));

        if Self::is_cached(&t) {
            info!(log, "item was already cached");
            Box::new(
                future::result(first_file_in_folder(&cached_path)).or_else(move |_| {
                    //invalidate cache
                    warn!(log, "Removing invalid cache folder {:?}", cached_path);
                    future::result(fs::remove_dir(cached_path).map_err(download::Error::from))
                        .and_then(|_| {
                            //FIXME: will retry forever
                            //retry
                            Self::with(t, manager, log)
                        })
                }),
            )
        } else {
            info!(log, "item is not cached, downloading now");
            match t.uri() {
                Ok(uri) => Box::new(
                    manager
                        .download(uri, cached_path.clone(), true, &log)
                        .and_then(move |_| first_file_in_folder(cached_path)),
                ),
                Err(e) => Box::new(future::err(e)),
            }
        }
    }
}

pub struct FileCache;

impl<T: Cacheable + Send + 'static> ::cache::Cache<T> for FileCache {
    fn with(t: T, manager: DownloadManager, log: Logger) -> download::BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));

        if Self::is_cached(&t) {
            info!(log, "item was already cached");
            Box::new(future::ok(cached_path))
        } else {
            info!(log, "item is not cached, downloading now");
            match t.uri() {
                Ok(uri) => Box::new(
                    manager
                        .download(uri, cached_path.clone(), false, &log)
                        .map(move |_| cached_path),
                ),
                Err(e) => Box::new(future::err(e)),
            }
        }
    }
}

//TODO: does it make more sense to implement cachable in terms of downloadable?
//      (i.e. opposite of what we're doing here)
use download::Downloadable;

impl<C: Cacheable + Sync> Downloadable for C {
    fn download(
        self,
        location: PathBuf,
        manager: DownloadManager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        <Self as Cacheable>::Cache::install_at(self, location, manager, log)
    }
}
