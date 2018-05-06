#![feature(custom_derive, slice_patterns, never_type, generators, proc_macro, proc_macro_non_items)]

#[macro_use] extern crate serde_derive;
extern crate serde_json;
#[macro_use] extern crate failure;
#[macro_use] extern crate failure_derive;
extern crate tokio;
extern crate futures_await as futures;
extern crate hyper;
extern crate hyper_tls;
extern crate http;
extern crate url;
extern crate sha1;
extern crate semver;
#[macro_use] extern crate slog;
extern crate time;
extern crate zip;
#[macro_use] extern crate nom;
extern crate kuchiki;
extern crate regex;
#[macro_use] extern crate lazy_static;
extern crate termcolor;
//FIXME: has_class in kuchiki should probably not require selectors to be imported
//       maybe file a bug for this
extern crate chrono;
extern crate selectors;

pub mod cache;
pub mod curseforge;
pub mod download;
pub mod util;
pub mod maven;
pub mod types;
pub mod forge_version;
pub mod hash_writer;
pub mod hacks;
pub mod upgrade;

pub use download::Downloadable;

pub use types::*;

#[macro_export]
macro_rules! die{
    ($($items:expr),+) => {{
        eprintln!($($items),+);
        std::process::exit(1)
    }}
}

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
    #[fail(display = "invalid toolchain name: {}", _0)]
    ReportError(String),
    #[fail(display = "couldn't compile selector")]
    Selector,
    #[fail(display = "unknown url scheme: '{}'", scheme)]
    UnknownScheme { scheme: String },
    #[fail(display = "Couldn't parse mod url: '{}'", url)]
    BadModUrl { url: String },
}

pub type Result<T> = ::std::result::Result<T, ::Error>;

impl From<!> for Error {
    fn from(never: !) -> Self {
        never
    }
}

impl From<http::uri::InvalidUri> for Error {
    fn from(err: http::uri::InvalidUri) -> Self {
        Error::Uri(err)
    }
}

impl From<hyper::Error> for Error {
    fn from(err: hyper::Error) -> Self {
        Error::Hyper(err)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(err: zip::result::ZipError) -> Self {
        Error::Zip(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Json(err)
    }
}

impl From<download::Error> for Error {
    fn from(err: download::Error) -> Self {
        Error::Download(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

pub type BoxFuture<I> = Box<futures::Future<Item = I, Error = Error> + Send + 'static>;
