#[macro_use]
extern crate scan_rules;
extern crate serde_json;

extern crate modpack_tool;

use modpack_tool::types::{ModSource, ModpackConfig};
use scan_rules::input::IntoScanCursor;

use scan_rules::scanner::Everything;
use scan_rules::scanner::runtime::until_pat_a;

use std::env;

fn main() {
    let pack_path = env::args().nth(1).expect("expected pack as first arg");
    let to_add = env::args().nth(2).expect("expected url of mod to add as second arg");
    let to_add = to_add.into_scan_cursor();

    let file = std::fs::File::open(&pack_path).expect("pack does not exist");
    let mut pack: ModpackConfig = serde_json::de::from_reader(file)
        .expect("pack file in bad format");

    let modsource = scan!{to_add;
    ("https://minecraft.curseforge.com/projects/",let project <| until_pat_a::<Everything<String>,&str>("/"),"/files/",let ver, ["/download"]?) => ModSource::CurseforgeMod{id:project,version:ver},
    (.._) => panic!("Unknown modsource url"),
  }.expect("bad mod url input");

    match modsource {
        ModSource::CurseforgeMod { ref id, .. } => {
            let new_id = id;
            pack.mods = pack.mods
                .into_iter()
                .filter(|source| {
                    match *source {
                        ModSource::CurseforgeMod { ref id, ref version } => {
                            if id == new_id {
                                println!("removing old version ({})", version);
                                false
                            } else {
                                true
                            }
                        }
                        _ => true,
                    }
                })
                .collect();
        }
        _ => panic!("Other mod sources not yet supported"),
    }

    println!("Adding: {:?}", modsource);

    pack.mods.push(modsource);

    let mut file = std::fs::File::create(pack_path).expect("pack does not exist");
    serde_json::ser::to_writer_pretty(&mut file, &pack).unwrap();

}
