mod upgrade;
pub use upgrade::upgrade;
mod add;
pub use add::add;

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "dev", about = "Commands for modpack developers.")]
pub enum Args{
    #[structopt(name="add")]
    Add(add::Args),
    #[structopt(name="upgrade")]
    Upgrade(upgrade::Args),
}

impl Args{
    pub async fn dispatch(self, log: slog::Logger) -> crate::Result<()>
    {
        match self{
            Args::Add(add_args) => {
                add(add_args).await
            }
            Args::Upgrade(upgrade_args) => {
                upgrade(upgrade_args).await
            }
        }
    }
}