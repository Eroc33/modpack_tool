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
use tokio;
use nom;

use futures::prelude::*;
use kuchiki::{ElementData, NodeDataRef};
use kuchiki::traits::TendrilSink;
use download::HttpSimple;
use types::ReleaseStatus;
use std::borrow::Borrow;
use std::io::Cursor;
use std::sync::Arc;
use std::str::FromStr;
use url::Url;
use regex::Regex;

use termcolor::{ColorSpec, WriteColor};
use termcolor::Color::*;
use std::io::Write;

use ::ModList;


macro_rules! print_inline{
    ($($args:tt)+) => {{
        print!($($args)+);
        if let Err(e) = std::io::stdout().flush(){
            panic!("Failed to flush stdout: {}",e);
        }
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
    id: String,
    version: u64,
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
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .expect("Failed to read line");
        match Response::from_str(line.as_str()){
            Ok(Some(r)) => return r,
            Ok(None) => return default,
            Err(_) => println!("Please enter yes, no, or nothing."),
        }
    }
}

fn extract_version_and_id(url: &str) -> (u64, &str) {
    let res = do_parse!{url,
        tag_s!("https://minecraft.curseforge.com/projects/") >>
        project: take_till_s!(|c: char| c=='/') >>
        tag_s!("/files/") >>
        ver: map_res!(take_while_s!(|c: char| c.is_digit(10)),u64::from_str) >>
        opt!(tag_s!("/download")) >>
        ((ver,project))
    };
    res.to_result().expect("Unknown modsource url")
}

fn get_attr<N>(node: N, name: &str) -> Option<String>
where
    N: Borrow<NodeDataRef<ElementData>>,
{
    node.borrow()
        .attributes
        .borrow()
        .get(name)
        .map(|s| s.to_owned())
}

//Checks if any curseforge projects have been moved, and updates the names
fn update_project_names(mods: ModList) -> Vec<impl Future<Item=ModSource,Error=::Error> + Send + 'static>{
    let httpclient = HttpSimple::new();
    mods.into_iter().map(|modd|{
        let httpclient = httpclient.clone();
        match modd {
            ModSource::CurseforgeMod(cfm) => {
                Box::new(async_block!{
                    let (_res,url) = await!(httpclient.get_following_redirects(cfm.project_uri()?)?)?;
                    let id = ::curseforge::parse_modid_from_url(url.as_str()).expect("Bad redirect on curseforge?");
                    Ok(ModSource::CurseforgeMod(::curseforge::Mod{
                        id,
                        ..cfm
                    }))
                }) as Box<Future<Item=ModSource,Error=::Error> + Send>
            }
            mvn @ ModSource::MavenMod{..} => Box::new(futures::future::ok(mvn)),
        }
    }).collect()
}

fn find_most_recent(
    project_name: String,
    target_game_version: semver::VersionReq,
    http_client: HttpSimple,
    target_release_status: ReleaseStatus,
) -> impl Future<Item = Option<ModVersionInfo>, Error = ::Error> + Send {
    lazy_static! {
        static ref TITLE_REGEX: Regex = regex::Regex::new("(<div>)|(</div><div>)|(</div>)")
            .expect("Couldn't compile pre-checked regex");
    }

    const BASE_URL: &str = "https://minecraft.curseforge.com";
    let base_url = Url::parse(BASE_URL).unwrap();
    let scrape_url = base_url
        .join(&format!("/projects/{}/files", project_name))
        .unwrap();
    async_block!{
        let uri = ::util::url_to_uri(&scrape_url)?;
        let body = await!(http_client.get(uri)
                .map_err(::Error::from)
                .and_then(move |res| {
                    res.into_body().map_err(::Error::from).fold(vec![],
                                    move |mut buf, chunk| -> Result<Vec<u8>, std::io::Error> {
                                        std::io::copy(&mut Cursor::new(chunk), &mut buf)?;
                                        Ok(buf)
                                    })
                }))?;
        let doc = kuchiki::parse_html()
            .from_utf8()
            .read_from(&mut Cursor::new(body))
            .unwrap();
        let rows = doc.select("table.project-file-listing tbody tr")
            .map_err(|_| ::Error::Selector)?;
        for row in rows {
            let row = row.as_node();
            let release_status =
                get_attr(row.select(".project-file-release-type div")
                                .map_err(|_| ::Error::Selector)?
                                .next()
                                .unwrap(),
                            "title");
            let files_cell = row.select(".project-file-name div")
                .map_err(|_| ::Error::Selector)?
                .next()
                .unwrap();
            let file_name = files_cell.as_node()
                .select(".project-file-name-container .overflow-tip")
                .map_err(|_| ::Error::Selector)?
                .next()
                .unwrap()
                .text_contents();
            //let more_files_url = file_name_container.attr("href");
            let primary_file =
                get_attr(files_cell.as_node()
                                .select(".project-file-download-button a")
                                .map_err(|_| ::Error::Selector)?
                                .next()
                                .unwrap(),
                            "href");
            let version_container = row.select(".project-file-game-version")
                .map_err(|_| ::Error::Selector)?
                .next()
                .unwrap();
            let mut game_versions: Vec<semver::Version> = vec![];
            if version_container.has_class(&("multiple".into()),CaseSensitivity::CaseSensitive){
                let additional_versions = version_container.as_node().select(".additional-versions")
                    .map_err(|_| ::Error::Selector)?
                    .next()
                    .unwrap();
                let cell_ref = additional_versions.attributes.borrow();
                if let Some(title) = cell_ref.get("title"){
                    for version in TITLE_REGEX.split(title){
                        if !(version.is_empty() || version.starts_with("Java") || version.starts_with("java")){
                            //this is an un-intelligent hack to fix mods with minecraft versions like 1.12 to match semver
                            let version = if version.chars().filter(|&c| c=='.').count() == 1 {
                                version.to_owned() + ".0"
                            }else{
                                version.to_owned()
                            };
                            game_versions.push(semver::Version::parse(version.as_str()).expect("Bad version from curseforge.com"));
                        }
                    }
                }
            }
            let primary_game_version = row.select(".project-file-game-version .version-label")
                .map_err(|_| ::Error::Selector)?
                .next()
                .unwrap()
                .text_contents();
            //this is an un-intelligent hack to fix mods with minecraft versions like 1.12 to match semver
            let primary_game_version = if primary_game_version.chars().filter(|&c| c=='.').count() == 1 {
                primary_game_version.to_owned() + ".0"
            }else{
                primary_game_version.to_owned()
            };
            game_versions.push(semver::Version::parse(primary_game_version.as_str()).expect("Bad version from curseforge.com"));

            let release_status =
            release_status.map(|status| status.parse().expect("Invalid ReleaseStatus"));

            if release_status.map(|status| target_release_status.accepts(&status)).unwrap_or(false) && game_versions.iter().any(|ver| target_game_version.matches(ver)){
                let url = primary_file.map(|s| base_url.join(&s).unwrap()).unwrap();
                let (version, _) = extract_version_and_id(url.as_str());
                return Ok(Some(ModVersionInfo {
                    id: project_name.to_string(),
                    version,
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

use curseforge;
use types::{ModSource, ModpackConfig};

pub fn check(
    target_game_version: semver::VersionReq,
    pack_path: String,
    mut pack: ModpackConfig,
) -> impl Future<Item = (), Error = ::Error> + Send + 'static {
    let http_client = HttpSimple::new();

    let strm = futures::stream::futures_unordered(update_project_names(pack.mods.clone()))
        .and_then(move |modd|{
            let target_game_version = target_game_version.clone();
            let http_client_handle = http_client.clone();
            async_block!{
                match modd{
                    ModSource::CurseforgeMod(curse_mod) => {
                        let found = await!(find_most_recent(curse_mod.id.clone(),
                                            target_game_version,
                                            http_client_handle,
                                            ReleaseStatus::Alpha))?;
                        if let Some(found) = found {
                            format_colored!((*COLOR_OUTPUT); (&SUCCESS_COLOR){"  COMPATIBLE: "}, "{}", curse_mod.id );
                            assert_eq!(curse_mod.id, found.id);
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

    async_block!{

        let modlist: Vec<(ModSource,Option<ReleaseStatus>)> = await!(strm.collect())?;

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
                let mut new_name = String::new();
                std::io::stdin().read_line(&mut new_name).expect("Failed to read pack name. Is terminal broken?");
                let new_name = new_name.trim();

                if !new_name.is_empty(){
                    pack.name = new_name.to_owned();
                }

                pack.mods = compatible;

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

pub fn run(
    target_game_version: semver::VersionReq,
    pack_path: String,
    mut pack: ModpackConfig,
    release_status: ReleaseStatus,
) -> impl Future<Item = (), Error = ::Error> + Send + 'static {
    let http_client = HttpSimple::new();

    async_block!{
        let mut new_mods = vec![];
        //FIXME: ideally we would borrow pack.mods to iterate over it, but for now we can't due to
        //       borrow tracing limitations in generators
        let old_mods = await!(futures::future::join_all(update_project_names(pack.mods.clone())))?;
        for modd in old_mods{
            let updated = match modd {
                ModSource::CurseforgeMod(curse_mod) => {
                    let found = await!(find_most_recent(curse_mod.id.clone(),
                                            target_game_version.clone(),
                                            http_client.clone(),
                                            release_status))?;
                    if let Some(found) = found {
                        assert_eq!(curse_mod.id, found.id);
                        if found.version > curse_mod.version {
                            let prompt = format!("Replace {} {} with {} ({})?",
                                curse_mod.id,
                                curse_mod.version,
                                found.version,
                                found.file_name);
                            if prompt_yes_no(prompt,Response::Yes) == Response::Yes {
                                Some(ModSource::CurseforgeMod(curseforge::Mod {
                                    id: found.id,
                                    version: found.version,
                                }))
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
