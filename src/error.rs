use snafu::Snafu;
use http;
use hyper;
use zip;

use std::result::Result as StdResult;

#[derive(Debug,Snafu)]
#[snafu(visibility(pub))]
pub enum Error{
    #[snafu(display("Pack {} has no auto_update_release_status", pack_file))]
    AutoUpdateDisabled{
        pack_file: String,
    },
    #[snafu(display("Http error {}", source))]
    Http{
        source: hyper::Error
    },
    #[snafu(display("Json error {}", source))]
    Json{
        source: serde_json::error::Error
    },
    #[snafu(display("Async Json error {}", source))]
    AsyncJson{
        source: crate::async_json::Error,
    },
    #[snafu(display("Download error {}", source))]
    Download{
        source: crate::download::Error
    },
    #[snafu(display("Console io error {}", source))]
    Console{
        source: std::io::Error
    },
    #[snafu(display("Io error {}", source))]
    Io{
        source: std::io::Error
    },
    #[snafu(display("Uri error {}", source))]
    Uri{
        source: http::uri::InvalidUri
    },
    #[snafu(display("Zip error {}", source))]
    Zip{
        source: zip::result::ZipError,
    },
    #[snafu(display("Couldn't compile selector"))]
    Selector,
    #[snafu(display("Unknown url scheme: `{}`",scheme))]
    Scheme{
        scheme: String,
    },
    #[snafu(display("Couldn't parse mod url: `{}`",url))]
    BadModUrl{
        url: String,
    },
    #[snafu(display("{}",source))]
    Dynamic{
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

// #[derive(Debug, Snafu)]
// pub enum Error {
//     #[fail(display = "{}", _0)]
//     Report(Context<String>),
// }

pub type Result<T> = StdResult<T, Error>;

pub trait ResultExt<T>{
    fn erased(self) -> Result<T>;
}

impl<T,E: std::error::Error + Send + Sync + 'static> ResultExt<T> for StdResult<T,E>{
    fn erased(self) -> Result<T>
    {
        use snafu::ResultExt;
        self.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>).context(Dynamic)
    }
}

pub mod prelude{
    pub use crate::error;
    pub use crate::error::ResultExt as _;
    pub use snafu::ResultExt as _;
    pub use snafu::IntoError as _;
}