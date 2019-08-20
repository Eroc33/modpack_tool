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

pub type Result<T> = StdResult<T, Error>;
pub type BoxFuture<T> = futures::future::BoxFuture<'static,Result<T>>;

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

pub trait TryFutureExt<T>{
    fn erased(self) -> BoxFuture<T>;
}

impl<Fut,T,E: std::error::Error + Send + Sync + 'static> TryFutureExt<T> for Fut
    where Fut: std::future::Future<Output=StdResult<T,E>> + Send + 'static
{
    fn erased(self) -> BoxFuture<T>
    {
        use futures::TryFutureExt  as _;
        use snafu::futures::TryFutureExt as _;
        Box::pin(self.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>).context(Dynamic))
    }
}

pub mod prelude{
    pub use crate::error::{
        self,
        ResultExt as _,
        TryFutureExt as _,
    };
    pub use snafu::{
        ResultExt as _,
        OptionExt as _,
        IntoError as _,
        futures::TryFutureExt as _,
    };
}