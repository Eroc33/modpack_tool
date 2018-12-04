use cache::Cache;
use curseforge;
use download::{self, DownloadManager, Downloadable};
use forge_version;
use futures::prelude::*;
use http::{self, Uri};
use maven::{MavenArtifact, ResolvedArtifact};
use slog::Logger;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use semver;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub enum ReleaseStatus {
    Release,
    Beta,
    Alpha,
}

#[derive(Debug)]
pub struct UnknownVariant(String);

impl ReleaseStatus {
    pub fn value(&self) -> &'static str {
        match *self {
            ReleaseStatus::Release => "Release",
            ReleaseStatus::Beta => "Beta",
            ReleaseStatus::Alpha => "Alpha",
        }
    }

    pub fn accepts(&self, other: &ReleaseStatus) -> bool {
        other == self || match *self {
            ReleaseStatus::Release => false,
            ReleaseStatus::Beta => ReleaseStatus::Release.accepts(other),
            ReleaseStatus::Alpha => ReleaseStatus::Beta.accepts(other),
        }
    }
}

impl FromStr for ReleaseStatus {
    type Err = UnknownVariant;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Release" => Ok(ReleaseStatus::Release),
            "Beta" => Ok(ReleaseStatus::Beta),
            "Alpha" => Ok(ReleaseStatus::Alpha),
            s => Err(UnknownVariant(s.to_string())),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub enum ModSource {
    CurseforgeMod(curseforge::Mod),
    MavenMod {
        repo: String,
        artifact: MavenArtifact,
    },
}

impl ModSource {
    pub fn version_string(&self) -> String {
        match *self {
            ModSource::CurseforgeMod(ref modd) => modd.version.to_string(),
            ModSource::MavenMod { ref artifact, .. } => artifact.version.to_string(),
        }
    }
    pub fn identifier_string(&self) -> String {
        match *self {
            ModSource::CurseforgeMod(ref modd) => modd.id.clone(),
            ModSource::MavenMod { ref artifact, .. } => artifact.to_string(),
        }
    }
    pub fn guess_project_url(&self) -> Option<String> {
        match *self {
            ModSource::CurseforgeMod(ref modd) => {
                modd.project_uri().map(|uri| uri.to_string()).ok()
            }
            ModSource::MavenMod { .. } => None,
        }
    }
}

impl Downloadable for ModSource {
    fn download(
        self,
        location: PathBuf,
        manager: DownloadManager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        match self {
            ModSource::CurseforgeMod(modd) => {
                curseforge::Cache::install_at(modd, location, manager, log)
            }
            ModSource::MavenMod { repo, artifact } => Box::new(async_block!{
                let repo = Uri::from_str(repo.as_str()).map_err(download::Error::from)?;
                self::await!(artifact.download_from(location.as_ref(), repo, manager, log))?;
                Ok(())
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum OS {
    #[serde(rename = "osx")]
    X,
    #[serde(rename = "windows")]
    Windows,
    #[serde(rename = "linux")]
    Linux,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OSSpec {
    pub name: OS,
}

impl OSSpec {
    pub fn matches(&self) -> bool {
        match self.name {
            OS::Windows => ::std::env::consts::OS == "windows",
            OS::Linux => ::std::env::consts::OS == "linux",
            OS::X => ::std::env::consts::OS == "macos",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Action {
    #[serde(rename = "allow")]
    Allow,
    #[serde(rename = "disallow")]
    Disallow,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Rule {
    pub action: Action,
    pub os: Option<OSSpec>,
}

impl Rule {
    pub fn os_matches(&self) -> bool {
        match self.os {
            None => true,
            Some(ref os) => os.matches(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Natives {
    pub linux: Option<String>,
    pub windows: Option<String>,
    pub osx: Option<String>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Extract {
    pub exclude: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MCLibraryListing {
    pub name: String,
    pub rules: Option<Vec<Rule>>,
    pub natives: Option<Natives>,
    pub extract: Option<Extract>,
}

impl MCLibraryListing {
    pub fn is_native(&self) -> bool {
        self.natives.is_some()
    }

    fn platform_native_classifier(&self) -> Option<String> {
        match ::std::env::consts::OS {
            "windows" => self.natives.clone().and_then(|n| n.windows),
            "linux" => self.natives.clone().and_then(|n| n.linux),
            "macos" => self.natives.clone().and_then(|n| n.osx),
            _ => None,
        }
    }
}

const MC_LIBS_MAVEN: &str = "https://libraries.minecraft.net/";

impl Downloadable for MCLibraryListing {
    #[async(boxed_send)]
    fn download(
        self,
        mut location: PathBuf,
        manager: DownloadManager,
        log: Logger,
    ) -> download::Result<()> {
        let resolved_artifact = if self.is_native() {
            let mut artifact = self.name.parse::<MavenArtifact>().unwrap();
            let disallowed = if let Some(ref rules) = self.rules {
                rules
                    .into_iter()
                    .any(|rule| rule.os_matches() && rule.action == Action::Disallow)
            } else {
                false
            };
            if disallowed {
                None //nothing to download
            } else {
                artifact.classifier = self.platform_native_classifier();
                let base = Uri::from_str(MC_LIBS_MAVEN)?;
                Some(artifact.resolve(base))
            }
        } else {
            let artifact = self.name.parse::<MavenArtifact>().unwrap();
            let base = Uri::from_str(MC_LIBS_MAVEN)?;
            Some(artifact.resolve(base))
        };
        if let Some(resolved_artifact) = resolved_artifact {
            location.push(resolved_artifact.to_path());
            self::await!(resolved_artifact.download(location, manager, log))?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MCVersionInfo {
    pub id: String,
    pub time: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "minecraftArguments")]
    pub minecraft_arguments: String,
    pub libraries: Vec<MCLibraryListing>,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(rename = "minimumLauncherVersion")]
    pub minimum_launcher_version: i64,
    pub assets: String,
}

use hyper;
use serde_json;
use std::str::FromStr;
use std::string::String;

pub type ModList = Vec<ModSource>;

impl MCVersionInfo {
    pub fn version(ver: &str) -> ::BoxFuture<MCVersionInfo> {
        let client = hyper::Client::new();
        let uri = Uri::from_str(
            format!(
                "http://s3.amazonaws.com/Minecraft.\
                 Download/versions/{0}/{0}.json",
                ver
            ).as_str(),
        ).unwrap();
        Box::new(
            client
                .get(uri)
                .map_err(::Error::from)
                .and_then(|res| {
                    res.into_body().map_err(::Error::from).fold(
                        vec![],
                        |mut buf, chunk| -> Result<Vec<u8>, ::Error> {
                            Cursor::new(chunk).read_to_end(&mut buf)?;
                            Ok(buf)
                        },
                    )
                })
                .and_then(|buf| {
                    let info: MCVersionInfo = serde_json::de::from_reader(Cursor::new(buf))?;
                    Ok(info)
                }),
        ) as ::BoxFuture<MCVersionInfo>
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModpackConfig {
    pub version: semver::VersionReq,
    pub name: String,
    pub forge: String,
    pub auto_update_release_status: Option<ReleaseStatus>,
    pub mods: ModList,
}

impl ModpackConfig {
    pub fn folder(&self) -> String {
        self.name.replace(|c: char| !c.is_alphanumeric(), "_")
    }
    pub fn forge_maven_artifact(&self) -> Result<ResolvedArtifact, http::uri::InvalidUri> {
        Ok(MavenArtifact {
            group: "net.minecraftforge".into(),
            artifact: "forge".into(),
            version: self.forge.clone(),
            classifier: Some("universal".into()),
            extension: Some("jar".into()),
        }.resolve(Uri::from_str(forge_version::BASE_URL)?))
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
    pub fn add_mod_by_url(&mut self, mod_url: &str) -> ::Result<()> {
        let modsource: ModSource = curseforge::Mod::from_url(mod_url)?.into();
        self.replace_mod(modsource);
        Ok(())
    }

    pub fn load<R>(reader: R) -> ::Result<Self>
    where
        R: Read,
    {
        Ok(serde_json::de::from_reader(reader)?)
    }

    pub fn save<W>(&self, writer: &mut W) -> ::Result<()>
    where
        W: Write,
    {
        serde_json::ser::to_writer_pretty(writer, &self)?;
        Ok(())
    }
}
