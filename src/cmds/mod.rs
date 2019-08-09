mod update;
mod add;
pub mod upgrade;
pub use self::update::*;
pub use self::add::*;

use futures::prelude::*;
use tokio;
use std::path::PathBuf;

use tokio::io::{AsyncRead,AsyncReadExt};

pub(crate) fn replace<P, R, FUT, F>(path: P, f: F) -> impl Future<Output=crate::Result<()>> + Send + 'static
where
    P: Into<PathBuf> + 'static,
    R: AsyncRead + Unpin + Send,
    FUT: Future<Output=crate::Result<R>> + Send,
    F: FnOnce(tokio::fs::File) -> FUT + Send + 'static,
{
    let path = path.into();
    async move{
        let file = tokio::fs::File::open(path.clone()).await?;
        let mut out = f(file).await?;
        let mut out_file = tokio::fs::File::create(path).await?;
        out.copy(&mut out_file).await?;
        Ok(())
    }
}
