use http::{self, Uri};
use std::path::PathBuf;
use std::str::FromStr;
use crate::download;

const CACHE_DIR: &str = "./curse_cache/";

pub fn parse_modid_from_url(url: &str) -> Result<String,crate::Error>{
    complete!(
        url,
        do_parse!(
            tag_s!("https://www.curseforge.com/minecraft/mc-mods/") >>
            id: take_till_s!(|c: char| c == '/') >> 
            (id.to_owned())
        )
    ).to_full_result()
    .map_err(|_| crate::Error::BadModUrl {
        url: url.to_owned(),
    })
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Mod {
    pub id: String,
    pub version: u64,
}

pub type Cache = crate::cache::FolderCache;

impl Mod {
    pub fn project_uri(&self) -> Result<Uri, http::uri::InvalidUri> {
        let loc = format!("https://www.curseforge.com/minecraft/mc-mods/{}/", self.id);
        Ok(Uri::from_str(&loc)?)
    }

    pub fn files_uri(&self) -> Result<Uri, http::uri::InvalidUri> {
        let loc = format!("https://www.curseforge.com/minecraft/mc-mods/{}/files",self.id);
        Ok(Uri::from_str(&loc)?)
    }

    pub fn from_url(url: &str) -> crate::Result<Self>{
        complete!(
            &url,
            do_parse!(
                tag_s!("https://www.curseforge.com/minecraft/mc-mods/") >>
                id: take_till_s!(|c: char| c == '/') >> tag_s!("/download/") >>
                version: map_res!(take_while_s!(|c: char| c.is_digit(10)), u64::from_str) >>
                opt!(tag_s!("/file")) >>
                (Self {
                    id: id.to_owned(),
                    version,
                })
            )
        ).to_full_result()
        .map_err(|_| crate::Error::BadModUrl {
            url: url.to_owned(),
        })
    }
}

impl crate::cache::Cacheable for Mod {
    type Cache = crate::cache::FolderCache;
    fn cached_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(CACHE_DIR);
        p.push(self.id.clone());
        p.push(self.version.to_string());
        p
    }

    fn uri(&self) -> Result<Uri, download::Error> {
        let loc = format!(
            "https://www.curseforge.com/minecraft/mc-mods/{}/download/{}/file",
            self.id,
            self.version.to_string()
        );
        Ok(Uri::from_str(&loc)?)
    }
}
impl Into<crate::types::ModSource> for Mod {
    fn into(self) -> crate::types::ModSource {
        crate::types::ModSource::CurseforgeMod(self)
    }
}
