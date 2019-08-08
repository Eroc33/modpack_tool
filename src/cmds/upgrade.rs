use serde_json;
use kuchiki;
use futures;
use regex;
use semver;
use termcolor;
//FIXME: has_class in kuchiki should probably not require selectors to be imported
//       maybe file a bug for this
use selectors::Element;
use selectors::attr::CaseSensitivity;
use std;
use nom;

use futures::{
    prelude::*,
    TryStreamExt,
    TryFutureExt,
};
use kuchiki::{
    ElementData,
    NodeDataRef,
    traits::TendrilSink,
};
use crate::{
    download::HttpSimple,
    types::ReleaseStatus,
    ModList,
};
use std::{
    io::{Cursor,Write},
    sync::Arc,
};
use url::Url;
use regex::Regex;

use termcolor::{ColorSpec, WriteColor,Color::*};


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
        spec.set_fg(Some(Cyan)).set_bold(true).set_intense(true);
        spec
    };
    static ref WARN_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Yellow)).set_bold(true).set_intense(true);
        spec
    };
    static ref SUCCESS_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Green)).set_bold(true).set_intense(true);
        spec
    };
    static ref FAILURE_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Red)).set_bold(true).set_intense(true);
        spec
    };
    static ref DEFAULT_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(White));
        spec
    };
}

#[derive(Debug)]
struct ModVersionInfo {
    modd: curseforge::Mod,
    file_name: String,
    download_url: Url,
    release_status: ReleaseStatus,
    game_versions: Vec<semver::Version>,
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
            map!(re_match!(r"(?i)Y|Yes"), |_| Some(Response::Yes)) |
            map!(re_match!(r"(?i)N|No"), |_| Some(Response::No)) |
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

fn prompt_yes_no(prompt: String, default: Response) -> Response {
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
        match modd {
            ModSource::CurseforgeMod(cfm) => {
                Box::pin(async move{
                    let (_res,url) = http_client.get_following_redirects(cfm.project_uri()?)?.await?;
                    let id = crate::curseforge::parse_modid_from_url(url.as_str()).expect("Bad redirect on curseforge?");
                    Ok(ModSource::CurseforgeMod(crate::curseforge::Mod{
                        id,
                        ..cfm
                    }))
                }) as crate::BoxFuture<ModSource>
            }
            mvn @ ModSource::MavenMod{..} => Box::pin(futures::future::ok(mvn)),
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
            .map(|s| s.to_owned())
    }
}

fn curseforge_ver_to_semver<S>(version: S) -> semver::Version
    where S: Into<String>
{
    let version = version.into();
    //this is an un-intelligent hack to fix mods with minecraft versions like 1.12 to match semver
    let version = if version.chars().filter(|&c| c=='.').count() == 1 {
        version + ".0"
    }else{
        version
    };
    semver::Version::parse(version.as_str()).expect("Bad version from curseforge.com")
}

fn find_most_recent(
    project_name: String,
    target_game_version: semver::VersionReq,
    http_client: HttpSimple,
    target_release_status: ReleaseStatus,
) -> impl Future<Output=Result<Option<ModVersionInfo>,crate::Error>> + Send {
    lazy_static! {
        static ref TITLE_REGEX: Regex = regex::Regex::new("(<div>)|(</div><div>)|(</div>)")
            .expect("Couldn't compile pre-checked regex");
    }

    const BASE_URL: &str = "https://minecraft.curseforge.com";
    let base_url = Url::parse(BASE_URL).unwrap();
    let scrape_url = base_url
        .join(&format!("/projects/{}/files", project_name))
        .unwrap();
    async move{
        let uri = crate::util::url_to_uri(&scrape_url)?;
        let body = http_client.get(uri)
                .map_err(crate::Error::from)
                .await?
                .into_body()
                .map_ok(hyper::Chunk::into_bytes).try_concat().await?;
        let doc = kuchiki::parse_html()
            .from_utf8()
            .read_from(&mut Cursor::new(body))
            .unwrap();
        let rows = doc.select("table.project-file-listing tbody tr")
            .map_err(|_| crate::Error::Selector)?;
        for row in rows {
            let release_status =
                row.select_first(".project-file-release-type div").get_attr("title");
            let files_cell = row.select_first(".project-file-name div");
            let file_name = files_cell.select_first(".project-file-name-container .overflow-tip").text_contents();
            let primary_file = files_cell.select_first(".project-file-download-button a").get_attr("href");
            let version_container = row.select_first(".project-file-game-version");
            let mut game_versions: Vec<semver::Version> = vec![];
            if version_container.has_class(&("multiple".into()),CaseSensitivity::CaseSensitive){
                let additional_versions = version_container.select_first(".additional-versions");
                if let Some(title) = additional_versions.get_attr("title"){
                    for version in TITLE_REGEX.split(title.as_str()){
                        if !(version.is_empty() || version.starts_with("Java") || version.starts_with("java")){
                            game_versions.push(curseforge_ver_to_semver(version));
                        }
                    }
                }
            }
            let primary_game_version = row.select_first(".project-file-game-version .version-label").text_contents();
            game_versions.push(curseforge_ver_to_semver(primary_game_version));

            let release_status =
            release_status.map(|status| status.parse().expect("Invalid ReleaseStatus"));

            if release_status.map(|status| target_release_status.accepts(&status)).unwrap_or(false) && game_versions.iter().any(|ver| target_game_version.matches(ver)){
                let url = primary_file.map(|s| base_url.join(&s).unwrap()).unwrap();
                return Ok(Some(ModVersionInfo {
                    modd: curseforge::Mod::from_url(url.as_str())?,
                    file_name,
                    download_url: url,
                    release_status: release_status.unwrap(),
                    game_versions,
                }));
            }
        }
        Ok(None)
    }
}

use crate::curseforge;
use crate::types::{ModSource, ModpackConfig};

pub fn new_version(
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
                        let found = find_most_recent(curse_mod.id.clone(),
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
                                format_coloredln!((*COLOR_OUTPUT); (&INFO_COLOR){ " (as {} {} release)", a_an, found.release_status.value() } );
                            }else{
                                format_coloredln!((*COLOR_OUTPUT); "" );
                            }
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

        let mut total = 0usize;
        let mut alpha_compatible = 0usize;
        let mut beta_compatible = 0usize;
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
                let percent_beta_compatible = (beta_compatible as f32)/(total as f32) * 100.0;
                println!("(although {:.1}% are compatible only in beta release)",percent_beta_compatible);
            }
            if alpha_compatible != 0 {
                min_required_status = ReleaseStatus::Alpha;
                let percent_alpha_compatible = (alpha_compatible as f32)/(total as f32) * 100.0;
                println!("(although {:.1}% are compatible only in alpha release)",percent_alpha_compatible);
            }
            if prompt_yes_no("Upgrade now?".into(),Response::Yes) == Response::Yes{
                match min_required_status {
                    ReleaseStatus::Alpha if pack_update_status != ReleaseStatus::Alpha => {
                        if prompt_yes_no("This will mean your pack must use alpha status mods. Is this ok?".into(),Response::No) == Response::No{
                            println!("Canceling upgrade");
                            return Ok(());
                        }
                    },
                    ReleaseStatus::Beta if pack_update_status != ReleaseStatus::Beta => {
                        if prompt_yes_no("This will mean your pack must use beta status mods. Is this ok?".into(),Response::No) == Response::No{
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

                let mut file = std::fs::File::create(pack_path).expect("pack does not exist");
                serde_json::ser::to_writer_pretty(&mut file, &pack)?;
                return Ok(());
            }
        }else{
            let percent_compatible = (compatible.len() as f32)/(total as f32) * 100.0;
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

pub fn same_version(
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
                    let found = find_most_recent(curse_mod.id.clone(),
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
                                found.file_name);
                            if prompt_yes_no(prompt,Response::Yes) == Response::Yes {
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

        let mut file = std::fs::File::create(pack_path).expect("pack does not exist");
        pack.save(&mut file)?;
        Ok(())
    }
}
