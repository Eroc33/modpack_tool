#![feature(slice_patterns, never_type, generators, proc_macro_hygiene, async_await)]

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
use futures;

#[macro_use]
extern crate slog;
#[macro_use]
extern crate lazy_static;

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
pub mod error;

pub use download::Downloadable;

pub const APP_INFO: &app_dirs::AppInfo = &app_dirs::AppInfo{
    name: "Corrosive Modpack Tool",
    author: "Euan Rochester",
};

pub use error::{Error,Result};

pub type BoxFuture<I> = futures::future::BoxFuture<'static, Result<I>>;
