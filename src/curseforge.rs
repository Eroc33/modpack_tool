use url::{self,Url};
use std::str::FromStr;
use std::path::PathBuf;

const CACHE_DIR: &'static str = "./curse_cache/";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Mod {
    pub id: String,
    pub version: i64
}

pub type Cache = ::cache::FolderCache;

impl ::cache::Cacheable for Mod{
    fn cached_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(CACHE_DIR);
        p.push(self.id.clone());
        p.push(self.version.to_string());
        p
    }
    fn url(&self) -> Result<Url, url::ParseError>
    {
        let path = [self.id.as_str(), "files", self.version.to_string().as_str(), "download"]
            .join("/");
        let base = Url::from_str("http://minecraft.curseforge.com/projects/");
        base.and_then(|url| url.join(path.as_str()))
    }
}
impl Into<::types::ModSource> for Mod{
    fn into(self) -> ::types::ModSource
    {
        ::types::ModSource::CurseforgeMod(self)
    }
}