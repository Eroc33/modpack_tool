mod update;
pub mod dev;
pub use self::update::*;

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "modpacktool-update", version = "0.1", author = "E. Rochester <euan@rochester.me.uk>")]
pub enum Args{
    #[structopt(name="dev")]
    Dev(dev::Args),
    #[structopt(name="update", visible_alias = "install")]
    Update(update::Args),
}
impl Args{
    pub async fn dispatch(self, log: slog::Logger) -> crate::Result<()>
    {
        match self{
            Args::Update(update_args) => {
                update_args.dispatch(log).await
            }
            Args::Dev(dev_args) => {
                dev_args.dispatch(log).await
            }
        }
    }
}