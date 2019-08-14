use kuchiki;
use futures;
use semver;
use termcolor;
use std;
use nom;

use futures::{
    prelude::*,
};
use kuchiki::{
    ElementData,
    NodeDataRef,
};
use crate::{
    download::HttpSimple,
    curseforge::ReleaseStatus,
    mod_source::ModList,
};
use std::{
    io::Write,
    sync::Arc,
};
use url::Url;
use structopt::StructOpt;
use failure::ResultExt;

use termcolor::{ColorSpec, WriteColor, Color};


macro_rules! print_inline{
    ($($args:tt)+) => {{
        print!($($args)+);
        if let Err(e) = std::io::stdout().flush(){
            panic!("Failed to flush stdout: {}",e);
        }
    }};
}

macro_rules! readln{
    () => {{
        let mut new_name = String::new();
        if let Err(e) = std::io::stdin().read_line(&mut new_name){
            panic!("Failed to read stdin: {}",e);
        }
        new_name
    }};
}

macro_rules! format_coloredln{
    ($output:expr; $($rest:tt)+ ) => {
        let mut buf = $output.buffer();
        format_colored!(_impl buf; $($rest)+ );
        writeln!(buf)?;
        $output.print(&buf)?;
    };
}

macro_rules! format_colored{
    ($output:expr; $($rest:tt)+ ) => {
        let mut buf = $output.buffer();
        format_colored!(_impl buf; $($rest)+ );
        $output.print(&buf)?;
    };
    (_impl $buf:expr ; ($color:expr){ $($inner: tt)* }, $($rest: tt)+ ) =>{
        $buf.set_color($color)?;
        format_colored!(_impl $buf; $($inner)* );
        $buf.set_color(&DEFAULT_COLOR)?;
        format_colored!(_impl $buf; $($rest)* );
    };
    (_impl $buf:expr ; ($color:expr){ $($inner: tt)* } ) =>{
        $buf.set_color($color)?;
        format_colored!(_impl $buf; $($inner)* );
        $buf.set_color(&DEFAULT_COLOR)?;
    };
    (_impl $buf:expr ; $($rest: tt)* ) =>{
        write!($buf, $($rest)+ )?;
    };
}

lazy_static! {
    static ref COLOR_OUTPUT: Arc<termcolor::BufferWriter> = Arc::new(termcolor::BufferWriter::stdout(
        termcolor::ColorChoice::Always
    ));
    static ref INFO_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Color::Cyan)).set_bold(true).set_intense(true);
        spec
    };
    static ref WARN_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Color::Yellow)).set_bold(true).set_intense(true);
        spec
    };
    static ref SUCCESS_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Color::Green)).set_bold(true).set_intense(true);
        spec
    };
    static ref FAILURE_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Color::Red)).set_bold(true).set_intense(true);
        spec
    };
    static ref DEFAULT_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Color::White));
        spec
    };
}

#[derive(Debug)]
struct ModVersionInfo {
    modd: curseforge::Mod,
    download_url: Url,
    release_status: ReleaseStatus,
}

#[derive(Debug,PartialEq, Eq)]
enum Response {
    Yes,
    No,
}

impl Response{
    fn from_str(s: &str) -> Result<Option<Self>, ()> {
        let res = alt_complete!(
            s.trim(),
            map!(re_match!(r"(?i)Y|Yes"), |_| Some(Self::Yes)) |
            map!(re_match!(r"(?i)N|No"), |_| Some(Self::No)) |
            map!(tag!(""), |_| None)
        );
        match res{
            nom::IResult::Done(i,o) => if i.is_empty(){ Ok(o) } else { Err(()) },
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests{
    use super::Response;
    #[test]
    fn response_parses_yes(){
        assert_eq!(Response::from_str("yes"),Ok(Some(Response::Yes)));
        assert_eq!(Response::from_str("Yes"),Ok(Some(Response::Yes)));
        assert_eq!(Response::from_str("y"),Ok(Some(Response::Yes)));
        assert_eq!(Response::from_str("Y"),Ok(Some(Response::Yes)));
    }

    #[test]
    fn response_parses_no(){
        assert_eq!(Response::from_str("no"),Ok(Some(Response::No)));
        assert_eq!(Response::from_str("No"),Ok(Some(Response::No)));
        assert_eq!(Response::from_str("n"),Ok(Some(Response::No)));
        assert_eq!(Response::from_str("N"),Ok(Some(Response::No)));
    }

    #[test]
    fn response_parses_other(){
        assert!(Response::from_str("adsafggg").is_err());
        assert!(Response::from_str("??£££23").is_err());
    }

    #[test]
    fn response_parses_empty(){
        assert_eq!(Response::from_str(""),Ok(None));
    }
}

fn prompt_yes_no(prompt: &str, default: Response) -> Response {
    loop{
        match default {
            Response::Yes => print_inline!("{}[Y/n]",prompt),
            Response::No => print_inline!("{}[y/N]",prompt),
        }
        let line = readln!();
        match Response::from_str(line.as_str()){
            Ok(Some(r)) => return r,
            Ok(None) => return default,
            Err(_) => println!("Please enter yes, no, or nothing."),
        }
    }
}

//Checks if any curseforge projects have been moved, and updates the names
fn update_project_names(mods: ModList) -> Vec<impl Future<Output=Result<ModSource,crate::Error>> + Send + 'static>{
    let http_client = HttpSimple::new();
    mods.into_iter().map(|modd|{
        let http_client = http_client.clone();
        async move{
            match modd {
                ModSource::CurseforgeMod(cfm) => {
                    let (_res,url) = http_client.get_following_redirects(cfm.project_uri()?)?.await?;
                    let id = crate::curseforge::parse_modid_from_url(url.as_str()).expect("Bad redirect on curseforge?");
                    Ok(ModSource::CurseforgeMod(crate::curseforge::Mod{
                        id,
                        ..cfm
                    }))
                }
                mvn @ ModSource::MavenMod{..} => Ok(mvn),
            }
        }
    }).collect()
}

trait SelectExt{
    fn select(&self,selector: &'static str) -> kuchiki::iter::Select<kuchiki::iter::Elements<kuchiki::iter::Descendants>>;
    fn select_first(&self,selector: &'static str) -> NodeDataRef<ElementData>{
        self.select(selector).next().unwrap()
    }
    fn get_attr(&self, name: &str) -> Option<String>;
}

impl SelectExt for NodeDataRef<ElementData>{
    fn select(&self,selector: &'static str) -> kuchiki::iter::Select<kuchiki::iter::Elements<kuchiki::iter::Descendants>>
    {
        self.as_node().select(selector).unwrap()
    }
    fn get_attr(&self, name: &str) -> Option<String>
    {
        self
            .attributes
            .borrow()
            .get(name)
            .map(std::borrow::ToOwned::to_owned)
    }
}

fn find_most_recent(
    curse_mod: curseforge::Mod,
    target_game_version: semver::VersionReq,
    http_client: HttpSimple,
    target_release_status: ReleaseStatus,
) -> impl Future<Output=Result<Option<ModVersionInfo>,crate::Error>> + Send {
    let mut stream = Box::pin(crate::curseforge::api::all_for_version(curse_mod, http_client, target_game_version).try_filter(move |release_info| {
        futures::future::ready(
            target_release_status.accepts(release_info.release_status)
                //already filtering by this on get
                //&& game_versions.iter().any(|ver| target_game_version.matches(ver))
        )
    }));
    async move{
        Ok(if let Some(release_info) = stream.try_next().await?{
            let url = Url::parse(&format!("https://www.curseforge.com/minecraft/mc-mods/{}/download/{}/file",release_info.modd.id,release_info.modd.version)).expect("bad prechecked url");
            Some(ModVersionInfo {
                modd: release_info.modd,
                download_url: url,
                release_status: release_info.release_status,
            })
        }else{
            None
        })
    }
}

use crate::curseforge;
use crate::mod_source::{ModSource, ModpackConfig};

fn new_version(
    target_game_version: semver::VersionReq,
    pack_path: String,
    mut pack: ModpackConfig,
) -> impl Future<Output=Result<(), crate::Error>> + Send + 'static {
    let http_client = HttpSimple::new();

    let strm = update_project_names(pack.mods.clone()).into_iter().collect::<futures::stream::futures_unordered::FuturesUnordered<_>>()
        .and_then(move |modd|{
            let target_game_version = target_game_version.clone();
            let http_client_handle = http_client.clone();
            async move{
                match modd{
                    ModSource::CurseforgeMod(curse_mod) => {
                        let found = find_most_recent(curse_mod.clone(),
                                            target_game_version,
                                            http_client_handle,
                                            ReleaseStatus::Alpha).await?;
                        if let Some(found) = found {
                            format_colored!((*COLOR_OUTPUT); (&SUCCESS_COLOR){"  COMPATIBLE: "}, "{}", curse_mod.id );
                            assert_eq!(curse_mod.id, found.modd.id);
                            if found.release_status != ReleaseStatus::Release {
                                let a_an = if found.release_status == ReleaseStatus::Alpha{
                                    "an"
                                }else if found.release_status == ReleaseStatus::Beta{
                                    "a"
                                }else{
                                    unreachable!("Status was not release, alpha, or beta")
                                };
                                format_colored!((*COLOR_OUTPUT); (&INFO_COLOR){ " (as {} {} release)", a_an, found.release_status.value() } );
                            }
                            format_coloredln!((*COLOR_OUTPUT); "" );
                            Ok((curse_mod.into(),Some(found.release_status)))
                        } else {
                            format_coloredln!((*COLOR_OUTPUT); (&FAILURE_COLOR){"INCOMPATIBLE: "}, "{}", curse_mod.id );
                            Ok((curse_mod.into(),None))
                        }
                    }
                    ModSource::MavenMod { artifact, repo } => {
                        format_colored!((*COLOR_OUTPUT); (&WARN_COLOR){"you must check maven mod: {:?}",artifact});
                        Ok((ModSource::MavenMod { artifact, repo },None))
                    },
                }
            }
        });

    async move{

        let modlist: Vec<(ModSource,Option<ReleaseStatus>)> = strm.try_collect::<Vec<_>>().await?;

        let mut total = 0_usize;
        let mut alpha_compatible = 0_usize;
        let mut beta_compatible = 0_usize;
        let mut compatible = vec![];
        let mut incompatible = vec![];

        for (modd,status) in modlist{
            total += 1;
            match status{
                None => incompatible.push(modd),
                Some(ReleaseStatus::Alpha) => {
                    compatible.push(modd);
                    alpha_compatible += 1;
                }
                Some(ReleaseStatus::Beta) => {
                    compatible.push(modd);
                    beta_compatible += 1;
                }
                Some(ReleaseStatus::Release) => {
                    compatible.push(modd);
                }
            }
        }

        if incompatible.is_empty() {
            let pack_update_status = pack.auto_update_release_status.unwrap_or(ReleaseStatus::Release);
            let mut min_required_status = pack_update_status;
            println!("All of your mods are compatible.");
            if beta_compatible != 0 {
                min_required_status = ReleaseStatus::Beta;
                let percent_beta_compatible = (beta_compatible as f64)/(total as f64) * 100.0;
                println!("(although {:.1}% are compatible only in beta release)",percent_beta_compatible);
            }
            if alpha_compatible != 0 {
                min_required_status = ReleaseStatus::Alpha;
                let percent_alpha_compatible = (alpha_compatible as f64)/(total as f64) * 100.0;
                println!("(although {:.1}% are compatible only in alpha release)",percent_alpha_compatible);
            }
            if prompt_yes_no("Upgrade now?",Response::Yes) == Response::Yes{
                match min_required_status {
                    ReleaseStatus::Alpha if pack_update_status != ReleaseStatus::Alpha => {
                        if prompt_yes_no("This will mean your pack must use alpha status mods. Is this ok?",Response::No) == Response::No{
                            println!("Canceling upgrade");
                            return Ok(());
                        }
                    },
                    ReleaseStatus::Beta if pack_update_status != ReleaseStatus::Beta => {
                        if prompt_yes_no("This will mean your pack must use beta status mods. Is this ok?",Response::No) == Response::No{
                            println!("Canceling upgrade");
                            return Ok(());
                        }
                    },
                    _ => {}
                }

                println!("Enter new pack name (leave blank to keep old name):");
                let new_name = readln!();
                let new_name = new_name.trim();

                if !new_name.is_empty(){
                    pack.name = new_name.to_owned();
                }

                //TODO: smarter dedup, where we keep the higher version number of mods with colliding ids
                //dedup via hashset
                pack.mods = compatible.into_iter().collect::<std::collections::HashSet<_>>().into_iter().collect();

                let mut file = tokio::fs::File::create(pack_path).await.expect("pack does not exist");
                crate::async_json::write_pretty(&mut file, &pack).await?;
                return Ok(());
            }
        }else{
            let percent_compatible = (compatible.len() as f64)/(total as f64) * 100.0;
            format_colored!((*COLOR_OUTPUT); (&INFO_COLOR){"\
                {:.1}% of your mods are compatible.\n\
                You must remove or replace incompatible mods before you can upgrade.\n\
                {} incompatible mods:\n\
            ", percent_compatible, incompatible.len()
            });
            for modd in incompatible{
                format_coloredln!((*COLOR_OUTPUT); (&WARN_COLOR){"\t {} ( {} )",modd.identifier_string(),modd.guess_project_url().unwrap_or_else(|| "COULD NOT GUESS PROJECT URL".to_owned()) });
            }
        }
        Ok(())
    }
}

fn same_version(
    pack_path: String,
    mut pack: ModpackConfig,
    release_status: ReleaseStatus,
) -> impl Future<Output=Result<(), crate::Error>> + Send + 'static {
    let http_client = HttpSimple::new();

    let target_game_version = pack.version.clone();

    async move{
        let mut new_mods = vec![];
        //FIXME: ideally we would borrow pack.mods to iterate over it, but for now we can't due to
        //       borrow tracing limitations in generators
        let old_mods = futures::future::try_join_all(update_project_names(pack.mods.clone())).await?;
        for modd in old_mods{
            let updated = match modd {
                ModSource::CurseforgeMod(curse_mod) => {
                    let found = find_most_recent(curse_mod.clone(),
                                            target_game_version.clone(),
                                            http_client.clone(),
                                            release_status).await?;
                    if let Some(found) = found {
                        assert_eq!(curse_mod.id, found.modd.id);
                        if found.modd.version > curse_mod.version {
                            let prompt = format!("Replace {} {} with {} ({})?",
                                curse_mod.id,
                                curse_mod.version,
                                found.modd.version,
                                found.download_url);
                            if prompt_yes_no(&prompt,Response::Yes) == Response::Yes {
                                Some(found.modd.into())
                            } else {
                                println!("\t skipping.");
                                None
                            }
                        } else {
                            println!("No update available for {}", curse_mod.id);
                            None
                        }
                    } else {
                        println!("Found no matching releases for {}", curse_mod.id);
                        None
                    }
                }
                mvn_mod @ ModSource::MavenMod { .. } => {
                    println!("skipping maven mod: {:?}", mvn_mod);
                    None
                }
            };
            if let Some(updated) = updated{
                new_mods.push(updated);
            }
        }
        for modsource in new_mods {
            pack.replace_mod(modsource);
        }

        let mut file = tokio::fs::File::create(pack_path).await.expect("pack does not exist");
        crate::async_json::write_pretty(&mut file, &pack).await?;
        Ok(())
    }
}


#[derive(Debug, StructOpt)]
#[structopt(name = "upgrade", about = "Checks upgrade compatibility for this pack from one minecraft version to the next.")]
pub struct Args{
    /// The metadata json file for the pack you wish to modify
    pack_file: String,
    /// The minecraft version to upgrade to
    mc_version: Option<String>,
}

pub async fn upgrade(args: Args) -> Result<(), crate::Error>{

    let Args{pack_file, mc_version} = args;

    let mut file = std::fs::File::open(&pack_file)
        .context(format!("pack {} does not exist", pack_file))?;
    let pack: ModpackConfig = serde_json::from_reader(&mut file).context("pack file in bad format".to_string())?;

    if let Some(ver) = mc_version{
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
        new_version(
            ver,
            pack_file,
            pack,
        ).await
    }else{
        let release_status = pack.auto_update_release_status
            .ok_or(crate::Error::AutoUpdateDisabled)
            .context(format!(
                "Pack {} has no auto_update_release_status",
                pack_file
            ))?;
        same_version(
            pack_file,
            pack,
            release_status,
        ).await
    }
}