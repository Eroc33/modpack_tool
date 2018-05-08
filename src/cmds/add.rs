use std::path::PathBuf;
use BoxFuture;
use futures::prelude::*;
use serde_json;
use std::io::Cursor;

pub fn add<P>(pack_path: P, mod_url: String) -> BoxFuture<()>
where
    P: Into<PathBuf>,
{
    use types::ModpackConfig;

    super::replace(pack_path, |file| {
        async_block!{
            let mut pack: ModpackConfig =
                serde_json::de::from_reader(file).expect("pack file in bad format");

            pack.add_mod_by_url(mod_url.as_str())
                .expect("Unparseable modsource url");

            Ok(Cursor::new(serde_json::to_vec_pretty(&pack)?))
        }
    })
}
