use std::path::PathBuf;
use futures::prelude::*;
use structopt::StructOpt;
use crate::error::prelude::*;
use snafu::Snafu;
use crate::async_json;

#[derive(Debug, StructOpt)]
#[structopt(name = "add", about = "Adds a mod to the provided pack file.")]
pub struct Args{
    /// The metadata json file for the pack you wish to modify
    pack_file: PathBuf,
    /// The url for the mod you wish to add
    mod_url: String,
}

#[derive(Debug,Snafu)]
enum Error{
    #[snafu(display("pack {} does not exist",pack_file))]
    MissingPack{
        pack_file: String,
        source: std::io::Error,
    },
    #[snafu(display("pack file {} is missing or in bad format", pack_file))]
    BadPackfile{
        pack_file: String,
        source: async_json::Error,
    },
    #[snafu(display("error while creating new packfile: {}", pack_file))]
    CreatingPack{
        pack_file: String,
        source: std::io::Error,
    },
    #[snafu(display("error while writing packfile: {}", pack_file))]
    PackfileOutput{
        pack_file: String,
        source: async_json::Error,
    },
    #[snafu(display("Unparseable modsource url: {} ({})", url, source))]
    UnparseableModsourceUrl{
        url: String,
        source: crate::Error,
    },
}

pub fn add(args: Args) -> impl Future<Output=Result<(),crate::Error>> + Send + 'static
{
    let Args{pack_file, mod_url} = args;

    use crate::mod_source::ModpackConfig;

    async move{
        let mut file = tokio::fs::File::open(pack_file.clone()).await.context(MissingPack{pack_file: pack_file.display().to_string()}).erased()?;
        let mut pack: ModpackConfig = crate::async_json::read(&mut file).await.context(BadPackfile{pack_file: pack_file.display().to_string()}).erased()?;

        pack.add_mod_by_url(mod_url.as_str()).context(UnparseableModsourceUrl{url: mod_url}).erased()?;

        let mut out_file = tokio::fs::File::create(pack_file.clone()).await.context(CreatingPack{pack_file: pack_file.display().to_string()}).erased()?;
        async_json::write_pretty( &mut out_file, &pack).await.context(PackfileOutput{pack_file: pack_file.display().to_string()}).erased()?;
        Ok(())
    }
}
