use std::path::PathBuf;
use futures::prelude::*;
use serde_json;
use std::io::Cursor;

pub fn add<P>(pack_path: P, mod_url: String) -> impl Future<Output=Result<(),crate::Error>> + Send + 'static
where
    P: Into<PathBuf> + 'static,
{
    use crate::types::ModpackConfig;

    super::replace(pack_path, |mut file| {
        async move{
            let mut pack: ModpackConfig = crate::async_json::read(&mut file).await.expect("pack file missing, or in bad format");

            pack.add_mod_by_url(mod_url.as_str())
                .expect("Unparseable modsource url");

            Ok(Cursor::new(serde_json::to_vec_pretty(&pack)?))
        }
    })
}
