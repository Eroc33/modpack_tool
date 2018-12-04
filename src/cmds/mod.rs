mod update;
mod add;
pub mod upgrade;
pub use self::update::*;
pub use self::add::*;

use futures::prelude::*;
use tokio;
use tokio::prelude::*;
use std::path::PathBuf;

pub(crate) fn replace<P, R, FUT, F>(path: P, f: F) -> impl Future<Item=(),Error=::Error> + Send + 'static
where
    P: Into<PathBuf> + 'static,
    R: AsyncRead + Send,
    FUT: Future<Item = R, Error = ::Error> + Send,
    F: FnOnce(tokio::fs::File) -> FUT + Send + 'static,
{
    let path = path.into();
    async_block!{
        let file = self::await!(tokio::fs::File::open(path.clone()))?;
        let out = self::await!(f(file))?;
        let out_file = self::await!(tokio::fs::File::create(path))?;
        self::await!(tokio::io::copy(out,out_file))?;
        Ok(())
    }
}
