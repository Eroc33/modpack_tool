#![feature(conservative_impl_trait,never_type,generators)]

extern crate serde_json;
#[macro_use]
extern crate scan_rules;
#[macro_use]
extern crate modpack_tool;
extern crate kuchiki;
extern crate url;
extern crate tokio_core;
extern crate futures_await as futures;
extern crate regex;
extern crate semver;
#[macro_use]
extern crate lazy_static;
extern crate termcolor;
//FIXME: has_class in kuchiki should probably not require selectors to be imported
//       maybe file a bug for this
extern crate selectors;

use futures::future;
use futures::prelude::*;
use kuchiki::{NodeDataRef, ElementData};
use kuchiki::traits::TendrilSink;
use modpack_tool::download::HttpSimple;
use modpack_tool::types::ReleaseStatus;
use scan_rules::scanner::Everything;
use scan_rules::scanner::runtime::until_pat_a;
use std::borrow::Borrow;
use std::io::Cursor;
use std::sync::Arc;
use tokio_core::reactor::{self,Core};
use url::Url;
use regex::Regex;

use termcolor::{ColorSpec,WriteColor};
use termcolor::Color::*;
use std::io::Write;

lazy_static!{
    pub static ref COLOR_OUTPUT: Arc<termcolor::BufferWriter> = Arc::new(termcolor::BufferWriter::stdout(termcolor::ColorChoice::Always));
    pub static ref INFO_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Cyan)).set_bold(true).set_intense(true);
        spec
    };
    pub static ref WARN_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Yellow)).set_bold(true).set_intense(true);
        spec
    };
    pub static ref SUCCESS_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Green)).set_bold(true).set_intense(true);
        spec
    };
    pub static ref FAILURE_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(Red)).set_bold(true).set_intense(true);
        spec
    };
    pub static ref DEFAULT_COLOR: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_fg(Some(White));
        spec
    };
}

#[derive(Debug)]
pub struct ModVersionInfo {
    id: String,
    version: u64,
    file_name: String,
    download_url: Url,
    release_status: ReleaseStatus,
    game_versions: Vec<semver::Version>,
}

#[derive(PartialEq,Eq)]
pub enum Response{
    Yes,
    No,
}

pub fn prompt_yes_no(default: Response) -> Response{
    readln!{
        (r"(?i)Y|Yes") => Response::Yes,
        (r"(?i)N|No") => Response::No,
        (.._) => default,
    }
}

pub fn prompt_release_status(default: ReleaseStatus) -> ReleaseStatus{
    readln!{
        (r"(?i)A|Alpha") => ReleaseStatus::Alpha,
        (r"(?i)B|Beta") => ReleaseStatus::Beta,
        (r"(?i)R|Release") => ReleaseStatus::Release,
        (.._) => default,
    }
}

pub fn extract_version_and_id(url: &str) -> (u64, String) {
    let res = scan!{url;
        ("https://minecraft.curseforge.com/projects/",let project <| until_pat_a::<Everything<String>,&str>("/"),"/files/",let ver, ["/download"]?) => (ver,project),
    };
    res.expect("Unknown modsource url")
}

pub fn get_attr<N>(node: N, name: &str) -> Option<String>
    where N: Borrow<NodeDataRef<ElementData>>
{
    node.borrow()
        .attributes
        .borrow()
        .get(name)
        .map(|s| s.to_owned())
}

pub fn find_most_recent
    (project_name: String,
     target_game_version: semver::VersionReq,
     http_client: HttpSimple,
     target_release_status: ReleaseStatus)
     -> impl Future<Item = Option<ModVersionInfo>, Error = modpack_tool::Error>{

    lazy_static!{
        static ref TITLE_REGEX: Regex = regex::Regex::new("(<div>)|(</div><div>)|(</div>)").expect("Couldn't compie pre-checked regex");
    }

    const BASE_URL: &'static str = "https://minecraft.curseforge.com";
    let base_url = Url::parse(BASE_URL).unwrap();
    let scrape_url = base_url.join(&format!("/projects/{}/files", project_name)).unwrap();
    async_block!{
        let uri = modpack_tool::util::url_to_uri(&scrape_url)?;
        let body = await!(http_client.get(uri)
                .and_then(move |res| {
                    res.body().fold(vec![],
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
            .map_err(|_| modpack_tool::ErrorKind::Selector)?;
        for row in rows {
            let row = row.as_node();
            let release_status =
                get_attr(row.select(".project-file-release-type div")
                                .map_err(|_| modpack_tool::ErrorKind::Selector)?
                                .next()
                                .unwrap(),
                            "title");
            let files_cell = row.select(".project-file-name div")
                .map_err(|_| modpack_tool::ErrorKind::Selector)?
                .next()
                .unwrap();
            let file_name = files_cell.as_node()
                .select(".project-file-name-container .overflow-tip")
                .map_err(|_| modpack_tool::ErrorKind::Selector)?
                .next()
                .unwrap()
                .text_contents();
            //let more_files_url = file_name_container.attr("href");
            let primary_file =
                get_attr(files_cell.as_node()
                                .select(".project-file-download-button a")
                                .map_err(|_| modpack_tool::ErrorKind::Selector)?
                                .next()
                                .unwrap(),
                            "href");
            let version_container = row.select(".project-file-game-version")
                .map_err(|_| modpack_tool::ErrorKind::Selector)?
                .next()
                .unwrap();
            let mut game_versions: Vec<semver::Version> = vec![];
            use selectors::Element;
            if version_container.has_class(&("multiple".into())){
                let additional_versions = version_container.as_node().select(".additional-versions")
                    .map_err(|_| modpack_tool::ErrorKind::Selector)?
                    .next()
                    .unwrap();
                let cell_ref = additional_versions.attributes.borrow();
                if let Some(title) = cell_ref.get("title"){
                    for version in TITLE_REGEX.split(title.as_ref()){
                        if !(version.starts_with("Java") || version.starts_with("java")) && !version.is_empty(){
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
                .map_err(|_| modpack_tool::ErrorKind::Selector)?
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

            if release_status.map(|status| target_release_status.accepts(&status)).unwrap_or(false) {
                if game_versions.iter().any(|ver| target_game_version.matches(ver)){
                    let url = primary_file.map(|s| base_url.join(&s).unwrap()).unwrap();
                    let (version, _) = extract_version_and_id(url.as_str());
                    return Ok(Some(ModVersionInfo {
                        id: project_name.to_string(),
                        version: version,
                        file_name: file_name,
                        download_url: url,
                        release_status: release_status.unwrap(),
                        game_versions,
                    }));
                }
            }
        }
        Ok(None)
    }
}

use modpack_tool::curseforge;
use modpack_tool::types::{ModSource, ModpackConfig};
use std::env;

fn check_mc_version_compat(target_game_version: semver::VersionReq, pack: ModpackConfig, handle: reactor::Handle) -> impl Future<Item=(),Error=modpack_tool::Error> + 'static{
    let http_client = HttpSimple::new(&handle);

    let check_futures:Vec<_> = pack.mods.clone().into_iter()
        .map(|modd| match modd {
            ModSource::CurseforgeMod(curse_mod) => {
                let http_client_handle = http_client.clone();
                let captured_target_game_version = target_game_version.clone();
                Box::new(
                    async_block!{
                        let found = await!(find_most_recent(curse_mod.id.clone(),
                                          captured_target_game_version.clone(),
                                          http_client_handle,
                                          ReleaseStatus::Alpha))?;
                        if let Some(found) = found {
                            let mut buf = (*COLOR_OUTPUT).buffer();
                            buf.set_color(&SUCCESS_COLOR);
                            write!(buf,"  COMPATIBLE: ");
                            buf.set_color(&DEFAULT_COLOR);
                            write!(buf,"{}", curse_mod.id);
                            assert_eq!(curse_mod.id, found.id);
                            if found.release_status != ReleaseStatus::Release {
                                let a_an = if found.release_status == ReleaseStatus::Alpha{
                                    "an"
                                }else if found.release_status == ReleaseStatus::Beta{
                                    "a"
                                }else{
                                    unreachable!("Status was not release, alpha, or beta");
                                };
                                buf.set_color(&INFO_COLOR);
                                writeln!(buf," (as {} {} release)",a_an,found.release_status.value());
                                buf.set_color(&DEFAULT_COLOR);
                            }else{
                                writeln!(buf,"");
                            }
                            (*COLOR_OUTPUT).print(&buf);
                            Ok((curse_mod.into(),Some(found.release_status)))
                        } else {
                            let mut buf = (*COLOR_OUTPUT).buffer();
                            buf.set_color(&FAILURE_COLOR);
                            write!(buf,"INCOMPATIBLE: ");
                            buf.set_color(&DEFAULT_COLOR);
                            writeln!(buf,"{}", curse_mod.id);
                            (*COLOR_OUTPUT).print(&buf);
                            Ok((curse_mod.into(),None))
                        }
                    }
                    
                ) as
                Box<Future<Item = (ModSource,Option<ReleaseStatus>), Error = modpack_tool::Error>>
            }
            ModSource::MavenMod { artifact, repo } => {
                let mut buf = (*COLOR_OUTPUT).buffer();
                buf.set_color(&WARN_COLOR);
                writeln!(buf,"you must check maven mod: {:?}",artifact);
                buf.set_color(&DEFAULT_COLOR);
                (*COLOR_OUTPUT).print(&buf);
                Box::new(future::ok((ModSource::MavenMod { artifact, repo },None)))
            }
        }).collect();

    async_block!{
        let mut total = 0usize;
        let mut alpha_compatible = 0usize;
        let mut beta_compatible = 0usize;
        let mut compatible = 0usize;
        let mut incompatible = vec![];

        let strm = futures::stream::futures_unordered(check_futures);

        #[async]
        for (modd,status) in strm{
            total += 1;
            match status{
                None => incompatible.push(modd),
                Some(ReleaseStatus::Alpha) => {
                    compatible += 1;
                    alpha_compatible += 1;
                }
                Some(ReleaseStatus::Beta) => {
                    compatible += 1;
                    beta_compatible += 1;
                }
                Some(ReleaseStatus::Release) => {
                    compatible += 1;
                }
            }
        }

        if incompatible.len() == 0{
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
            println!("Upgrade now? [Y/n]");
            if prompt_yes_no(Response::Yes) == Response::Yes{
                println!("Enter new pack name:");
                let mut new_name = String::new();
                std::io::stdin().read_line(&mut new_name).expect("Failed to read pack name. Is terminal broken?");
                match min_required_status {
                    ReleaseStatus::Alpha if pack_update_status != ReleaseStatus::Alpha => {
                        println!("This will mean your pack must use alpha status mods. Is this ok? [y/N]");
                        if prompt_yes_no(Response::No) == Response::No{
                            println!("Canceling upgrade");
                            return Ok(());
                        }
                    },
                    ReleaseStatus::Beta if pack_update_status != ReleaseStatus::Beta => {
                        println!("This will mean your pack must use beta status mods. Is this ok? [y/N]");
                        if prompt_yes_no(Response::No) == Response::No{
                            println!("Canceling upgrade");
                            return Ok(());
                        }
                    },
                    _ => {}
                }
                await!(do_update(new_name,pack,min_required_status,handle))?;
                return Ok(())
            }
        }else{
            let percent_compatible = (compatible as f32)/(total as f32) * 100.0;
            let mut buf = (*COLOR_OUTPUT).buffer();
            buf.set_color(&INFO_COLOR);
            writeln!(buf,"{:.1}% of your mods are compatible.",percent_compatible);
            writeln!(buf,"You must remove or replace incompatible mods before you can upgrade.");
            writeln!(buf,"{} incompatible mods:",incompatible.len());
            buf.set_color(&WARN_COLOR);
            for modd in incompatible{
                writeln!(buf,"\t {} ( {} )",modd.identifier_string(),modd.guess_project_url().unwrap_or_else(|| "COULD NOT GUESS PROJECT URL".to_owned()));
            }
            buf.set_color(&DEFAULT_COLOR);
            (*COLOR_OUTPUT).print(&buf);
        }
        Ok(())
    }
}

fn do_update(pack_path: String, mut pack: ModpackConfig, release_status: ReleaseStatus, handle: reactor::Handle) -> impl Future<Item=(),Error=modpack_tool::Error> + 'static{
    let http_client = HttpSimple::new(&handle);

    let target_version = pack.version.clone();

    async_block!{
        let mut new_mods = vec![];
        //FIXME: ideally we would borrow pack.mods to iterate over it, but for now we can't due to
        //       borrow tracing limitations in generators
        let old_mods = pack.mods.clone();
        for modd in old_mods{
            let updated = match modd {
                ModSource::CurseforgeMod(curse_mod) => {
                    let found = await!(find_most_recent(curse_mod.id.clone(),
                                            target_version.clone(),
                                            http_client.clone(),
                                            release_status))?;
                    if let Some(found) = found {
                        assert_eq!(curse_mod.id, found.id);
                        if found.version > curse_mod.version {
                            print!("Replace {} {} with {} ({})? [Y/n]",
                                curse_mod.id,
                                curse_mod.version,
                                found.version,
                                found.file_name);
                            if prompt_yes_no(Response::Yes) == Response::Yes {
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
        for modsource in new_mods.into_iter() {
            pack.replace_mod(modsource);
        }

        let mut file = std::fs::File::create(pack_path).expect("pack does not exist");
        serde_json::ser::to_writer_pretty(&mut file, &pack)?;
        Ok(())
    }
}

fn main() {
    let mut core = Core::new().expect("Failed to start tokio");
    let pack_path = env::args().nth(1).expect("expected pack as first arg");

    let file = std::fs::File::open(&pack_path).expect("pack does not exist");
    let pack: ModpackConfig = serde_json::de::from_reader(file)
        .expect("pack file in bad format");

    let release_status =
        pack.auto_update_release_status.unwrap_or_else(|| {
            die!("Pack must have an auto_update_release_status to be able to auto update")
        });

    match env::args().nth(2).map(|ver|{
        if ver.chars().next().expect("Argument with 0 length?").is_numeric(){
            //view a versionreq of x as ~x
            println!("Interpreting version {} as ~{}",ver,ver);
            format!("~{}",ver)
        }else{
            ver
        }
    }).map(|ver| semver::VersionReq::parse(ver.as_str()).unwrap_or_else(|_| die!("Second argument must be a semver version requirement"))){
        Some(target_ver) => {
            let fut = check_mc_version_compat(target_ver,pack,core.handle());
            core.run(fut).unwrap()
        }
        None => {
            let fut = do_update(pack_path,pack,release_status,core.handle());
            core.run(fut).unwrap()
        }
    };
}
