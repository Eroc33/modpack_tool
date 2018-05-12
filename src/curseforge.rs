use http::{self, Uri};
use std::path::PathBuf;
use std::str::FromStr;
use download;

const CACHE_DIR: &str = "./curse_cache/";

pub fn parse_modid_from_url(url: &str) -> Result<String,::Error>{
    complete!(
        url,
        do_parse!(
            tag_s!("https://minecraft.curseforge.com/projects/") >>
            id: take_till_s!(|c: char| c == '/') >> 
            (id.to_owned())
        )
    ).to_full_result()
    .map_err(|_| ::Error::BadModUrl {
        url: url.to_owned(),
    })
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Mod {
    pub id: String,
    pub version: u64,
}

pub type Cache = ::cache::FolderCache;

impl Mod {
    pub fn project_uri(&self) -> Result<Uri, http::uri::InvalidUri> {
        let loc = format!("https://minecraft.curseforge.com/projects/{}/", self.id);
        Ok(Uri::from_str(&loc)?)
    }
}

impl ::cache::Cacheable for Mod {
    type Cache = ::cache::FolderCache;
    fn cached_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(CACHE_DIR);
        p.push(self.id.clone());
        p.push(self.version.to_string());
        p
    }

    fn uri(&self) -> Result<Uri, download::Error> {
        let loc = format!(
            "https://minecraft.curseforge.com/projects/{}/files/{}/download",
            self.id,
            self.version.to_string()
        );
        Ok(Uri::from_str(&loc)?)
    }
}
impl Into<::types::ModSource> for Mod {
    fn into(self) -> ::types::ModSource {
        ::types::ModSource::CurseforgeMod(self)
    }
}
