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

use modpack_tool::{
    Result,
    mod_source::{ModpackConfig,IndirectableModpack},
    error::prelude::*,
};

use snafu::Snafu;
use slog::{Drain, Logger};
use std::sync::{Arc, Mutex};

async fn load_hybrid_config() -> modpack_tool::Result<Option<ModpackConfig>>{
    #[derive(Debug,Snafu)]
    enum HybridConfigError{
        #[snafu(display("Io Error while opening hybrid config: {}", source))]
        Io{
            source: std::io::Error,
        },
        #[snafu(display("Zip Error while opening hybrid config: {}", source))]
        Zip{
            source: zip::result::ZipError,
        },
        #[snafu(display("Json Error while opening hybrid config: {}", source))]
        Json{
            source: serde_json::Error,
        },
    }

    let own_path = std::env::args().nth(0).expect("arg 0 should always be available");
    let file = tokio::fs::File::open(own_path).await.context(Io).erased()?;
    let mut zip_reader = match zip::read::ZipArchive::new(file.into_std()){
        Err(zip::result::ZipError::InvalidArchive(_)) => {
            return Ok(None);
        }
        other => other,
    }.context(Zip).erased()?;
    let indirected: IndirectableModpack = serde_json::from_reader(zip_reader.by_name("config.json").context(Zip).erased()?).context(Json).erased()?;
    Ok(Some(indirected.resolve().await?))
}

async fn async_main() -> Result<()> {
    let _sentry = sentry::init("https://0b0da309fa014d60b7b5e6a9da40529e@sentry.io/1207316");
    sentry::integrations::panic::register_panic_handler();
    let mut builder = env_logger::Builder::from_default_env();
    sentry::integrations::log::init(Some(Box::new(builder.build())), Default::default());
    

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
    let cmd_res = if let Ok(Some(pack)) = load_hybrid_config().await{
        modpack_tool::cmds::update(pack,log).await
    }else{
        let command = modpack_tool::cmds::Args::from_args();
        command.dispatch(log).await 
    };
    if let Err(e) = cmd_res {
        println!("Error: {}", e);
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