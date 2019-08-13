use futures::{
    prelude::*,
    Stream,StreamExt,
};
use std::{
    str::FromStr,
    pin::Pin,
};
use http::Uri;
use crate::{
    download::HttpSimple,
    curseforge::{self,ReleaseStatus},
};

trait SelectExt{
    fn select(&self,selector: &'static str) -> kuchiki::iter::Select<kuchiki::iter::Elements<kuchiki::iter::Descendants>>;
    fn select_first(&self,selector: &'static str) -> kuchiki::NodeDataRef<kuchiki::ElementData>{
        self.select(selector).next().unwrap()
    }
    fn get_attr(&self, name: &str) -> Option<String>;
}

impl SelectExt for kuchiki::NodeDataRef<kuchiki::ElementData>{
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

fn mc_version_to_curseforge_id(s: &str) -> Option<&'static str>{
    std::dbg!(s);
    Some(match s{
        "~1.0.0" => "180",
        "~1.1.0" => "186",
        "~1.2.1" => "201",
        "~1.2.2" => "202",
        "~1.2.3" => "203",
        "~1.2.5" => "204",
        "~1.3.1" => "241",
        "~1.3.2" => "246",
        "~1.4.2" => "255",
        "~1.4.6" => "268",
        "~1.4.7" => "272",
        "~1.5.0" => "279",
        "~1.5.1" => "280",
        "~1.5.2" => "312",
        "~1.6.1" => "318",
        "~1.6.2" => "320",
        "~1.6.4" => "326",
        "~1.7.2" => "361",
        "~1.7.4" => "367",
        "~1.8.0" => "531",
        "~1.8.1" => "532",
        "~1.8.3" => "568",
        "~1.9.0" => "585",
        "~1.10" => "591",
        "~1.11" => "630",
        "~1.7.5" => "4444",
        "~1.7.6" => "4445",
        "~1.7.7" => "4446",
        "~1.7.8" => "4447",
        "~1.7.9" => "4448",
        "~1.7.10" => "4449",
        "~1.8-Snapshot" => "4450",
        "~1.8.0" => "4455",
        "~1.4.4" => "4460",
        "~1.4.5" => "4461",
        "~1.2.4" => "4462",
        "~1.8.1" => "4463",
        "~1.8.2" => "4465",
        "~1.8.3" => "4466",
        "~1.8.4" => "4478",
        "~1.8.5" => "4479",
        "~1.8.6" => "4480",
        "~1.8.7" => "5642",
        "~1.8.8" => "5703",
        "~1.9-Snapshot" => "5707",
        "~1.8.9" => "5806",
        "~1.9.0" => "5946",
        "~1.9.2" => "5997",
        "~1.9.1" => "5998",
        "~1.9.4" => "6084",
        "~1.9.3" => "6085",
        "~1.10-Snapshot" => "6143",
        "~1.10.0" => "6144",
        "~1.10.1" => "6160",
        "~1.10.2" => "6170",
        "~1.11-Snapshot" => "6239",
        "~0.16.0" => "6298",
        "~1.11.0" => "6317",
        "~1.0.0" => "6373",
        "~1.1.0" => "6374",
        "~1.2.1" => "6375",
        "~1.2.2" => "6376",
        "~1.2.3" => "6377",
        "~1.2.5" => "6378",
        "~1.3.1" => "6379",
        "~1.3.2" => "6380",
        "~1.4.2" => "6381",
        "~1.4.6" => "6382",
        "~1.4.7" => "6383",
        "~1.5.0" => "6384",
        "~1.5.1" => "6385",
        "~1.5.2" => "6386",
        "~1.6.1" => "6387",
        "~1.6.2" => "6388",
        "~1.6.4" => "6389",
        "~1.7.2" => "6390",
        "~1.7.4" => "6391",
        "~1.11.1" => "6451",
        "~1.11.2" => "6452",
        "~1.12.0-Snapshot" => "6514",
        "~1.12.0" => "6580",
        "~1.12.0" => "6588",
        "~1.12.1" => "6711",
        "~1.12.2" => "6756",
        "~1.13.0-Snapshot" => "6834",
        "~0.16.1" => "6875",
        "~0.16.2" => "6876",
        "~1.0.0" => "6877",
        "~1.0.1" => "6878",
        "~1.0.2" => "6879",
        "~1.0.3" => "6880",
        "~1.2.8" => "6881",
        "~1.2.7" => "6882",
        "~1.2.6" => "6883",
        "~1.2.5" => "6884",
        "~1.2.3" => "6885",
        "~1.2.2" => "6886",
        "~1.2.1" => "6887",
        "~1.2.0" => "6888",
        "~1.0.4" => "6889",
        "~1.0.5" => "6890",
        "~1.0.6" => "6891",
        "~1.0.7" => "6892",
        "~1.0.8" => "6893",
        "~1.0.9" => "6894",
        "~1.1.0" => "6895",
        "~1.1.1" => "6896",
        "~1.1.2" => "6897",
        "~1.1.7" => "6898",
        "~1.13.0" => "7081",
        "~1.13.0" => "7105",
        "~1.13.1" => "7107",
        "~1.13.2" => "7132",
        "~1.14.0-Snapshot" => "7133",
        "~1.2.9" => "7134",
        "~1.2.10" => "7135",
        "~1.2.11" => "7136",
        "~1.2.13" => "7137",
        "~1.4.0" => "7138",
        "~1.5.0" => "7139",
        "~1.6.0" => "7140",
        "~1.7.0" => "7141",
        "~1.14.0" => "7318",
        "~1.14.0" => "7330",
        "~1.14.1" => "7344",
        "~1.7.1" => "7352",
        "~1.14.2" => "7361",
        "~1.14.3" => "7413",
        "~1.8.0" => "7430",
        "~1.9.0" => "7431",
        "~1.10.0" => "7432",
        "~1.11.0" => "7433",
        "~1.12.0" => "7434",
        "~1.13.0" => "7435",
        "~1.14.0" => "7436",
        "~1.14.4" => "7469",
        _ => return None,
    })
}

fn parse_files_url(url: &str) -> Result<u64,crate::Error>{
    complete!(
            &url,
            do_parse!(
                tag_s!("/minecraft/mc-mods/") >>
                _id: take_till_s!(|c: char| c == '/') >> tag_s!("/files/") >>
                version: map_res!(take_while_s!(|c: char| c.is_digit(10)), u64::from_str) >>
                (version)
            )
        ).to_full_result()
        .map_err(|_| crate::Error::BadModUrl {
            url: url.to_owned(),
        })
}

//TODO: replace with TryStreamExt::try_faltten when we can update to futures-preview-0.1.18
fn try_flatten_stream<'a,S,S2,E>(stream: S) -> impl Stream<Item=Result<S2::Item,E>> + Send + 'a
    where S: Stream<Item = Result<S2,E>> + Send + 'a,
          S2: Stream + Send + 'a,
          S2::Item: Send + 'a,
          E: Send + 'a,
{
    stream.then(move |inner_stream_res: Result<S2,E>| async move{
        match inner_stream_res{
            Ok(inner_stream) => {
                Box::pin(inner_stream.map(Ok)) as Pin<Box<dyn Stream<Item=_> + Send>>
            },
            Err(e) => {
                Box::pin(futures::stream::iter(vec![Err(e)]))
            }
        }
    }).flatten()
}

pub struct ReleaseInfo{
    pub release_status: ReleaseStatus,
    pub modd: curseforge::Mod,
}

pub fn all_for_version(
    curse_mod: curseforge::Mod,
    http_client: HttpSimple,
    target_game_version: semver::VersionReq,
) -> impl Stream<Item=Result<ReleaseInfo,crate::Error>> + Send {

    let page_num = 1;
    try_flatten_stream(futures::stream::unfold(Some(page_num),move |page_num|{
        let http_client = http_client.clone();
        let target_game_version = target_game_version.clone();
        let curse_mod = curse_mod.clone();
        async move{
            let page_num = if let Some(page_num) = page_num {
                page_num
            }else{
                return None;
            };
            let page = match page_for_version(curse_mod.clone(), http_client.clone(), target_game_version.clone(), page_num).await{
                Ok(page) => page,
                Err(e) => return Some((Err(e),None)),
            };
            if page.len() >= 25{
                //more pages
                Some((Ok(page), Some(page_num)))
            }else{
                //done
                Some((Ok(page),None))
            }
        }
    })
    .map_ok(futures::stream::iter))
    
}

async fn page_for_version(
    curse_mod: curseforge::Mod,
    http_client: HttpSimple,
    target_game_version: semver::VersionReq,
    page_num: u64,
) -> Result<Vec<ReleaseInfo>,crate::Error> {

    use std::io::Cursor;
    use kuchiki::traits::TendrilSink;

    let encoded_version = mc_version_to_curseforge_id(&target_game_version.to_string()).expect("bad game version");
    
    let all_url = format!("https://www.curseforge.com/minecraft/mc-mods/{}/files/all?filter-game-version=2020709689:{}&page={}",curse_mod.id,encoded_version,page_num);

    let body = http_client.get(Uri::from_str(&all_url)?)
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

    let mut mods = vec![];

    for row in rows {
        let release_status =
            row.select("td").nth(0).unwrap().select_first("span").text_contents();
        let files_cell = row.select("td").nth(1).unwrap();
        let file_name = files_cell.text_contents();
        let version = parse_files_url(&files_cell.select_first("a").get_attr("href").expect("missing link to file shouldn't be possible"))?;
        let primary_file = format!("https://www.curseforge.com/minecraft/mc-mods/{}/download/{}/file",curse_mod.id,version);

        mods.push(ReleaseInfo{release_status: ReleaseStatus::parse_short(&release_status).expect("Bad release status"),modd:curseforge::Mod{version,id: curse_mod.id.clone()}});
    }
    return Ok(mods);
}