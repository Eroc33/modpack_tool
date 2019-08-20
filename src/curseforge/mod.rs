use http::{self, Uri};
use std::path::PathBuf;
use std::str::FromStr;
use crate::{
    error::prelude::*,
};

pub mod api;
mod release_status;
pub use release_status::*;

pub fn parse_modid_from_url(url: &str) -> Result<String,crate::Error>{
    use nom::bytes::complete::*;

    fn error<'a, I>(url: &'a str) -> impl (Fn(nom::Err<(I,nom::error::ErrorKind)>) -> crate::Error) + 'a{
        move |_|{
            crate::Error::BadModUrl {
                url: url.to_owned(),
            }
        }
    }

    let (rest,_tag) = tag("https://www.curseforge.com/minecraft/mc-mods/")(url).map_err(error(url))?;
    let (_rest, id) = take_till(|c: char| c == '/')(rest).map_err(error(url))?;

    Ok(id.to_string())
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
        use nom::bytes::complete::*;
        use nom::branch::*;
        use nom::combinator::*;

        fn error<'a, I>(url: &'a str) -> impl (Fn(nom::Err<(I,nom::error::ErrorKind)>) -> crate::Error) + 'a{
            move |_|{
                crate::Error::BadModUrl {
                    url: url.to_owned(),
                }
            }
        }

        let (rest,_tag) = tag("https://www.curseforge.com/minecraft/mc-mods/")(url).map_err(error(url))?;
        let (rest,id) = take_till(|c: char| c == '/')(rest).map_err(error(url))?;
        let (rest,_tag) = alt((tag("/download/"),tag("/files/")))(rest).map_err(error(url))?;
        let (rest,version) = map_res(take_while(|c: char| c.is_digit(10)), u64::from_str)(rest).map_err(error(url))?;
        let (_rest,_tag) = opt(tag("/file"))(rest).map_err(error(url))?;

        Ok(Self{
            id: id.to_owned(),
            version,
        })
    }
}

impl crate::cache::Cacheable for Mod {
    type Cache = crate::cache::FolderCache;
    fn cached_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(app_dirs::app_dir(app_dirs::AppDataType::UserCache, crate::APP_INFO, "curse_cache").expect("Cache directory must be accesible"));
        p.push(self.id.clone());
        p.push(self.version.to_string());
        p
    }

    fn uri(&self) -> Result<Uri, crate::cache::Error> {
        let loc = format!(
            "https://www.curseforge.com/minecraft/mc-mods/{}/download/{}/file",
            self.id,
            self.version.to_string()
        );
        Ok(Uri::from_str(&loc).context(crate::cache::error::BadUri)?)
    }
}
impl Into<crate::mod_source::ModSource> for Mod {
    fn into(self) -> crate::mod_source::ModSource {
        crate::mod_source::ModSource::CurseforgeMod(self)
    }
}
