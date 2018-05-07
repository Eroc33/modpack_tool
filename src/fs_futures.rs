use std;
use std::path::Path;
#[macro_use]
use futures::try_ready;
use futures::prelude::*;
use futures::Async::*;
use tokio_threadpool;

use std::io;
use std::io::ErrorKind::Other;
use std::fs::DirEntry;

fn blocking_io<F, T>(f: F) -> Poll<T, io::Error>
where
    F: FnOnce() -> io::Result<T>,
{
    match tokio_threadpool::blocking(f) {
        Ok(Ready(Ok(v))) => Ok(v.into()),
        Ok(Ready(Err(err))) => Err(err),
        Ok(NotReady) => Ok(NotReady),
        Err(_) => Err(blocking_err()),
    }
}

fn blocking_io_strm<F, T>(f: F) -> Poll<Option<T>, io::Error>
where
    F: FnOnce() -> Option<io::Result<T>>,
{
    match tokio_threadpool::blocking(f) {
        Ok(Ready(Some(Ok(v)))) => Ok(Some(v).into()),
        Ok(Ready(Some(Err(err)))) => Err(err),
        Ok(Ready(None)) => Ok(None.into()),
        Ok(NotReady) => Ok(NotReady),
        Err(_) => Err(blocking_err()),
    }
}

fn blocking_err() -> io::Error {
    io::Error::new(
        Other,
        "fs_futures must be called \
         from the context of the Tokio runtime.",
    )
}

/// Future returned by `create_dir_all`.
#[derive(Debug)]
pub struct CreateDirAll<P> {
    path: P,
}

impl<P> CreateDirAll<P>
where
    P: AsRef<Path> + Send + 'static,
{
    pub(crate) fn new(path: P) -> Self {
        CreateDirAll { path }
    }
}

impl<P> Future for CreateDirAll<P>
where
    P: AsRef<Path> + Send + 'static,
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let res = try_ready!(blocking_io(|| std::fs::create_dir_all(&self.path)));
        Ok(res.into())
    }
}

pub fn create_dir_all<P>(path: P) -> CreateDirAll<P>
where
    P: AsRef<Path> + Send + 'static,
{
    CreateDirAll::new(path)
}

/// Future returned by `create_dir_all`.
#[derive(Debug)]
pub struct RemoveFile<P> {
    path: P,
}

impl<P> RemoveFile<P>
where
    P: AsRef<Path> + Send + 'static,
{
    pub(crate) fn new(path: P) -> Self {
        RemoveFile { path }
    }
}

impl<P> Future for RemoveFile<P>
where
    P: AsRef<Path> + Send + 'static,
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let res = try_ready!(blocking_io(|| std::fs::remove_file(&self.path)));
        Ok(res.into())
    }
}

pub fn remove_file<P>(path: P) -> RemoveFile<P>
where
    P: AsRef<Path> + Send + 'static,
{
    RemoveFile::new(path)
}

/// Stream returned by `read_dir`.
#[derive(Debug)]
pub struct ReadDir {
    read_dir: std::fs::ReadDir,
}

impl ReadDir {
    pub(crate) fn new(read_dir: std::fs::ReadDir) -> Self {
        ReadDir { read_dir }
    }
}

impl Stream for ReadDir {
    type Item = DirEntry;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let res = try_ready!(blocking_io_strm(|| self.read_dir.next()));
        Ok(res.into())
    }
}

pub fn read_dir<P>(path: P) -> std::io::Result<ReadDir>
where
    P: AsRef<Path> + Send + 'static,
{
    Ok(ReadDir::new(std::fs::read_dir(path)?))
}
