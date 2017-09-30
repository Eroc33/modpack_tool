#![feature(proc_macro,custom_derive, plugin,slice_patterns,generators)]
#![deny(clippy)]

#[macro_use]
extern crate scan_rules;
#[macro_use]
extern crate modpack_tool;
extern crate tokio_core;
#[macro_use]
extern crate serde_json;
extern crate clap;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_json;
extern crate zip;
extern crate futures_await as futures;

use futures::prelude::*;
use modpack_tool::{Result, BoxFuture};
use modpack_tool::download::{Downloadable, DownloadManager};
use modpack_tool::hacks;
use modpack_tool::maven;
use modpack_tool::types::*;

use slog::{Logger, Drain};

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio_core::reactor::Core;

fn load_pack<R>(reader: R) -> serde_json::Result<ModpackConfig>
    where R: Read
{
    serde_json::de::from_reader(reader)
}

fn add_launcher_profile(pack_path: &PathBuf,
                        pack_name: String,
                        version_id: &VersionId,
                        log: &Logger)
                        -> Result<()> {
    use serde_json::value::Value;

    let pack_path = pack_path.canonicalize()?;
    let pack_path = pack_path.to_str().unwrap();
    // FIXME move this to a function switched out for various platforms
    let pack_path = pack_path.trim_left_matches(r#"\\?\"#); //de UNC prefix path, because apparently java can't handle it

    let mut mc_path = mc_install_loc();
    mc_path.push("launcher_profiles.json");
    let profiles_file = std::fs::File::open(&mc_path)?;
    let mut launcher_profiles: Value = serde_json::from_reader(profiles_file)?;

    {
        use serde_json::map::Entry;

        let profiles = launcher_profiles.pointer_mut("/profiles")
            .expect("profiles key is missing")
            .as_object_mut()
            .expect("profiles is not an object");

        debug!(log,"Read profiles key"; "profiles"=> ?profiles, "key"=>pack_name.as_str());

        match profiles.entry(pack_name.as_str()) {
            Entry::Occupied(mut occupied) => {
                let profile = occupied.get_mut()
                    .as_object_mut()
                    .expect("Profile value was not an object");
                profile.insert("name".to_string(), serde_json::to_value(pack_name).unwrap());
                profile.insert("gameDir".to_string(),
                               serde_json::to_value(pack_path).unwrap());
                profile.insert("lastVersionId".to_string(),
                               serde_json::to_value(version_id.0.clone()).unwrap());
            }
            Entry::Vacant(empty) => {
                // bump the memory higher than the mojang default if this is our initial creation
                empty.insert(json!({
                    "javaArgs": "-Xms2G -Xmx2G",
                    "name": pack_name,
                    "gameDir": pack_path,
                    "lastVersionId": version_id.0
                }));
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
                    log: &Logger)
                    -> modpack_tool::BoxFuture<()> {
    let log = log.new(o!("stage"=>"download_modlist"));

    Box::new(async_block!{
        pack_path.push("mods");
        let pack_path = pack_path.clone();
        std::fs::create_dir_all(&pack_path)?;

        for entry in std::fs::read_dir(&pack_path)? {
            let entry = entry?;
            std::fs::remove_file(&entry.path())?;
        }
        Ok(await!(mod_list.download(pack_path, manager, log))?)
    })
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
                 log: &Logger)
                 -> BoxFuture<VersionId> {
    use serde_json::Value;

    let log = log.new(o!("stage"=>"install_forge"));
    pack_path.push("forge");

    Box::new(async_block!{
        std::fs::create_dir_all(&pack_path)?;
        
        let forge_maven_artifact_path = forge_artifact.to_path();
        let reader = await!(forge_artifact.reader(manager.clone(), log.clone()))?;

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
        fs::create_dir_all(mc_path.clone())?;

        mc_path.push(format!("{}.json", version_id.as_str()));

        debug!(log, "saving version json to minecraft install loc");
        let mut version_file = std::fs::File::create(&mc_path)?;
        std::io::copy(&mut zip_reader.by_name("version.json")?,
                        &mut version_file)?;

        debug!(log, "Applying version json hacks");
        hacks::hack_forge_version_json(mc_path)?;

        let mut mc_path = mc_install_loc();
        mc_path.push("libraries");
        mc_path.push(forge_maven_artifact_path);
        mc_path.pop(); //pop the filename
        
        await!(forge_artifact.install_at_no_classifier(&mc_path, manager, log))?;
        Ok(VersionId(version_id))
    })
}

fn add(pack_path: &str, mod_url: &str){
    use modpack_tool::curseforge;
    use modpack_tool::types::{ModSource, ModpackConfig};
    use scan_rules::input::IntoScanCursor;

    use scan_rules::scanner::Everything;
    use scan_rules::scanner::runtime::until_pat_a;

    let mod_url = mod_url.into_scan_cursor();

    let file = std::fs::File::open(&pack_path).expect("pack does not exist");
    let mut pack: ModpackConfig = serde_json::de::from_reader(file)
        .expect("pack file in bad format");

    let modsource: ModSource = scan!{mod_url;
        ("https://minecraft.curseforge.com/projects/",let project <| until_pat_a::<Everything<String>,&str>("/"),"/files/",let ver, ["/download"]?) => curseforge::Mod{id:project,version:ver}.into(),
    }.expect("Unknown modsource url");

    pack.replace_mod(modsource);

    let mut file = std::fs::File::create(pack_path).expect("pack does not exist");
    serde_json::ser::to_writer_pretty(&mut file, &pack).unwrap();
}

fn update(path: &str, log: Logger){
    let mut core = Core::new().expect("Couldn't initialise tokio");
    let handle = core.handle();

    let download_manager = DownloadManager::new(&handle);

    // slog_stdlog::set_logger(log.new(o!())).unwrap();

    info!(log, "loading pack config");
    let post_log = log.clone();

    let modpack = match std::fs::File::open(&path) {
        Ok(file) => load_pack(file),
        Err(_) => die!("Couldn't open {:?}", &path),
    };
    let mut pack_path = PathBuf::from(".");
    let run_fn = match modpack {
        Err(e) => die!("File error: {:?}",e),
        Ok(pack) => async_block!{
            let forge_maven_artifact = pack.forge_maven_artifact()?;
            pack_path.push(pack.folder());
            let ModpackConfig { name: pack_name, mods, .. } = pack;
            let (id, _) = await!(install_forge(pack_path.clone(),
                            forge_maven_artifact,
                            download_manager.clone(),
                            &log)
                .join(download_modlist(pack_path.clone(), mods, download_manager.clone(), &log)))?;
            return add_launcher_profile(&pack_path, pack_name, &id, &log);
        },
    };
    core.run(run_fn).unwrap();
    info!(post_log, "done");
}

fn build_cli() -> clap::App<'static,'static>{
    clap::App::new("modpacktool-update")
        .version("0.1")
        .author("E. Rochester <euan@rochester.me.uk>")
        .subcommands(vec![
            clap::SubCommand::with_name("update")
            .visible_alias("install")
            .about("Updates the on-disk mods from the provided pack file")
            .arg(clap::Arg::with_name("pack_file")
                .required(true)
                .index(1)
                .help("The metadata json file for the pack you wish to update"))
            ,
            clap::SubCommand::with_name("add_mod")
            .about("Adds a mod to the provided pack file")
            .arg(clap::Arg::with_name("pack_file")
                .required(true)
                .index(1)
                .help("The metadata json file for the pack you wish to modify"))
            .arg(clap::Arg::with_name("mod_url")
                .required(true)
                .index(2)
                .help("The url for the mod you wish to add"))
            ,
            clap::SubCommand::with_name("try_upgrade")
            .about("Checks if upgrade compatibility for this pack from one minecraft version to the next.")
            .arg(clap::Arg::with_name("pack_file")
                .required(true)
                .index(1)
                .help("The metadata json file for the pack you wish to modify"))
            .arg(clap::Arg::with_name("mc_version")
                .required(true)
                .index(2)
                .help("The minecraft version to upgrade to"))
        ])
}

fn main() {
    let matches = build_cli().get_matches();

    match matches.subcommand(){
        ("update",Some(args)) => {
            let pack_path = args.value_of("pack_file").expect("pack_file is required!");

            let log_path = pack_path.to_owned() + ".log";

            let log_file = std::fs::File::create(log_path).expect("Couldn't open log file");
            let log_file_stream = slog_json::Json::default(log_file);


            let root = Logger::root(Arc::new(Mutex::new(slog::Duplicate::new(slog::LevelFilter::new(slog_term::term_compact(),
                                                                        slog::Level::Debug),
                                                    log_file_stream)
                                        .fuse())).ignore_res(),
                                    o!());
            let log = root.new(o!());

            update(pack_path,log);
        }
        ("add",Some(args)) => {
            let pack_path = args.value_of("pack_file").expect("pack_file is required!");
            let mod_url = args.value_of("mod_url").expect("mod_url is required!");

            add(pack_path,mod_url);
        }
        ("try_upgrade",Some(args)) => {
            
        }
        _ => {
            build_cli().print_help().expect("Failed to print help. Is the terminal broken?");
        }
    }
}
