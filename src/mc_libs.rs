use http::Uri;
use crate::{
    download::{self,Downloadable},
    maven,
};
use std::{
    io::Cursor,
    path::PathBuf,
};
use slog::Logger;
use futures::prelude::*;

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
    fn download(
        self,
        mut location: PathBuf,
        manager: download::Manager,
        log: Logger,
    ) -> download::BoxFuture<()> {
        Box::pin(async move{
            let resolved_artifact = if self.is_native() {
                let mut artifact = self.name.parse::<maven::Artifact>().unwrap();
                let disallowed = if let Some(ref rules) = self.rules {
                    rules
                        .iter()
                        .any(|rule| rule.os_matches() && rule.action == Action::Disallow)
                } else {
                    false
                };
                if disallowed {
                    None //nothing to download
                } else {
                    artifact.classifier = self.platform_native_classifier();
                    let base = Uri::from_str(MC_LIBS_MAVEN).context(crate::download::error::Uri)?;
                    Some(artifact.resolve(base))
                }
            } else {
                let artifact = self.name.parse::<maven::Artifact>().unwrap();
                let base = Uri::from_str(MC_LIBS_MAVEN).context(crate::download::error::Uri)?;
                Some(artifact.resolve(base))
            };
            if let Some(resolved_artifact) = resolved_artifact {
                location.push(resolved_artifact.to_path());
                resolved_artifact.download(location, manager, log).await?;
            }
            Ok(())
        })
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
use crate::error;
use snafu::ResultExt;

impl MCVersionInfo {
    pub fn version(ver: &str) -> impl Future<Output=crate::Result<Self>> {
        let client = hyper::Client::new();
        let uri = Uri::from_str(
            format!(
                "http://s3.amazonaws.com/Minecraft.\
                 Download/versions/{0}/{0}.json",
                ver
            ).as_str(),
        ).unwrap();
        async move{
            let res = client.get(uri).await.context(error::Http)?;
            let buf = res.into_body().map_ok(hyper::Chunk::into_bytes).try_concat().await.context(error::Http)?;
            let info: Self = serde_json::de::from_reader(Cursor::new(buf)).context(error::Json)?;
            Ok(info)
        }
    }
}