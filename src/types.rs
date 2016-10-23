use download::{self, DownloadManager, Downloadable};
use forge_version;
use futures::{self, Future};

use maven::{MavenArtifact, ResolvedArtifact};
use slog::Logger;
use std::path::PathBuf;
use url::{self, Url};
use curseforge;
use cache::Cache;

#[derive(Serialize, Deserialize, Debug,Clone)]
pub enum ModSource {
    CurseforgeMod (curseforge::Mod),
    MavenMod{
        repo: String,
        artifact: MavenArtifact,
    },
}

impl Downloadable for ModSource {
    fn download(self,
                location: PathBuf,
                manager: DownloadManager,
                log: Logger)
                -> download::BoxFuture<()> {
        match self {
            ModSource::CurseforgeMod ( modd ) => {
                curseforge::Cache::install_at(modd,location,manager,log)
            }
            ModSource::MavenMod { repo, artifact } => {
                futures::lazy(move || {
                        Url::from_str(repo.as_str()).map_err(download::Error::from)
                    })
                    .and_then(move |repo| {
                        artifact.download_from(location.as_ref(), repo, manager, log)
                    })
                    .boxed()
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum OS {
    #[serde(rename="osx")]
    X,
    #[serde(rename="windows")]
    Windows,
    #[serde(rename="linux")]
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
    #[serde(rename="allow")]
    Allow,
    #[serde(rename="disallow")]
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

const MC_LIBS_MAVEN: &'static str = "https://libraries.minecraft.net/";

impl Downloadable for MCLibraryListing {
    fn download(self,
                mut location: PathBuf,
                manager: DownloadManager,
                log: Logger)
                -> download::BoxFuture<()> {
        futures::lazy(move || {
                if !self.is_native() {
                    let artifact = self.name.parse::<MavenArtifact>().unwrap();
                    let base = Url::from_str(MC_LIBS_MAVEN)?;
                    Ok(Some(artifact.resolve(base)))
                } else {
                    let mut artifact = self.name.parse::<MavenArtifact>().unwrap();
                    let disallowed = if let Some(ref rules) = self.rules {
                        rules.into_iter()
                            .any(|rule| rule.os_matches() && rule.action == Action::Disallow)
                    } else {
                        false
                    };
                    if disallowed {
                        Ok(None)//nothing to download
                    } else {
                        artifact.classifier = self.platform_native_classifier();
                        let base = Url::from_str(MC_LIBS_MAVEN)?;
                        Ok(Some(artifact.resolve(base)))
                    }
                }
            })
            .and_then(move |resolved_artifact| {
                if let Some(resolved_artifact) = resolved_artifact {
                    location.push(resolved_artifact.to_path());
                    resolved_artifact.download(location, manager, log)
                } else {
                    futures::finished(()).boxed()
                }
            })
            .boxed()
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MCVersionInfo {
    pub id: String,
    pub time: String,
    #[serde(rename="releaseTime")]
    pub release_time: String,
    #[serde(rename="type")]
    pub type_: String,
    #[serde(rename="minecraftArguments")]
    pub minecraft_arguments: String,
    pub libraries: Vec<MCLibraryListing>,
    #[serde(rename="mainClass")]
    pub main_class: String,
    #[serde(rename="minimumLauncherVersion")]
    pub minimum_launcher_version: i64,
    pub assets: String,
}

use hyper;
use serde_json;
use std::str::FromStr;
use std::string::String;

pub type ModList = Vec<ModSource>;

impl MCVersionInfo {
    pub fn version(ver: &str) -> serde_json::Result<MCVersionInfo> {
        let client = hyper::Client::new();
        let url = Url::from_str(format!("http://s3.amazonaws.com/Minecraft.\
                                         Download/versions/{0}/{0}.json",
                                        ver)
                .as_str())
            .unwrap();
        let res = client.get(url).send().unwrap();
        serde_json::de::from_reader(res)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ModpackConfig {
    pub version: String,
    pub name: String,
    pub forge: String,
    pub mods: ModList,
}

impl ModpackConfig {
    pub fn folder(&self) -> String {
        self.name.replace(|c: char| !c.is_alphanumeric(), "_")
    }
    pub fn forge_maven_artifact(&self) -> Result<ResolvedArtifact, url::ParseError> {
        Ok(MavenArtifact {
                group: "net.minecraftforge".into(),
                artifact: "forge".into(),
                version: self.forge.clone(),
                classifier: Some("universal".into()),
                extension: Some("jar".into()),
            }
            .resolve(Url::from_str(forge_version::BASE_URL)?))
    }
}
