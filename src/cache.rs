use crate::download;
use slog::Logger;
use std::path::{Path, PathBuf};
use std::result::Result as StdResult;
use crate::{
    util,
    error::prelude::*,
};
use http::Uri;

pub mod error{
    use snafu::Snafu;
    #[derive(Debug,Snafu)]
    #[snafu(visibility(pub))]
    pub enum Error{
        #[snafu(display("Io error {} while opening cached file: {}", source, path))]
        OpeningCached{
            path: String,
            source: std::io::Error,
        },
        #[snafu(display("Io error {} while creating install dir: {}", source, path))]
        CreatingInstallDir{
            path: String,
            source: std::io::Error,
        },
        #[snafu(display("Io error {} while symlinking {} to {}", source, from, to))]
        Symlink{
            from: String,
            to: String,
            source: std::io::Error,
        },
        #[snafu(display("Error {} while downloading {}", source, uri))]
        Downloading{
            uri: http::Uri,
            source: crate::download::Error,
        },
        #[snafu(display("Io error {} while removing cached item {}", source, path))]
        RemovingOldCached{
            path: String,
            source: std::io::Error,
        },
        #[snafu(display("Invalid uri for item fetched through cache: {}", source))]
        BadUri{
            source: http::uri::InvalidUri,
        },
        #[snafu(display("Error {} while finding first file in folder: {}", source, path))]
        FirstFileInFolder{
            path: String,
            source: std::io::Error,
        },
        #[snafu(display("No files in folder while finding first file in folder: {}", path))]
        NoFilesInFolder{
            path: String,
        },
        #[snafu(display("{}", source))]
        Dynamic{
            source: Box<dyn std::error::Error + Send + Sync>
        },
    }
}

pub use error::Error;

pub type Result<T> = StdResult<T, Error>;

pub trait ResultExt<T>{
    fn erased(self) -> Result<T>;
}

impl<T,E: std::error::Error + Send + Sync + 'static> ResultExt<T> for StdResult<T,E>{
    fn erased(self) -> Result<T>
    {
        use snafu::ResultExt;
        self.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>).context(error::Dynamic)
    }
}

pub type BoxFuture<T> = futures::future::BoxFuture<'static, Result<T>>;

pub trait Cacheable: Send + Sized + 'static {
    type Cache: Cache<Self>;
    fn cached_path(&self) -> PathBuf;
    fn uri(&self) -> Result<Uri>;
    fn reader(self, manager: download::Manager, log: Logger) -> BoxFuture<tokio::fs::File> {
        Box::pin(async move{
            let path = Self::Cache::with(self, manager, log).await?;
            Ok(tokio::fs::File::open(path.clone()).await.context(error::OpeningCached{path: path.display().to_string()})?)
        })
    }
    fn install_at(
        self,
        location: &Path,
        manager: download::Manager,
        log: Logger,
    ) -> BoxFuture<()> {
        Self::Cache::install_at(self, location.to_owned(), manager, log)
    }
}

pub trait Cache<T: Cacheable + Send + 'static> {
    fn is_cached(t: &T) -> bool {
        t.cached_path().exists()
    }

    fn with(t: T, manager: download::Manager, log: Logger) -> BoxFuture<PathBuf>;

    fn install_at(
        t: T,
        mut location: PathBuf,
        manager: download::Manager,
        log: Logger,
    ) -> BoxFuture<()> {
        Box::pin(async move{
            let cached_path = Self::with(t, manager, log.clone()).await?;
            info!(log, "installing item"; "location"=>location.as_path().to_string_lossy().into_owned());

            tokio::fs::create_dir_all(&location).await.context(error::CreatingInstallDir{path: location.display().to_string()})?;

            if let Some(name) = cached_path.file_name() {
                location.push(name);
            }
            match util::symlink(cached_path.clone(), location.clone(), &log).await {
                Err(util::SymlinkError::Io{source}) => return Err(error::Symlink{from: cached_path.display().to_string(), to: location.display().to_string()}.into_error(source)),
                Err(util::SymlinkError::AlreadyExists) => {
                    //TODO: verify the file, and replace/redownload it if needed
                    warn!(log, "File already exists, assuming content is correct");
                }
                Ok(_) => {}
            }
            Ok(())
        })
    }
}

fn first_file_in_folder<P: AsRef<Path>>(path: P) -> Result<PathBuf> {
    let path = path.as_ref();
    Ok(path
        .read_dir()
        .context(error::FirstFileInFolder{path: path.display().to_string()})?
        .next()
        .context(error::NoFilesInFolder{path: path.display().to_string()})?
        .unwrap()
        .path())
}

pub struct FolderCache;

impl<T: Cacheable + Send + 'static> Cache<T> for FolderCache {
    fn with(t: T, manager: download::Manager, log: Logger) -> BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));

        Box::pin(async move{
            if !Self::is_cached(&t){
                info!(log, "item is not cached, downloading now");
                let uri = t.uri()?;
                manager.download(uri.clone(), cached_path.clone(), true, &log).await.context(error::Downloading{uri})?;
            }
            if let Ok(path) = first_file_in_folder(&cached_path){
                return Ok(path);
            }else{
                //invalidate cache
                warn!(log, "Removing invalid cache folder {:?}", cached_path);
                tokio::fs::remove_dir(&cached_path).await.context(error::RemovingOldCached{path: cached_path.display().to_string()})?;
                //FIXME: will retry forever
                //retry
                Ok(Self::with(t, manager, log).await?)
            }
        })
    }
}

pub struct FileCache;

impl<T: Cacheable + Send + 'static> Cache<T> for FileCache {
    fn with(t: T, manager: download::Manager, log: Logger) -> BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));

        Box::pin(async move{
            if !Self::is_cached(&t) {
                info!(log, "item is not cached, downloading now");
                let uri = t.uri()?;
                manager.download(uri.clone(), cached_path.clone(), false, &log).await.context(error::Downloading{uri})?;
            }
            Ok(cached_path)
        })
    }
}

//TODO: does it make more sense to implement cacheable in terms of downloadable?
//      (i.e. opposite of what we're doing here)
use download::Downloadable;

impl<C: Cacheable + Sync> Downloadable for C {
    fn download(
        self,
        location: PathBuf,
        manager: download::Manager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        Box::pin(async move{
            <Self as Cacheable>::Cache::install_at(self, location, manager, log).await.context(download::error::Cached)?;
            Ok(())
        })
    }
}
