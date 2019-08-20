use crate::{
    cache::Cache,
    curseforge,
    download::{self,Downloadable},
    forge_version,
    maven::{self, ResolvedArtifact},
    error::prelude::*,
};
use futures::prelude::*;
use http::{self, Uri};
use slog::Logger;
use std::{
    path::PathBuf,
    str::FromStr,
};
use semver;

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub enum ModSource {
    CurseforgeMod(curseforge::Mod),
    MavenMod {
        repo: String,
        artifact: maven::Artifact,
    },
}

impl ModSource {
    pub fn version_string(&self) -> String {
        match *self {
            Self::CurseforgeMod(ref modd) => modd.version.to_string(),
            Self::MavenMod { ref artifact, .. } => artifact.version.to_string(),
        }
    }
    pub fn identifier_string(&self) -> String {
        match *self {
            Self::CurseforgeMod(ref modd) => modd.id.clone(),
            Self::MavenMod { ref artifact, .. } => artifact.to_string(),
        }
    }
    pub fn guess_project_url(&self) -> Option<String> {
        match *self {
            Self::CurseforgeMod(ref modd) => {
                modd.project_uri().map(|uri| uri.to_string()).ok()
            }
            Self::MavenMod { .. } => None,
        }
    }
}

impl Downloadable for ModSource {
    fn download(
        self,
        location: PathBuf,
        manager: download::Manager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        Box::pin(async move{
            match self {
                Self::CurseforgeMod(modd) => {
                    curseforge::Cache::install_at(modd, location, manager, log).await.context(crate::download::error::Cached)?;
                }
                Self::MavenMod { repo, artifact } => {
                    let repo = Uri::from_str(repo.as_str()).context(crate::download::error::BadUri)?;
                    artifact.download_from(location.as_ref(), repo, manager, log).await.context(crate::download::error::Cached)?;
                }
            }
            Ok(())
        })
    }
}

pub type ModList = Vec<ModSource>;

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum IndirectableModpack{
    Real(ModpackConfig),
    Indirected(String),
}

impl IndirectableModpack{
    pub async fn resolve(self) -> Result<ModpackConfig,crate::Error>{
        match self{
            IndirectableModpack::Indirected(uri_str) => {
                let uri = Uri::from_str(&uri_str).context(error::Uri)?;
                let (res,_url) = crate::download::HttpSimple::new()
                    .get_following_redirects(uri)
                    .context(error::Download)?
                    .await
                    .context(error::Download)?;
                let data = res
                    .into_body()
                    .map_ok(hyper::Chunk::into_bytes).try_concat().await
                    .context(error::Http)?;
                Ok(serde_json::from_reader(std::io::Cursor::new(data)).context(error::Json)?)
            },
            IndirectableModpack::Real(modpack) => Ok(modpack),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModpackConfig {
    pub version: semver::VersionReq,
    pub name: String,
    pub forge: String,
    pub auto_update_release_status: Option<curseforge::ReleaseStatus>,
    pub mods: ModList,
    pub icon: Option<String>,
}

impl ModpackConfig {
    pub fn folder(&self) -> String {
        self.name.replace(|c: char| !c.is_alphanumeric(), "_")
    }
    pub fn forge_maven_artifact(&self) -> ResolvedArtifact {
        maven::Artifact {
            group: "net.minecraftforge".into(),
            artifact: "forge".into(),
            version: self.forge.clone(),
            classifier: Some("universal".into()),
            extension: Some("jar".into()),
        }.resolve(Uri::from_str(forge_version::BASE_URL).expect("const Uri should always be valid"))
    }
    pub fn replace_mod(&mut self, modsource: ModSource) {
        match modsource {
            ModSource::CurseforgeMod(curseforge::Mod { ref id, .. }) => {
                let new_id = id;
                let mut old_mods = vec![];
                ::std::mem::swap(&mut self.mods, &mut old_mods);
                self.mods = old_mods
                    .into_iter()
                    .filter(|source| match *source {
                        ModSource::CurseforgeMod(curseforge::Mod {
                            ref id,
                            ref version,
                        }) => {
                            if id == new_id {
                                println!("removing old version ({})", version);
                                false
                            } else {
                                true
                            }
                        }
                        _ => true,
                    })
                    .collect();
            }
            _ => panic!("Other mod sources not yet supported"),
        }

        println!("Adding: {:?}", modsource);

        self.mods.push(modsource);
    }
    pub fn add_mod_by_url(&mut self, mod_url: &str) -> crate::Result<()> {
        let modsource: ModSource = curseforge::Mod::from_url(mod_url)?.into();
        self.replace_mod(modsource);
        Ok(())
    }
    pub async fn load_maybe_indirected(file: &mut tokio::fs::File) -> Result<ModpackConfig,crate::Error>{
        let indirectable: IndirectableModpack = crate::async_json::read(file).await.context(NotAValidIndirectableModpack).erased()?;
        Ok(indirectable.resolve().await?)
    }
}

use snafu::Snafu;
#[derive(Debug,Snafu)]
pub enum Error{
    #[snafu(display("not a valid (indirectable) modpack config"))]
    NotAValidIndirectableModpack{
        source: crate::async_json::Error,
    }
}