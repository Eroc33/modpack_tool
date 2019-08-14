mod update;
pub mod dev;
pub use self::update::*;

use futures::prelude::*;
use tokio;
use std::path::PathBuf;
use structopt::StructOpt;
use crate::mod_source::ModpackConfig;
use failure::ResultExt;

#[derive(Debug, StructOpt)]
#[structopt(name = "modpacktool-update", version = "0.1", author = "E. Rochester <euan@rochester.me.uk>")]
pub enum Args{
    #[structopt(name="dev")]
    Dev(dev::Args),
    #[structopt(name="update")]
    Update(update::Args),
}
impl Args{
    pub async fn dispatch(self, log: slog::Logger) -> crate::Result<()>
    {
        match self{
            Args::Update(update_args) => {
                if !update_args.pack_file.exists(){
                    eprintln!("{:?} is not an accesible path",update_args.pack_file);
                    Ok(())
                } else if !update_args.pack_file.is_file(){
                    eprintln!("No file exists at the path {:?}",update_args.pack_file);
                    Ok(())
                }else{
                    let mut file = tokio::fs::File::open(update_args.pack_file.clone()).await.context(format!("{:?} is not a file",&update_args.pack_file))?;
                    let pack = ModpackConfig::load_maybe_indirected(&mut file).await?;
                    update(pack,log).await
                }
            }
            Args::Dev(dev_args) => {
                dev_args.dispatch(log).await
            }
        }
    }
}

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
