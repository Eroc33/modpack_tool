#![feature(slice_patterns, never_type, generators, proc_macro_hygiene, async_await)]

#[macro_use]
extern crate serde_derive;
use failure;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate failure_derive;
use futures;
use http;
use hyper;

#[macro_use]
extern crate slog;
use zip;
#[macro_use]
extern crate lazy_static;

use failure::Context;

pub mod cache;
pub mod curseforge;
pub mod download;
pub mod util;
pub mod maven;
pub mod mod_source;
pub mod forge_version;
pub mod hacks;
pub mod cmds;
pub mod async_json;
pub mod mc_libs;

pub use download::Downloadable;

pub const APP_INFO: &app_dirs::AppInfo = &app_dirs::AppInfo{
    name: "Corrosive Modpack Tool",
    author: "Euan Rochester",
};

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "Download error: {}", _0)]
    Download(#[cause] download::Error),
    #[fail(display = "IO error: {}", _0)]
    Io(#[cause] ::std::io::Error),
    #[fail(display = "Uri error: {}", _0)]
    Uri(#[cause] http::uri::InvalidUri),
    #[fail(display = "Hyper error: {}", _0)]
    Hyper(#[cause] hyper::Error),
    #[fail(display = "Zip error: {}", _0)]
    Zip(#[cause] zip::result::ZipError),
    #[fail(display = "JSON error: {}", _0)]
    Json(#[cause] serde_json::error::Error),
    #[fail(display = "{}", _0)]
    Report(Context<String>),
    #[fail(display = "couldn't compile selector")]
    Selector,
    #[fail(display = "unknown url scheme: '{}'", scheme)]
    UnknownScheme { scheme: String },
    #[fail(display = "Couldn't parse mod url: '{}'", url)]
    BadModUrl { url: String },
    #[fail(display = "Packs must have an auto_update_release_status to be able to auto update")]
    AutoUpdateDisabled,
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<crate::async_json::Error> for Error{
    fn from(err: crate::async_json::Error) -> Self {
        match err{
            crate::async_json::Error::Io(io) => Self::Io(io),
            crate::async_json::Error::Json(io) => Self::Json(io),
        }
    }
}

impl From<!> for Error {
    fn from(never: !) -> Self {
        never
    }
}

impl From<http::uri::InvalidUri> for Error {
    fn from(err: http::uri::InvalidUri) -> Self {
        Self::Uri(err)
    }
}

impl From<hyper::Error> for Error {
    fn from(err: hyper::Error) -> Self {
        Self::Hyper(err)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(err: zip::result::ZipError) -> Self {
        Self::Zip(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<download::Error> for Error {
    fn from(err: download::Error) -> Self {
        Self::Download(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<Context<&str>> for Error {
    fn from(err: Context<&str>) -> Self {
        Self::Report(err.map(|s| s.to_string()))
    }
}

impl From<Context<String>> for Error {
    fn from(err: Context<String>) -> Self {
        Self::Report(err)
    }
}

pub type BoxFuture<I> = futures::future::BoxFuture<'static, Result<I>>;
