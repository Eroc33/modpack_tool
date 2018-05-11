#![feature(proc_macro, custom_derive, plugin, slice_patterns, generators, proc_macro_non_items)]

extern crate clap;
extern crate env_logger;
extern crate futures_await as futures;
extern crate modpack_tool;
extern crate semver;
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate failure;
extern crate slog_json;
extern crate slog_term;
extern crate tokio;
extern crate zip;

use failure::*;

use futures::prelude::*;
use modpack_tool::Result;
use modpack_tool::types::*;

use slog::{Drain, Logger};

use std::sync::{Arc, Mutex};

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

fn main() -> Result<()> {
    env_logger::init();
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

    let log_file = std::fs::File::create(log_path).expect("Couldn't open log file");
    let log_file_stream = slog_json::Json::default(log_file);

    let root = Logger::root(
        Arc::new(Mutex::new(
            slog::Duplicate::new(slog_term::term_compact(), log_file_stream).fuse(),
        )).ignore_res(),
        o!(),
    );
    let log = root.new(o!());

    let run: Option<Box<Future<Item = (), Error = modpack_tool::Error> + Send>> =
        match matches.subcommand() {
            ("update", Some(args)) => {
                let pack_path = args.value_of("pack_file").expect("pack_file is required!");

                Some(Box::new(modpack_tool::cmds::update(
                    pack_path.to_owned(),
                    log.clone(),
                )))
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
                    "try_upgrade" | "do_upgrade" => {
                        let ver = args.value_of("mc_version")
                            .expect("mc_version is required due to arg parser");

                        let file = std::fs::File::open(&pack_path)
                            .context(format!("pack {} does not exist", pack_path))?;
                        let pack: ModpackConfig =
                            ModpackConfig::load(file).context(format!("pack file in bad format"))?;

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
                        match sub_cmd {
                            "try_upgrade" => Some(Box::new(modpack_tool::cmds::upgrade::check(
                                &ver,
                                pack_path.to_owned(),
                                pack,
                            ))),
                            "do_upgrade" => {
                                let release_status = pack.auto_update_release_status
                                    .ok_or(modpack_tool::Error::AutoUpdateDisabled)
                                    .context(format!(
                                        "Pack {} has no auto_update_release_status",
                                        pack_path
                                    ))?;
                                Some(Box::new(modpack_tool::cmds::upgrade::run(
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

                        Some(Box::new(modpack_tool::cmds::add(pack_path.to_owned(), mod_url.to_owned())))
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
        None => return Ok(()),
    };

    let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
    tokio::run(
        run.then(move |res|{
            tx.send(res).expect("Send failure while sending error");
            Ok(())
        })
    );
    rx.recv().expect("Recv failure while getting error")
}
