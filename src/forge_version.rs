use hyper;
use serde_json;
use std::collections::HashMap;

pub const BASE_URL: &'static str = "http://files.minecraftforge.net/maven/";
pub const JSON_URL: &'static str = "http://files.minecraftforge.\
                                    net/maven/net/minecraftforge/forge/json/";

#[derive(Serialize, Deserialize, Debug)]
pub struct ForgeVersionList {
    adfocus: String,
    artifact: String,
    branches: HashMap<String, u64>,
    homepage: String,
    mcversion: HashMap<String, Vec<u64>>,
    name: String,
    number: HashMap<u64, Branch>,
    promos: HashMap<String, u64>,
    webpath: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Branch {
    branch: Option<String>,
    build: u64,
    files: Vec<File>,
    mcversion: String,
    modified: f64,
    version: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct File(String, String, String);

impl File {
    pub fn typ(&self) -> &str {
        &self.0
    }

    pub fn role(&self) -> &str {
        &self.1
    }

    pub fn hash(&self) -> &str {
        &self.2
    }
}

pub fn get_version_list() -> serde_json::Result<ForgeVersionList> {
    let data = hyper::Client::new().get(JSON_URL).send().unwrap();
    serde_json::de::from_reader(data)
}
