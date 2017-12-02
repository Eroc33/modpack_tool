#![feature(proc_macro,custom_derive, plugin,slice_patterns,generators, conservative_impl_trait)]

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
extern crate semver;
#[macro_use]
extern crate error_chain;
extern crate env_logger;

use futures::prelude::*;
use modpack_tool::{Result, BoxFuture};
use modpack_tool::download::{Downloadable, DownloadManager};
use modpack_tool::hacks;
use modpack_tool::maven;
use modpack_tool::upgrade;
use modpack_tool::types::*;

use slog::{Logger, Drain};

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio_core::reactor::{self,Core};

fn load_pack<R>(reader: R) -> serde_json::Result<ModpackConfig>
    where R: Read
{
    serde_json::de::from_reader(reader)
}

fn add_launcher_profile(pack_path: &PathBuf,
                        pack_name: String,
                        version_id: &VersionId,
                        _log: &Logger)
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

        //debug!(log,"Read profiles key"; "profiles"=> ?profiles, "key"=>pack_name.as_str());

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
        trace!(log,"Creating pack folder");
        std::fs::create_dir_all(&pack_path)?;
        trace!(log,"Created pack folder");
        
        let forge_maven_artifact_path = forge_artifact.to_path();
        let reader = await!(forge_artifact.reader(&manager, &log))?;

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
        
        await!(forge_artifact.install_at_no_classifier(&mc_path, &manager, log))?;
        Ok(VersionId(version_id))
    })
}

fn add(pack_path: &str, mod_url: &str){
    use modpack_tool::types::ModpackConfig;

    let file = std::fs::File::open(&pack_path).expect("pack does not exist");
    let mut pack: ModpackConfig = serde_json::de::from_reader(file)
        .expect("pack file in bad format");
		
	pack.add_mod_by_url(mod_url).expect("Unparseable modsource url");

    let mut file = std::fs::File::create(pack_path).expect("pack does not exist");
    pack.save(&mut file).unwrap();
}

fn update(path: &str, log: Logger, handle: &reactor::Handle) -> impl Future<Item=(),Error=modpack_tool::Error> +'static{

    let download_manager = DownloadManager::new(handle);

    // slog_stdlog::set_logger(log.new(o!())).unwrap();

    info!(log, "loading pack config");
    let post_log = log.clone();
    let path = path.to_owned();
    async_block!{
        let file = std::fs::File::open(path)?;
        let modpack = load_pack(file); 
        let mut pack_path = PathBuf::from(".");
        let pack = modpack?;
        let forge_maven_artifact = pack.forge_maven_artifact()?;
        pack_path.push(pack.folder());
        let ModpackConfig { name: pack_name, mods, .. } = pack;
        let (id, _) = await!(install_forge(pack_path.clone(),
                        forge_maven_artifact,
                        download_manager.clone(),
                        &log)
            .join(download_modlist(pack_path.clone(), mods, download_manager.clone(), &log)))?;
        add_launcher_profile(&pack_path, pack_name, &id, &log)?;
        info!(post_log,"Done");
        Ok(())
    }
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
            clap::SubCommand::with_name("dev")
            .about("Commands for modpack developers")
            .subcommands(vec![
                clap::SubCommand::with_name("try_upgrade")
                .about("Checks upgrade compatibility for this pack from one minecraft version to the next.")
                .arg(clap::Arg::with_name("pack_file")
                    .required(true)
                    .index(1)
                    .help("The metadata json file for the pack you wish to modify"))
                .arg(clap::Arg::with_name("mc_version")
                    .required(true)
                    .index(2)
                    .help("The minecraft version to upgrade to"))
                ,
                clap::SubCommand::with_name("do_upgrade")
                .about("Upgrades this pack from one minecraft version to the next.")
                .arg(clap::Arg::with_name("pack_file")
                    .required(true)
                    .index(1)
                    .help("The metadata json file for the pack you wish to modify"))
                .arg(clap::Arg::with_name("mc_version")
                    .required(true)
                    .index(2)
                    .help("The minecraft version to upgrade to"))
                ,
                clap::SubCommand::with_name("add")
                .about("Adds a mod to the provided pack file")
                .arg(clap::Arg::with_name("pack_file")
                    .required(true)
                    .index(1)
                    .help("The metadata json file for the pack you wish to modify"))
                .arg(clap::Arg::with_name("mod_url")
                    .required(true)
                    .index(2)
                    .help("The url for the mod you wish to add"))
            ])
        ])
}

macro_rules! report_error{
    ($($items:expr),+) => {{
        report_error(format!($($items),+))
    }}
}

fn report_error<S: Into<String>>(s: S) -> modpack_tool::Error{
    modpack_tool::ErrorKind::ReportError(s.into()).into()
}

quick_main!(run);

fn run() -> Result<i32> {
    env_logger::init().expect("Logger setup failure");
    let matches = build_cli().get_matches();
    let mut core = Core::new().expect("Failed to start tokio");

    let log_path = "modpack_tool.log";

    let log_file = std::fs::File::create(log_path).expect("Couldn't open log file");
    let log_file_stream = slog_json::Json::default(log_file);


    let root = Logger::root(Arc::new(Mutex::new(slog::Duplicate::new(slog_term::term_compact(),
                                            log_file_stream)
                                .fuse())).ignore_res(),
                            o!());
    let log = root.new(o!());

    let run: Option<Box<Future<Item=(),Error=modpack_tool::Error>>> = match matches.subcommand(){
        ("update",Some(args)) => {
            let pack_path = args.value_of("pack_file").expect("pack_file is required!");

            Some(Box::new(update(pack_path,log,&core.handle())))
        }
        ("dev",Some(args)) => {
            let sub_cmd = match args.subcommand_name(){
                Some(sub_cmd)=> sub_cmd,
                None => {
                    build_cli().print_help().expect("Failed to print help. Is the terminal broken?");
                    return Ok(0);
                }
            };
            let args = args.subcommand_matches(sub_cmd).expect("due to just being given subcommand_name");

            let pack_path = args.value_of("pack_file").expect("pack_file is required due to arg parser");

            match sub_cmd{
                "try_upgrade"|"do_upgrade" => {
                    let ver = args.value_of("mc_version").expect("mc_version is required due to arg parser");

                    let file = std::fs::File::open(&pack_path).map_err(|_| report_error!("pack {} does not exist",pack_path))?;
                    let pack: ModpackConfig = ModpackConfig::load(file)
                        .map_err(|_| report_error("pack file in bad format"))?;

                    let ver = if ver.chars().next().expect("mc_version should not have length 0 due to arg parser").is_numeric(){
                        //interpret a versionreq of x as ~x
                        println!("Interpreting version {} as ~{}",ver,ver);
                        format!("~{}",ver)
                    }else{
                        ver.to_owned()
                    };
                    let ver = semver::VersionReq::parse(ver.as_str()).map_err(|_| report_error!("Second argument ({}) was not a semver version requirement",ver))?;
                    match sub_cmd{
                        "try_upgrade" => {
                            Some(Box::new(upgrade::check(&ver,pack_path.to_owned(),pack,&core.handle())))
                        }
                        "do_upgrade" => {
                            let release_status =
                                pack.auto_update_release_status.ok_or_else(|| {
                                    report_error!("Pack {} must have an auto_update_release_status to be able to auto update",pack_path)
                                })?;
                            Some(Box::new(upgrade::run(ver,pack_path.to_owned(),pack,release_status,&core.handle())))
                        }
                        _ => unreachable!()
                    }
                }
                "add" => {
                    let mod_url = args.value_of("mod_url").expect("mod_url is required!");

                    add(pack_path,mod_url);
                    None
                }
                _ => {
                    build_cli().print_help().expect("Failed to print help. Is the terminal broken?");
                    None
                }
            }
        }
        _ => {
            build_cli().print_help().expect("Failed to print help. Is the terminal broken?");
            None
        }
    };
    match core.run(run){
        Ok(_) => Ok(0),
        Err(modpack_tool::Error(modpack_tool::ErrorKind::ReportError(string),_)) => {
            eprintln!("ERROR: {}",string);
            Ok(1)
        }
        Err(e) => Err(e)
    }
}
