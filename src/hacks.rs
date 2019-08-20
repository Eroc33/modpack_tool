// FIXME Forge devs refuse to actually match the spec

use crate::{
    error::prelude::*,
    maven,
};
use snafu::Snafu;
use serde_json::{self, Value};
use std;
use std::path::Path;

const HACK_REQUIRED: &[& str] = &["com.typesafe.akka", "com.typesafe"];

const HACK_REPO_REDIRECT: & str = "https://repo1.maven.org/maven2/";

#[derive(Debug,Snafu)]
pub enum Error{
    #[snafu(display("Forge version json couldn't be opened due to: {}", source))]
    ForgeVersionJsonMissing{
        source: std::io::Error,
    },
    #[snafu(display("Forge version json was in a bad format: {}", source))]
    BadForgeVersionJsonFormat{
        source: serde_json::Error,
    },
    #[snafu(display("Failed to write forge version json at path {} due to {}", path, source))]
    WritingForgeVersionJson{
        path: String,
        source: serde_json::Error,
    }
}


//TODO: use more specific "version json" error here?
pub fn hack_forge_version_json<P>(path: P) -> Result<(),Error>
    where P: AsRef<Path>
{
    let path = path.as_ref();
    let version_file = std::fs::File::open(path).context(ForgeVersionJsonMissing)?;
    let mut version: Value = serde_json::from_reader(version_file).context(BadForgeVersionJsonFormat)?;

    {
        let libraries: &mut Vec<Value> = version.as_object_mut()
            .expect("version json was not a map")
            .get_mut("libraries")
            .expect("libraries object missing")
            .as_array_mut()
            .expect("libraries object was not an array");

        for library in libraries {
            let library = library.as_object_mut().expect("library object not a map");
            let artifact: maven::Artifact = library.get("name")
                .expect("no library name")
                .as_str()
                .expect("library name was not a string")
                .parse()
                .expect("library name was not a maven identifier");
            if HACK_REQUIRED.contains(&artifact.group.as_str()) {
                library.insert("url".to_string(), serde_json::to_value(HACK_REPO_REDIRECT).expect("Couldn't convert pre-checked const to json value"));
            }
        }
    }

    let mut version_file = std::fs::File::create(path).context(ForgeVersionJsonMissing)?;
    serde_json::to_writer_pretty(&mut version_file, &version).context(WritingForgeVersionJson{path: path.display().to_string()})?;
    Ok(())
}
