use download::{self,DownloadManager};
use slog::Logger;
use std::path::{PathBuf,Path};
use url::{self,Url};
use std::result::Result;
use std::fs;
use util;
use futures::{self, Future};

pub trait Cacheable{
    fn cached_path(&self) -> PathBuf;
    fn url(&self) -> Result<Url, url::ParseError>;
}

pub trait Cache<T: Cacheable>{
    fn is_cached(t: &T) -> bool {
        t.cached_path().exists()
    }
    
    fn with(t: &T,
                manager: DownloadManager,
                log: Logger)
                -> download::BoxFuture<PathBuf>;
                
    fn install_at(t: T,
                  mut location: PathBuf,
                  manager: DownloadManager,
                  log: Logger)
                  -> ::download::BoxFuture<()>{
        Self::with(&t, manager, log.clone()).and_then(move |cached_path| {
            info!(log, "installing item"; "location"=>location.as_path().to_string_lossy().into_owned());

            fs::create_dir_all(&location)?;
            
            cached_path.file_name().map(|n| location.push(n));
            util::symlink(cached_path, location, &log)?;
            Ok(())
        }).boxed()
    }
}

fn first_file_in_folder<P: AsRef<Path>>(path: P) -> PathBuf
{
    path.as_ref().read_dir().expect("cache folder was unreadable").next().expect("cache folder was empty").unwrap().path()
}

pub struct FolderCache;

impl<T: Cacheable> ::cache::Cache<T> for FolderCache {
    fn with(t: &T,
                manager: DownloadManager,
                log: Logger)
                -> download::BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));
        
        if !Self::is_cached(t) {
            info!(log, "item is not cached, downloading now");
            match t.url() {
                Ok(url) => {
                    manager.download(&url, cached_path.clone(), true, log)
                        .map(move |_| first_file_in_folder(cached_path))
                        .boxed()
                }
                Err(e) => futures::failed(download::Error::from(e)).boxed(),
            }
        } else {
            info!(log, "item was already cached");
            futures::finished(first_file_in_folder(cached_path)).boxed()
        }
    }
}

pub struct FileCache;

impl<T: Cacheable> ::cache::Cache<T> for FileCache {
    fn with(t: &T,
                manager: DownloadManager,
                log: Logger)
                -> download::BoxFuture<PathBuf> {
        let cached_path = t.cached_path();
        let log = log.new(o!("cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));
        
        if !Self::is_cached(t) {
            info!(log, "item is not cached, downloading now");
            match t.url() {
                Ok(url) => {
                    manager.download(&url, cached_path.clone(), false, log)
                        .map(move |_| cached_path)
                        .boxed()
                }
                Err(e) => futures::failed(download::Error::from(e)).boxed(),
            }
        } else {
            info!(log, "item was already cached");
            futures::finished(cached_path).boxed()
        }
    }
}
