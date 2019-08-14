#![feature(plugin, slice_patterns, generators, proc_macro_hygiene, async_await)]

use env_logger;

use structopt::StructOpt;
use modpack_tool;

#[macro_use]
extern crate slog;

use slog_json;
use slog_term;
use slog_stdlog;
use tokio;

use sentry;

use modpack_tool::Result;

use slog::{Drain, Logger};

use std::sync::{Arc, Mutex};

async fn async_main() -> Result<()> {
    let _sentry = sentry::init("https://0b0da309fa014d60b7b5e6a9da40529e@sentry.io/1207316");
    sentry::integrations::panic::register_panic_handler();
    let mut builder = env_logger::Builder::from_default_env();
    sentry::integrations::log::init(Some(Box::new(builder.build())), Default::default());
    let command = modpack_tool::cmds::Args::from_args();

    let log_path = "modpack_tool.log";

    let log_file = tokio::fs::File::create(log_path).await.expect("Couldn't open log file");
    let log_file_stream = slog_json::Json::default(log_file.into_std());

    let root = Logger::root(
        Arc::new(Mutex::new(
            /*slog::Duplicate::new(slog_stdlog::StdLog.filter_level(slog::Level::Warning),
                slog::Duplicate::new(slog_term::term_compact().filter_level(slog::Level::Error), */log_file_stream/*))*/.fuse(),
        )).ignore_res(),
        o!(),
    );
    let log = root.new(o!());

    if let Err(e) = command.dispatch(log).await {
        println!("Reporting error to sentry: {:?}", e);
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