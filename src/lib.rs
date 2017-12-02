#![feature(custom_derive,slice_patterns,conservative_impl_trait,never_type,generators,proc_macro)]

#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate hyper;
extern crate hyper_tls;
extern crate url;
extern crate sha1;
#[macro_use]
extern crate error_chain;
extern crate tokio_core;
extern crate semver;
#[macro_use]
extern crate slog;
extern crate time;
extern crate zip;
extern crate futures_await as futures;
#[macro_use]
extern crate nom;
extern crate kuchiki;
extern crate regex;
#[macro_use]
extern crate lazy_static;
extern crate termcolor;
//FIXME: has_class in kuchiki should probably not require selectors to be imported
//       maybe file a bug for this
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

error_chain! {
    links {
        Download(download::Error, download::ErrorKind);
    }
    foreign_links{
        Io(::std::io::Error);
        Uri(hyper::error::UriError);
        Hyper(hyper::Error);
        Zip(zip::result::ZipError);
        Json(serde_json::error::Error);
    }
    errors {
        ReportError(t: String){
            description("User facing error")
            display("{}",t)
        }
        Selector{
            description("couldn't compile selector")
            display("couldn't compile selector")
        }
        UnknownScheme(t: String) {
            description("unknown url scheme")
            display("unknown url scheme: '{}'", t)
        }
		BadModUrl(s: String){
			description("Couldn't parse mod url")
            display("Couldn't parse mod url: '{}'", s)
		}
    }
}

impl From<!> for Error{
    fn from(never:!)->Self{
        never
    }
}

pub type BoxFuture<I> = Box<futures::Future<Item = I, Error = Error>>;
