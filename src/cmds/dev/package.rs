use std::{
    path::PathBuf,
    io::{Write,Seek},
};
use zip;
use structopt::StructOpt;
use tokio::io::AsyncReadExt;

#[derive(Debug, StructOpt)]
#[structopt(name = "add", about = "Adds a mod to the provided pack file.")]
pub struct Args{
    /// The metadata json file for the pack you wish to package
    pack_file: PathBuf,
    /// The path to create the packaged one-click installer at
    oneclick_path: PathBuf,
}

pub async fn package(args: Args) -> Result<(),crate::Error>
{
    let Args{pack_file, oneclick_path} = args;

    //copy this executable
    let own_path = std::env::args().nth(0).expect("arg 0 should always be available");
    crate::util::fs_copy(own_path, oneclick_path.clone()).await?;

    //load the pack contents asyncronously
    let mut pack_config = tokio::fs::File::open(pack_file).await?;
    let mut pack_config_contents = vec![];
    pack_config.read_to_end(&mut pack_config_contents).await?;

    //then append zip to it
    let mut file  = std::fs::OpenOptions::new().write(true).truncate(false).open(oneclick_path)?;
    file.seek(std::io::SeekFrom::End(0))?;
    let mut tmp = std::io::Cursor::new(vec![]);
    {
        let mut writer = zip::write::ZipWriter::new(&mut tmp);
        //with the relevant file inside as "config.json"
        writer.start_file("config.json",zip::write::FileOptions::default())?;
        writer.write_all(&pack_config_contents[..])?;
        writer.finish()?;
    }
    file.write_all(tmp.into_inner().as_slice())?;
    Ok(())
}
