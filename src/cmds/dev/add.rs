use std::path::PathBuf;
use futures::prelude::*;
use serde_json;
use std::io::Cursor;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "add", about = "Adds a mod to the provided pack file.")]
pub struct Args{
    /// The metadata json file for the pack you wish to modify
    pack_file: PathBuf,
    /// The url for the mod you wish to add
    mod_url: String,
}

pub fn add(args: Args) -> impl Future<Output=Result<(),crate::Error>> + Send + 'static
{
    let Args{pack_file, mod_url} = args;

    use crate::mod_source::ModpackConfig;

    crate::cmds::replace(pack_file, |mut file| {
        async move{
            let mut pack: ModpackConfig = crate::async_json::read(&mut file).await.expect("pack file missing, or in bad format");

            pack.add_mod_by_url(mod_url.as_str())
                .expect("Unparseable modsource url");

            Ok(Cursor::new(serde_json::to_vec_pretty(&pack)?))
        }
    })
}
