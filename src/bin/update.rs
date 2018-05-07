#![feature(proc_macro, custom_derive, plugin, slice_patterns, generators, proc_macro_non_items)]

extern crate clap;
extern crate env_logger;
extern crate futures_await as futures;
extern crate modpack_tool;
extern crate semver;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate slog_json;
extern crate slog_term;
extern crate tokio;
extern crate zip;

use futures::prelude::*;
use modpack_tool::{BoxFuture, Result};
use modpack_tool::download::{DownloadManager, Downloadable};
use modpack_tool::hacks;
use modpack_tool::maven;
use modpack_tool::upgrade;
use modpack_tool::types::*;
use modpack_tool::cache::Cacheable;
use modpack_tool::util;
use modpack_tool::fs_futures;

use tokio::prelude::*;

use slog::{Drain, Logger};

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;

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

fn replace<P, R, FUT, F>(path: P, f: F) -> BoxFuture<()>
where
    P: Into<PathBuf>,
    R: AsyncRead + Send,
    FUT: Future<Item = R, Error = modpack_tool::Error> + Send,
    F: FnOnce(tokio::fs::File) -> FUT + Send + 'static,
{
    let path = path.into();
    Box::new(async_block!{
        let file = await!(tokio::fs::File::open(path.clone()))?;
        let out = await!(f(file))?;
        let out_file = await!(tokio::fs::File::create(path))?;
        await!(tokio::io::copy(out,out_file))?;
        Ok(())
    })
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

    Ok(replace(mc_path, |profiles_file| {
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

fn add<P>(pack_path: P, mod_url: String) -> BoxFuture<()>
where
    P: Into<PathBuf>,
{
    use modpack_tool::types::ModpackConfig;

    replace(pack_path, |file| {
        async_block!{
            let mut pack: ModpackConfig =
                serde_json::de::from_reader(file).expect("pack file in bad format");

            pack.add_mod_by_url(mod_url.as_str())
                .expect("Unparseable modsource url");

            Ok(std::io::Cursor::new(serde_json::to_vec_pretty(&pack)?))
        }
    })
}

fn update(path: String, log: Logger) -> BoxFuture<()> {
    let download_manager = DownloadManager::new();

    info!(log, "loading pack config");
    Box::new(async_block!{
        let file = std::fs::File::open(path)?;
        let pack = ModpackConfig::load(file)?;
        let mut pack_path = PathBuf::from(".");
        let forge_maven_artifact = pack.forge_maven_artifact()?;
        pack_path.push(pack.folder());
        let ModpackConfig { name: pack_name, mods, .. } = pack;

        let joint_task: Box<Future<Item=(VersionId,()),Error=::modpack_tool::Error>+Send> = Box::new(
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

fn build_cli() -> clap::App<'static, 'static> {
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

fn report_error<S: Into<String>>(s: S) -> modpack_tool::Error {
    modpack_tool::Error::ReportError(s.into())
}

fn run() -> Result<i32> {
    env_logger::init();
    let matches = build_cli().get_matches();

    let log_path = "modpack_tool.log";

    let log_file = std::fs::File::create(log_path).expect("Couldn't open log file");
    let log_file_stream = slog_json::Json::default(log_file);

    let root = Logger::root(
        Arc::new(Mutex::new(
            slog::Duplicate::new(slog_term::term_compact(), log_file_stream).fuse(),
        )).ignore_res(),
        o!(),
    );
    let log = root.new(o!());

    let run: Option<Box<Future<Item = (), Error = modpack_tool::Error> + Send>> = match matches
        .subcommand()
    {
        ("update", Some(args)) => {
            let pack_path = args.value_of("pack_file").expect("pack_file is required!");

            Some(Box::new(update(pack_path.to_owned(), log)))
        }
        ("dev", Some(args)) => {
            let sub_cmd = match args.subcommand_name() {
                Some(sub_cmd) => sub_cmd,
                None => {
                    build_cli()
                        .print_help()
                        .expect("Failed to print help. Is the terminal broken?");
                    return Ok(0);
                }
            };
            let args = args.subcommand_matches(sub_cmd)
                .expect("due to just being given subcommand_name");

            let pack_path = args.value_of("pack_file")
                .expect("pack_file is required due to arg parser");

            match sub_cmd {
                "try_upgrade" | "do_upgrade" => {
                    let ver = args.value_of("mc_version")
                        .expect("mc_version is required due to arg parser");

                    let file = std::fs::File::open(&pack_path)
                        .map_err(|_| report_error!("pack {} does not exist", pack_path))?;
                    let pack: ModpackConfig = ModpackConfig::load(file)
                        .map_err(|_| report_error("pack file in bad format"))?;

                    let ver = if ver.chars()
                        .next()
                        .expect("mc_version should not have length 0 due to arg parser")
                        .is_numeric()
                    {
                        //interpret a versionreq of x as ~x
                        println!("Interpreting version {} as ~{}", ver, ver);
                        format!("~{}", ver)
                    } else {
                        ver.to_owned()
                    };
                    let ver = semver::VersionReq::parse(ver.as_str()).map_err(|_| {
                        report_error!(
                            "Second argument ({}) was not a semver version requirement",
                            ver
                        )
                    })?;
                    match sub_cmd {
                        "try_upgrade" => {
                            Some(Box::new(upgrade::check(&ver, pack_path.to_owned(), pack)))
                        }
                        "do_upgrade" => {
                            let release_status = pack.auto_update_release_status.ok_or_else(
                                || {
                                    report_error!("Pack {} must have an auto_update_release_status to be able to auto update",pack_path)
                                },
                            )?;
                            Some(Box::new(upgrade::run(
                                ver,
                                pack_path.to_owned(),
                                pack,
                                release_status,
                            )))
                        }
                        _ => unreachable!(),
                    }
                }
                "add" => {
                    let mod_url = args.value_of("mod_url").expect("mod_url is required!");

                    Some(add(pack_path, mod_url.to_owned()))
                }
                _ => {
                    build_cli()
                        .print_help()
                        .expect("Failed to print help. Is the terminal broken?");
                    None
                }
            }
        }
        _ => {
            build_cli()
                .print_help()
                .expect("Failed to print help. Is the terminal broken?");
            None
        }
    };

    let run = match run {
        Some(f) => f,
        None => return Ok(0),
    };

    let (tx, rx) = std::sync::mpsc::channel::<i32>();
    tokio::run(
        run.map(move |_| 0i32)
            .then(move |res| match res {
                Err(modpack_tool::Error::ReportError(string)) => {
                    eprintln!("ERROR: {}", string);
                    Ok(1)
                }
                Err(e) => {
                    eprintln!("{}", e);
                    Ok(1)
                }
                Ok(v) => Ok(v),
            })
            .map(move |ret_val| {
                tx.send(ret_val).unwrap();
            }),
    );

    match rx.recv() {
        Ok(ret_val) => Ok(ret_val),
        Err(_) => Ok(1),
    }
}

fn main() {
    ::std::process::exit(match run() {
        Ok(ret) => ret,
        Err(ref e) => {
            eprintln!("{}", e);
            1
        }
    });
}
