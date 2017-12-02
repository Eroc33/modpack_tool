// FIXME Forge devs refuse to actually match the spec

use maven::MavenArtifact;
use serde_json::{self, Value};
use std;
use std::path::Path;

const HACK_REQUIRED: &[& str] = &["com.typesafe.akka", "com.typesafe"];

const HACK_REPO_REDIRECT: & str = "https://repo1.maven.org/maven2/";

pub fn hack_forge_version_json<P>(path: P) -> ::Result<()>
    where P: AsRef<Path>
{
    let version_file = std::fs::File::open(path.as_ref())?;
    let mut version: Value = serde_json::from_reader(version_file)?;

    {
        let libraries: &mut Vec<Value> = version.as_object_mut()
            .expect("version json was not a map")
            .get_mut("libraries")
            .expect("libraries object missing")
            .as_array_mut()
            .expect("libraries object was not an array");

        for library in libraries {
            let library = library.as_object_mut().expect("library object not a map");
            let artifact: MavenArtifact = library.get("name")
                .expect("no library name")
                .as_str()
                .expect("library name was not a string")
                .parse()
                .expect("library name was not a maven identifier");
            if HACK_REQUIRED.contains(&artifact.group.as_str()) {
                library.insert("url".to_string(), serde_json::to_value(HACK_REPO_REDIRECT)?);
            }
        }
    }

    let mut version_file = std::fs::File::create(path.as_ref())?;
    serde_json::to_writer_pretty(&mut version_file, &version)?;
    Ok(())
}
