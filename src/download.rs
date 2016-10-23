// this allow can be removed after error_chain 0.5.1 or greater is released
#![allow(redundant_closure)]

use futures;
use futures::Future as FutureTrait;
use hyper;
use slog::Logger;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use time;
use url::{self, Url};
use util;

error_chain! {
  foreign_links {
    ::std::io::Error, Io;
    url::ParseError, Url;
    hyper::Error, Hyper;
    time::OutOfRangeError, DurationOutOfRange;
    ::std::time::SystemTimeError, StdTimeError;
  }
}

// trait alias
pub trait Future<I>: futures::Future<Item = I, Error = ::download::Error> {}
impl<I, T: futures::Future<Item = I, Error = ::download::Error>> Future<I> for T {}

pub type BoxFuture<I> = Box<::futures::Future<Item = I, Error = ::download::Error> + Send>;

pub trait Downloadable: Sync {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()>;
}

impl<D: Downloadable + Send> Downloadable for Vec<D> {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        futures::collect(self.into_iter()
                .map(move |d| d.download(location.clone(), manager.clone(), log.clone()))
                .collect::<Vec<BoxFuture<()>>>()
                .into_iter())
            .map(|_| ())
            .boxed()
    }
}

impl<'a, D: Downloadable + Send + Clone> Downloadable for &'a [D] {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        futures::collect(self.into_iter()
                .map(move |d| {
                    d.clone().download(location.clone(), manager.clone(), log.clone())
                })
                .collect::<Vec<BoxFuture<()>>>()
                .into_iter())
            .map(|_| ())
            .boxed()
    }
}

impl Downloadable for Url {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        manager.download(&self, location, false, log).boxed()
    }
}

#[derive(Clone,Default)]
pub struct DownloadManager {
    handle: Arc<hyper::Client>,
}

impl DownloadManager {
    pub fn new() -> Self {
        DownloadManager { handle: Arc::new(hyper::Client::new()) }
    }

    pub fn get<U: hyper::client::IntoUrl>(&self, url: U) -> hyper::client::RequestBuilder {
        self.handle.deref().get(url)
    }

    pub fn download(&self,
                    url: &Url,
                    path: PathBuf,
                    append_filename: bool,
                    log: Logger)
                    -> impl Future<()> {
        util::download_with(url, path, append_filename, self.handle.deref(), log)
    }
}
