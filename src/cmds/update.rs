use futures::prelude::*;
use slog::Logger;
use std::path::PathBuf;
use serde_json::{self, Value};
use tokio;
use std;
use zip;
use failure::*;

use {BoxFuture, Result};
use download::{DownloadManager, Downloadable};
use hacks;
use maven;
use types::*;
use cache::Cacheable;
use util;
use fs_futures;

pub fn update(path: String, log: Logger) -> BoxFuture<()> {
    let download_manager = DownloadManager::new();

    info!(log, "loading pack config");
    Box::new(async_block!{
        let file = std::fs::File::open(path.clone()).context(format!("{} is not a file",&path))?;
        let pack = ModpackConfig::load(file).context(format!("{} is not a valid modpack config",&path))?;
        let mut pack_path = PathBuf::from(".");
        let forge_maven_artifact = pack.forge_maven_artifact()?;
        pack_path.push(pack.folder());
        let ModpackConfig { name: pack_name, mods, .. } = pack;

        let joint_task: Box<Future<Item=(VersionId,()),Error=::Error>+Send> = Box::new(
                install_forge(pack_path.clone(),
                        forge_maven_artifact,
                        download_manager.clone(),
                        &log)
                .join(download_modlist(pack_path.clone(), mods, download_manager.clone(), &log))
            );

        let (id, _) = await!(joint_task)?;
        await!(add_launcher_profile(&pack_path, pack_name, id, &log)?)?;
        info!(log,"Done");
        Ok(())
    })
}

fn merge(a: &mut Value, b: &Value) {
    match (a, b) {
        (&mut Value::Object(ref mut a), &Value::Object(ref b)) => for (k, v) in b {
            merge(a.entry(k.clone()).or_insert(Value::Null), v);
        },
        (a, b) => {
            *a = b.clone();
        }
    }
}

fn add_launcher_profile(
    pack_path: &PathBuf,
    pack_name: String,
    version_id: VersionId,
    _log: &Logger,
) -> Result<BoxFuture<()>> {
    use serde_json::value::Value;

    //de UNC prefix path, because apparently java can't handle it
    let pack_path = pack_path.canonicalize()?;
    let pack_path = util::remove_unc_prefix(pack_path);

    let mut mc_path = mc_install_loc();
    mc_path.push("launcher_profiles.json");

    Ok(super::replace(mc_path, |profiles_file| {
        async_block!{
            let mut launcher_profiles: Value = serde_json::from_reader(profiles_file)?;

            {
                use serde_json::map::Entry;

                let profiles = launcher_profiles
                    .pointer_mut("/profiles")
                    .expect("profiles key is missing")
                    .as_object_mut()
                    .expect("profiles is not an object");

                //debug!(log,"Read profiles key"; "profiles"=> ?profiles, "key"=>pack_name.as_str());

                let always_set = json!({
                                "name": pack_name,
                                "gameDir": pack_path,
                                "lastVersionId": version_id.0
                            });

                match profiles.entry(pack_name.as_str()) {
                    Entry::Occupied(mut occupied) => {
                        let profile = occupied.get_mut();
                        merge(profile, &always_set);
                    }
                    Entry::Vacant(empty) => {
                        // bump the memory higher than the mojang default if this is our initial creation
                        let mut to_set = json!({
                            "javaArgs": "-Xms2G -Xmx2G"
                        });
                        merge(&mut to_set, &always_set);
                        empty.insert(to_set);
                    }
                }
            }

            Ok(std::io::Cursor::new(serde_json::to_vec_pretty(&launcher_profiles)?))
        }
    }))
}

fn download_modlist(
    mut pack_path: PathBuf,
    mod_list: ModList,
    manager: DownloadManager,
    log: &Logger,
) -> BoxFuture<()> {
    let log = log.new(o!("stage"=>"download_modlist"));

    Box::new(async_block!{
        pack_path.push("mods");
        await!(fs_futures::create_dir_all(pack_path.clone()))?;

        #[async]
        for entry in fs_futures::read_dir(pack_path.clone())? {
            await!(fs_futures::remove_file(entry.path().clone()))?;
        }
        await!(mod_list.download(pack_path, manager, log))?;
        Ok(())
    })
}

fn mc_install_loc() -> PathBuf {
    // FIXME this isn't how minecraft handles install location on non-windows platforms
    let mut mc_path =
        PathBuf::from(std::env::var("APPDATA").expect("Your windows install is fucked"));
    mc_path.push(".minecraft");
    mc_path
}

struct VersionId(pub String);

fn install_forge(
    mut pack_path: PathBuf,
    forge_artifact: maven::ResolvedArtifact,
    manager: DownloadManager,
    log: &Logger,
) -> BoxFuture<VersionId> {
    use serde_json::Value;

    let log = log.new(o!("stage"=>"install_forge"));
    pack_path.push("forge");

    Box::new(async_block!{
        trace!(log,"Creating pack folder");
        await!(fs_futures::create_dir_all(pack_path.clone()))?;
        trace!(log,"Created pack folder");

        let forge_maven_artifact_path = forge_artifact.to_path();
        let reader = await!(forge_artifact.clone().reader(manager.clone(), log.clone()))?;

        debug!(log, "Opening forge jar");
        let mut zip_reader = zip::ZipArchive::new(reader)?;
        let version_id: String = {
            debug!(log, "Reading version json");
            let version_reader = zip_reader.by_name("version.json")?;
            let version_info: Value =
                serde_json::from_reader(version_reader)?;
            version_info["id"]
                .as_str()
                .expect("bad version.json id value")
                .into()
        };

        let mut mc_path = mc_install_loc();
        mc_path.push("versions");
        mc_path.push(version_id.as_str());
        debug!(log, "creating profile folder");
        await!(fs_futures::create_dir_all(mc_path.clone()))?;

        mc_path.push(format!("{}.json", version_id.as_str()));

        debug!(log, "saving version json to minecraft install loc");

        let mut version_file = await!(tokio::fs::File::create(mc_path.clone()))?;
        //TODO: figure out how to use tokio copy here
        //note zip_reader.by_name() returns a ZipFile and ZipFile: !Send
        std::io::copy(&mut zip_reader.by_name("version.json")?,
                        &mut version_file)?;

        debug!(log, "Applying version json hacks");
        hacks::hack_forge_version_json(mc_path)?;

        let mut mc_path = mc_install_loc();
        mc_path.push("libraries");
        mc_path.push(forge_maven_artifact_path);
        mc_path.pop(); //pop the filename

        await!(forge_artifact.install_at_no_classifier(mc_path, manager, log))?;
        Ok(VersionId(version_id))
    })
}
