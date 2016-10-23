#![feature(custom_derive, plugin,custom_attribute,slice_patterns)]
#![deny(clippy)]

extern crate modpack_tool;

extern crate serde;
extern crate serde_json;
extern crate url;
extern crate sha1;
#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate slog;
extern crate slog_stdlog;
extern crate slog_term;
extern crate slog_stream;
extern crate slog_json;
extern crate zip;
extern crate futures;

use futures::Future;

use modpack_tool::{Result, BoxFuture};
use modpack_tool::download::{Downloadable, DownloadManager};
use modpack_tool::hacks;
use modpack_tool::maven;
use modpack_tool::types::*;

use slog::{DrainExt, Logger};

use std::fs;
use std::io::Read;
use std::path::PathBuf;

fn load_pack<R>(reader: R) -> serde_json::Result<ModpackConfig>
    where R: Read
{
    serde_json::de::from_reader(reader)
}

fn add_launcher_profile(pack_path: PathBuf,
                        pack_name: String,
                        version_id: &VersionId,
                        _log: Logger)
                        -> Result<()> {
    use std::collections::btree_map::Entry;
    use serde_json::value::{ToJson, Value};
    use serde_json::value::Map;


    let pack_path = pack_path.canonicalize()?;
    let pack_path = pack_path.to_str().unwrap();
    // FIXME move this to a function switched out for various platforms
    let pack_path = pack_path.trim_left_matches(r#"\\?\"#);//de UNC prefix path, because apparently java can't handle it

    let mut mc_path = mc_install_loc();
    mc_path.push("launcher_profiles.json");
    let profiles_file = std::fs::File::open(&mc_path)?;
    let mut launcher_profiles: Value = serde_json::from_reader(profiles_file)?;

    {
        let profiles: &mut Map<String, Value> = launcher_profiles.as_object_mut()
            .expect("profiles json was not an object")
            .get_mut("profiles")
            .expect("profiles object missing")
            .as_object_mut()
            .expect("profiles object was not an object");

        match profiles.entry(pack_name.clone()) {
            Entry::Occupied(mut entry) => {
                let mut obj =
                    entry.get_mut().as_object_mut().expect("profile entry was not an object");
                obj.insert("name".to_string(), pack_name.to_json());
                obj.insert("gameDir".to_string(), pack_path.to_json());
                obj.insert("lastVersionId".to_string(), version_id.0.to_json());
            }
            Entry::Vacant(entry) => {
                let mut obj: Map<&str, Value> = Map::new();
                // bump the memory higher than the mojang default if this is our initial creation
                obj.insert("javaArgs", "-Xms2G -Xmx2G".to_json());
                obj.insert("name", pack_name.to_json());
                obj.insert("gameDir", pack_path.to_json());
                obj.insert("lastVersionId", version_id.0.to_json());
                entry.insert(obj.to_json());
            }
        }
    }

    let mut profiles_file = std::fs::File::create(mc_path)?;
    serde_json::to_writer_pretty(&mut profiles_file, &launcher_profiles)?;

    Ok(())
}

fn download_modlist(mut pack_path: PathBuf,
                    mod_list: ModList,
                    manager: DownloadManager,
                    log: Logger)
                    -> BoxFuture<()> {
    let log = log.new(o!("stage"=>"download_modlist"));

    pack_path.push("mods");
    futures::lazy({
            let pack_path = pack_path.clone();
            move || {
                std::fs::create_dir_all(&pack_path)?;

                for entry in std::fs::read_dir(&pack_path)? {
                    let entry = entry?;
                    std::fs::remove_file(&entry.path())?;
                }
                Ok(())
            }
        })
        .and_then(|_| mod_list.download(pack_path, manager, log).map_err(Into::into))
        .boxed()
}

fn mc_install_loc() -> PathBuf {
    // FIXME this isn't how minecraft handles install location on non-windows platforms
    let mut mc_path = PathBuf::from(std::env::var("APPDATA")
        .expect("Your windows install is fucked"));
    mc_path.push(".minecraft");
    mc_path
}

struct VersionId(pub String);

fn install_forge(mut pack_path: PathBuf,
                 forge_artifact: maven::ResolvedArtifact,
                 manager: DownloadManager,
                 log: Logger)
                 -> BoxFuture<VersionId> {
    use serde_json::Value;

    let log = log.new(o!("stage"=>"install_forge"));

    pack_path.push("forge");
    futures::lazy(move || {
            std::fs::create_dir_all(&pack_path)?;
            Ok(())
        })
        .and_then(move |_| {
            let forge_maven_artifact_path = forge_artifact.to_path();
            forge_artifact.reader(manager.clone(), log.clone())
                .map_err(modpack_tool::Error::from)
                .and_then(move |reader| {
                    futures::lazy({
                        let log = log.clone();
                        move || {
                            debug!(log,"Opening forge jar");
                            let mut zip_reader = zip::ZipArchive::new(reader)?;
                            let version_id: String = {
                                debug!(log,"Reading version json");
                                let version_reader = zip_reader.by_name("version.json")?;
                                let version_info: Value = serde_json::from_reader(version_reader)?;
                                version_info.find("id")
                                    .expect("bad version.json")
                                    .as_str()
                                    .expect("bad version.json id value")
                                    .into()
                            };

                            let mut mc_path = mc_install_loc();
                            mc_path.push("versions");
                            mc_path.push(version_id.as_str());
                            debug!(log,"creating profile folder");
                            fs::create_dir_all(mc_path.clone())?;
                            
                            mc_path.push(format!("{}.json", version_id.as_str()));
                            
                            debug!(log,"saving version json to minecraft install loc");
                            let mut version_file = std::fs::File::create(&mc_path)?;
                            std::io::copy(&mut zip_reader.by_name("version.json")?,
                                          &mut version_file)?;
                                          
                            debug!(log,"Applying version json hacks");
                            hacks::hack_forge_version_json(mc_path)?;

                            let mut mc_path = mc_install_loc();
                            mc_path.push("libraries");
                            mc_path.push(forge_maven_artifact_path);
                            mc_path.pop();//pop the filename
                            Ok((version_id, mc_path))
                        }})
                        .and_then(move |(version_id, mc_path)| {
                            forge_artifact.install_at_no_classifier(&mc_path, manager, log)
                                .map_err(Into::into)
                                .map(move |_| VersionId(version_id))
                        })
                })
        })
        .boxed()
}

fn main() {
    let path = std::env::args().nth(1).expect("pass pack as first argument");

    let log_path = path.clone() + ".log";

    let log_file = std::fs::File::create(log_path).expect("Couldn't open log file");
    let log_file_stream = slog_stream::stream(log_file, slog_json::default());


    let root = Logger::root(slog::duplicate(slog::LevelFilter::new(slog_term::streamer()
                                                                       .compact()
                                                                       .build(),
                                                                   slog::Level::Debug),
                                            log_file_stream)
                                .fuse(),
                            o!());
    let log = root.new(o!());
    let download_manager = DownloadManager::new();

    // slog_stdlog::set_logger(log.new(o!())).unwrap();

    info!(log, "loading pack config");

    let modpack = match std::fs::File::open(&path) {
        Ok(file) => load_pack(file),
        Err(_) => panic!("Couldn't open {:?}", &path),
    };
    let mut pack_path = PathBuf::from(".");
    match modpack {
            Err(e) => panic!(e),
            Ok(pack) => {
                let forge_artifact = pack.forge_maven_artifact();
                forge_artifact.map_err(Into::into).and_then(move |forge_maven_artifact| {
                    pack_path.push(pack.folder());
                    let ModpackConfig { name: pack_name, mods, .. } = pack;
                    install_forge(pack_path.clone(),
                                  forge_maven_artifact,
                                  download_manager.clone(),
                                  log.clone())
                        .and_then(move |id| {
                            download_modlist(pack_path.clone(),
                                             mods,
                                             download_manager.clone(),
                                             log.clone())
                                .and_then(move |_| {
                                    add_launcher_profile(pack_path, pack_name, &id, log.clone())
                                })
                        })
                        .wait()
                })
            }
        }
        .unwrap();
    info!(root, "done");
}
