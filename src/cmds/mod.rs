mod update;
mod add;
pub mod upgrade;
pub use self::update::*;
pub use self::add::*;

use futures::prelude::*;
use tokio;
use tokio::prelude::*;
use BoxFuture;
use std::path::PathBuf;

pub(crate) fn replace<P, R, FUT, F>(path: P, f: F) -> BoxFuture<()>
where
    P: Into<PathBuf>,
    R: AsyncRead + Send,
    FUT: Future<Item = R, Error = ::Error> + Send,
    F: FnOnce(tokio::fs::File) -> FUT + Send + 'static,
{
    let path = path.into();
    Box::new(async_block!{
        let file = await!(tokio::fs::File::open(path.clone()))?;
        let out = await!(f(file))?;
        let out_file = await!(tokio::fs::File::create(path))?;
        await!(tokio::io::copy(out,out_file))?;
        Ok(())
    })
}
