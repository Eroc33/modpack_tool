use crate::download;
use slog::Logger;
use std::path::{Path, PathBuf};
use std::result::Result;
use crate::util;
use http::Uri;

pub trait Cacheable: Send + Sized + 'static {
    type Cache: Cache<Self>;
    fn cached_path(&self) -> PathBuf;
    fn uri(&self) -> Result<Uri, download::Error>;
    fn reader(self, manager: download::Manager, log: Logger) -> download::BoxFuture<tokio::fs::File> {
        Box::pin(async move{
            let path = Self::Cache::with(self, manager, log).await?;
            Ok(tokio::fs::File::open(path).await?)
        })
    }
    fn install_at(
        self,
        location: &Path,
        manager: download::Manager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        Self::Cache::install_at(self, location.to_owned(), manager, log)
    }
}

pub trait Cache<T: Cacheable + Send + 'static> {
    fn is_cached(t: &T) -> bool {
        t.cached_path().exists()
    }

    fn with(t: T, manager: download::Manager, log: Logger) -> download::BoxFuture<PathBuf>;

    fn install_at(
        t: T,
        mut location: PathBuf,
        manager: download::Manager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        Box::pin(async move{
            let cached_path = Self::with(t, manager, log.clone()).await?;
            info!(log, "installing item"; "location"=>location.as_path().to_string_lossy().into_owned());

            tokio::fs::create_dir_all(&location).await?;

            if let Some(name) = cached_path.file_name() {
                location.push(name);
            }
            match util::symlink(cached_path, location, &log).await {
                Err(util::SymlinkError::Io(ioe)) => return Err(ioe.into()),
                Err(util::SymlinkError::AlreadyExists) => {
                    //TODO: verify the file, and replace/redownload it if needed
                    warn!(log, "File already exist, assuming content is correct");
                }
                Ok(_) => {}
            }
            Ok(())
        })
    }
}

fn first_file_in_folder<P: AsRef<Path>>(path: P) -> Result<PathBuf, download::Error> {
    Ok(path.as_ref()
        .read_dir()
        .map_err(|_| download::Error::Cache)?
        .next()
        .ok_or_else(|| download::Error::Cache)?
        .unwrap()
        .path())
}

pub struct FolderCache;

impl<T: Cacheable + Send + 'static> crate::cache::Cache<T> for FolderCache {
    fn with(t: T, manager: download::Manager, log: Logger) -> download::BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));

        Box::pin(async move{
            if !Self::is_cached(&t){
                info!(log, "item is not cached, downloading now");
                let uri = t.uri()?;
                manager.download(uri, cached_path.clone(), true, &log).await?;
            }
            if let Ok(path) = first_file_in_folder(&cached_path){
                return Ok(path);
            }else{
                //invalidate cache
                warn!(log, "Removing invalid cache folder {:?}", cached_path);
                tokio::fs::remove_dir(&cached_path).await?;
                //FIXME: will retry forever
                //retry
                Ok(Self::with(t, manager, log).await?)
            }
        })
    }
}

pub struct FileCache;

impl<T: Cacheable + Send + 'static> crate::cache::Cache<T> for FileCache {
    fn with(t: T, manager: download::Manager, log: Logger) -> download::BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));

        Box::pin(async move{
            if !Self::is_cached(&t) {
                info!(log, "item is not cached, downloading now");
                let uri = t.uri()?;
                manager.download(uri, cached_path.clone(), false, &log).await?;
            }
            Ok(cached_path)
        })
    }
}

//TODO: does it make more sense to implement cachable in terms of downloadable?
//      (i.e. opposite of what we're doing here)
use download::Downloadable;

impl<C: Cacheable + Sync> Downloadable for C {
    fn download(
        self,
        location: PathBuf,
        manager: download::Manager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        <Self as Cacheable>::Cache::install_at(self, location, manager, log)
    }
}
