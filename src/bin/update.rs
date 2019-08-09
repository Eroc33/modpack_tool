#![feature(plugin, slice_patterns, generators, proc_macro_hygiene, async_await)]

use clap;
use env_logger;

use modpack_tool;
use semver;

#[macro_use]
extern crate slog;

use slog_json;
use slog_term;
use slog_stdlog;
use tokio;

use sentry;

use failure::*;

use modpack_tool::Result;
use modpack_tool::types::*;

use slog::{Drain, Logger};

use std::sync::{Arc, Mutex};
use std::path::PathBuf;

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
                clap::SubCommand::with_name("upgrade")
                .about("Checks upgrade compatibility for this pack from one minecraft version to the next.")
                .arg(clap::Arg::with_name("pack_file")
                    .required(true)
                    .index(1)
                    .help("The metadata json file for the pack you wish to modify"))
                .arg(clap::Arg::with_name("version")
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

async fn run_command<'a>(matches: clap::ArgMatches<'a>, log: Logger) -> modpack_tool::Result<()>
{
    match matches.subcommand() {
        ("update", Some(args)) => {
            let pack_path: PathBuf = args.value_of("pack_file").expect("pack_file is required!").into();

            if !pack_path.exists(){
                eprintln!("{:?} is not an accesible path",pack_path);
                Ok(())
            } else if !pack_path.is_file(){
                eprintln!("No file exists at the path {:?}",pack_path);
                Ok(())
            }else{
                modpack_tool::cmds::update(
                    pack_path,
                    log.clone(),
                ).await
            }
        }
        ("dev", Some(args)) => {
            let sub_cmd = match args.subcommand_name() {
                Some(sub_cmd) => sub_cmd,
                None => {
                    build_cli()
                        .print_help()
                        .expect("Failed to print help. Is the terminal broken?");
                    return Ok(());
                }
            };
            let args = args.subcommand_matches(sub_cmd)
                .expect("due to just being given subcommand_name");

            let pack_path = args.value_of("pack_file")
                .expect("pack_file is required due to arg parser");

            match sub_cmd {
                "upgrade" => {
                    let mut file = std::fs::File::open(&pack_path)
                        .context(format!("pack {} does not exist", pack_path))?;
                    let pack: ModpackConfig = serde_json::from_reader(&mut file).context("pack file in bad format".to_string())?;

                    if let Some(ver) = args.value_of("mc_version"){
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
                        let ver = semver::VersionReq::parse(ver.as_str()).context(format!(
                            "Second argument ({}) was not a semver version requirement",
                            ver
                        ))?;
                        modpack_tool::cmds::upgrade::new_version(
                            ver,
                            pack_path.to_owned(),
                            pack,
                        ).await
                    }else{
                        let release_status = pack.auto_update_release_status
                            .ok_or(modpack_tool::Error::AutoUpdateDisabled)
                            .context(format!(
                                "Pack {} has no auto_update_release_status",
                                pack_path
                            ))?;
                        modpack_tool::cmds::upgrade::same_version(
                            pack_path.to_owned(),
                            pack,
                            release_status,
                        ).await
                    }
                }
                "add" => {
                    let mod_url = args.value_of("mod_url").expect("mod_url is required!");

                    modpack_tool::cmds::add(pack_path.to_owned(), mod_url.to_owned()).await
                }
                _ => {
                    build_cli()
                        .print_help()
                        .expect("Failed to print help. Is the terminal broken?");
                    Ok(())
                }
            }
        }
        _ => {
            build_cli()
                .print_help()
                .expect("Failed to print help. Is the terminal broken?");
            Ok(())
        }
    }
}

async fn async_main() -> Result<()> {
    let _sentry = sentry::init("https://0b0da309fa014d60b7b5e6a9da40529e@sentry.io/1207316");
    sentry::integrations::panic::register_panic_handler();
    let mut builder = env_logger::Builder::from_default_env();
    sentry::integrations::log::init(Some(Box::new(builder.build())), Default::default());
    let matches = match build_cli().get_matches_safe(){
        Ok(matches) => matches,
        Err(e) => {
            if e.use_stderr() {
                eprintln!("{}", e.message);
            }else{
                println!("{}", e.message);
            }
            return Ok(())
        },
    };

    let log_path = "modpack_tool.log";

    let log_file = tokio::fs::File::create(log_path).await.expect("Couldn't open log file");
    let log_file_stream = slog_json::Json::default(log_file.into_std());

    let root = Logger::root(
        Arc::new(Mutex::new(
            slog::Duplicate::new(slog_stdlog::StdLog.filter_level(slog::Level::Warning),
                slog::Duplicate::new(slog_term::term_compact().filter_level(slog::Level::Error), log_file_stream)).fuse(),
        )).ignore_res(),
        o!(),
    );
    let log = root.new(o!());

    if let Err(e) = run_command(matches, log).await {
        println!("Reporting error to sentry");
        sentry::integrations::failure::capture_fail(&e);
    }
    Ok(())
}

fn main() ->  Result<()>  {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.spawn(async move {
        tx.send(async_main().await).unwrap();
    });
    rt.block_on(rx).unwrap()
}