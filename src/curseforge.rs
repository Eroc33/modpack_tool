use hyper::{self, Uri};
use std::path::PathBuf;
use std::str::FromStr;

const CACHE_DIR: & str = "./curse_cache/";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Mod {
    pub id: String,
    pub version: u64,
}

pub type Cache = ::cache::FolderCache;

impl Mod{
    pub fn project_uri(&self) -> Result<Uri, hyper::error::UriError> {
        let loc = format!("https://minecraft.curseforge.com/projects/{}/",
                          self.id);
        Ok(Uri::from_str(&loc)?)
    }
}

impl ::cache::Cacheable for Mod {
    fn cached_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(CACHE_DIR);
        p.push(self.id.clone());
        p.push(self.version.to_string());
        p
    }

    fn uri(&self) -> Result<Uri, hyper::error::UriError> {
        let loc = format!("https://minecraft.curseforge.com/projects/{}/files/{}/download",
                          self.id,
                          self.version.to_string());
        Ok(Uri::from_str(&loc)?)
    }
}
impl Into<::types::ModSource> for Mod {
    fn into(self) -> ::types::ModSource {
        ::types::ModSource::CurseforgeMod(self)
    }
}
