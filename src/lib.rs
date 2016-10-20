#![feature(custom_derive,custom_attribute,slice_patterns,conservative_impl_trait,proc_macro)]
#![deny(clippy)]

#[macro_use]
extern crate serde_derive;
extern crate rustc_serialize;
extern crate serde;
extern crate serde_json;
extern crate hyper;
extern crate url;
extern crate sha1;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate error_chain;

#[macro_use]
extern crate slog;
extern crate time;
extern crate zip;
extern crate futures;

pub mod download;
pub mod util;
pub mod maven;
pub mod types;
pub mod forge_version;
pub mod hash_writer;
pub mod hacks;

pub use download::Downloadable;

pub use types::*;
