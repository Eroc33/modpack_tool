mod update;
mod add;
pub mod upgrade;
pub use self::update::*;
pub use self::add::*;

use futures::prelude::*;
use tokio;
use std::path::PathBuf;

pub(crate) fn replace<P, R, FUT, F>(path: P, f: F) -> impl Future<Output=crate::Result<()>> + Send + 'static
where
    P: Into<PathBuf> + 'static,
    R: std::io::Read + Send,
    FUT: Future<Output=crate::Result<R>> + Send,
    F: FnOnce(tokio::fs::File) -> FUT + Send + 'static,
{
    let path = path.into();
    async move{
        let file = tokio::fs::File::open(path.clone()).await?;
        let mut out = f(file).await?;
        let out_file = tokio::fs::File::create(path).await?;
        //TODO: use async copy when possible: will require changing the type of R & updating to tokio 0.2.0-alpha-1
        std::io::copy(&mut out, &mut out_file.into_std())?;
        Ok(())
    }
}
