use futures::{
    self,
    prelude::*,
    TryStreamExt,
};
use slog::Logger;
use std::path::PathBuf;
use serde_json::{self, Value};
use tokio::{self,io::AsyncWriteExt};
use std;
use zip;
use failure::*;

use crate::{
    BoxFuture,
    Result,
    download::{DownloadManager, Downloadable},
    hacks,
    maven,
    types::*,
    cache::Cacheable,
    util,
};
use indicatif::{MultiProgress,ProgressBar,ProgressStyle};
use std::sync::Arc;

fn bar_style() -> ProgressStyle{
    ProgressStyle::default_bar()
        .template("{prefix:23.bold.dim} {spinner:.green} [{elapsed_precise}] {wide_bar} {pos:>3}/{len:3} {msg:!}")
}

pub fn update(path: PathBuf, log: Logger) -> BoxFuture<()> {
    let mprog = Arc::new(MultiProgress::new());
    let download_manager = DownloadManager::new();

    let mprog_runner = mprog.clone();

    info!(log, "loading pack config");
    Box::pin(async move {
        let bar = mprog.add(ProgressBar::new(3));
        bar.set_style(bar_style());
        let t_handle = std::thread::spawn(move ||{
            mprog_runner.join().unwrap();
        });
        let file = std::fs::File::open(path.clone()).context(format!("{:?} is not a file",&path))?;
        let pack = ModpackConfig::load(file).context(format!("{:?} is not a valid modpack config",&path))?;
        let mut pack_path = PathBuf::from(".");
        let forge_maven_artifact = pack.forge_maven_artifact()?;
        pack_path.push(pack.folder());
        let ModpackConfig { name: pack_name, mods, .. } = pack;

        let install_fut = install_forge(pack_path.clone(),
                            forge_maven_artifact,
                            download_manager.clone(),
                            &log,
                            mprog.clone());

        let download_mods_fut = download_modlist(pack_path.clone(), mods, download_manager.clone(), &log, mprog.clone());

        let (id, _) = futures::try_join!(
            install_fut,
            download_mods_fut
        )?;
        //bar.enable_steady_tick(10);
        add_launcher_profile(&pack_path, pack_name, id, &log, bar)?.await?;
        info!(log,"Done");
        t_handle.join().unwrap();
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
    bar: ProgressBar,
) -> Result<impl Future<Output=Result<()>> + Send + 'static> {
    bar.set_prefix("Adding launcher profile");

    //de UNC prefix path, because apparently java can't handle it
    let pack_path = pack_path.canonicalize()?;
    let pack_path = util::remove_unc_prefix(pack_path);

    let mut mc_path = mc_install_loc();
    mc_path.push("launcher_profiles.json");

    bar.set_message("loading profile json");

    Ok(
        async move{
            let profiles_file = tokio::fs::File::open(mc_path.clone()).await?;
            let out = {
                bar.inc(1);
                bar.set_message("loaded profile json");
                let mut launcher_profiles: Value = serde_json::from_reader(profiles_file.into_std())?;

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
                bar.inc(1);
                bar.set_message("Saving new profiles json");
                serde_json::to_vec_pretty(&launcher_profiles)?
            };
            let mut out_file = tokio::fs::File::create(mc_path).await?;
            out_file.write(&out[..]).await?;
            bar.finish_with_message("done");
            Ok(())
        }
    )
}

fn download_modlist(
    mut pack_path: PathBuf,
    mod_list: ModList,
    manager: DownloadManager,
    log: &Logger,
    mprog: Arc<MultiProgress>,
) -> BoxFuture<()> {
    let log = log.new(o!("stage"=>"download_modlist"));

    Box::pin(async move{
        pack_path.push("mods");
        tokio::fs::create_dir_all(pack_path.clone()).await?;

        let entry_stream = tokio::fs::read_dir(pack_path.clone()).await?;
        let entries: Vec<_> = entry_stream.try_collect().await?;

        let bar = mprog.add(ProgressBar::new(entries.len() as u64));
        bar.set_style(bar_style());

        bar.set_prefix("Removing old mod files");

        for entry in entries {
            bar.inc(1);
            bar.set_message(format!("Removing: {}",entry.path().to_str().unwrap()).as_str());
            tokio::fs::remove_file(entry.path().clone()).await?;
        }
        bar.finish_with_message("Done");
        //TODO: add progress tracking to download manager downloads
        mod_list.download(pack_path, manager, log).await?;
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
    _mprog: Arc<MultiProgress>,
) -> BoxFuture<VersionId> {

    let log = log.new(o!("stage"=>"install_forge"));
    pack_path.push("forge");

    Box::pin(async move{
        trace!(log,"Creating pack folder");
        tokio::fs::create_dir_all(pack_path.clone()).await?;
        trace!(log,"Created pack folder");

        let forge_maven_artifact_path = forge_artifact.to_path();
        let reader = forge_artifact.clone().reader(manager.clone(), log.clone()).await?;

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
        tokio::fs::create_dir_all(mc_path.clone()).await?;

        mc_path.push(format!("{}.json", version_id.as_str()));

        debug!(log, "saving version json to minecraft install loc");

        let version_file = tokio::fs::File::create(mc_path.clone()).await?;
        //TODO: figure out how to use tokio copy here
        //note zip_reader.by_name() returns a ZipFile and ZipFile: !Send
        std::io::copy(&mut zip_reader.by_name("version.json")?,
                        &mut version_file.into_std())?;

        debug!(log, "Applying version json hacks");
        hacks::hack_forge_version_json(mc_path)?;

        let mut mc_path = mc_install_loc();
        mc_path.push("libraries");
        mc_path.push(forge_maven_artifact_path);
        mc_path.pop(); //pop the filename

        forge_artifact.install_at_no_classifier(mc_path, manager, log).await?;
        Ok(VersionId(version_id))
    })
}
