use crate::error::prelude::*;
use std::{
    path::PathBuf,
    io::{Write,Seek},
};
use zip;
use structopt::StructOpt;
use tokio::io::AsyncReadExt;
use snafu::Snafu;

#[derive(Debug, StructOpt)]
#[structopt(name = "add", about = "Adds a mod to the provided pack file.")]
pub struct Args{
    /// The metadata json file for the pack you wish to package
    pack_file: PathBuf,
    /// The path to create the packaged one-click installer at
    oneclick_path: PathBuf,
}

#[derive(Debug,Snafu)]
pub enum Error{
    #[snafu(display("file {} does not exist",path))]
    OpeningFile{
        path: String,
        source: std::io::Error,
    },
    #[snafu(display("io error {} while reading pack file at {}", source, path))]
    ReadingPackfile{
        path: String,
        source: std::io::Error,
    },
    #[snafu(display("io error {} while creating pack file at {}", source, path))]
    CreatingHybridPackfile{
        path: String,
        source: std::io::Error,
    }
}

#[derive(Debug,Snafu)]
enum HybridPackfileError{
    #[snafu(display("io error {} while creating hybrid zip in internal buffer", source))]
    Io{
        source: std::io::Error,
    },
    #[snafu(display("zip error {} while creating hybrid zip in internal buffer", source))]
    Zip{
        source: zip::result::ZipError,
    }
}

pub async fn package(args: Args) -> Result<(),crate::Error>
{
    let Args{pack_file, oneclick_path} = args;

    //copy this executable
    let own_path = std::env::args().nth(0).expect("arg 0 should always be available");
    crate::util::fs_copy(own_path, oneclick_path.clone()).await.context(CreatingHybridPackfile{path: oneclick_path.display().to_string()}).erased()?;

    //load the pack contents asyncronously
    let mut pack_config = tokio::fs::File::open(pack_file.clone()).await.context(OpeningFile{path: pack_file.display().to_string()}).erased()?;
    let mut pack_config_contents = vec![];
    pack_config.read_to_end(&mut pack_config_contents).await.context(ReadingPackfile{path: pack_file.display().to_string()}).erased()?;

    //then append zip to it
    let mut file  = std::fs::OpenOptions::new().write(true).truncate(false).open(oneclick_path.clone()).context(OpeningFile{path: oneclick_path.display().to_string()}).erased()?;
    file.seek(std::io::SeekFrom::End(0)).context(CreatingHybridPackfile{path: oneclick_path.display().to_string()}).erased()?;
    let mut tmp = std::io::Cursor::new(vec![]);
    {
        let mut writer = zip::write::ZipWriter::new(&mut tmp);
        //with the relevant file inside as "config.json"
        writer.start_file("config.json",zip::write::FileOptions::default()).context(Zip).erased()?;
        writer.write_all(&pack_config_contents[..]).context(Io).erased()?;
        writer.finish().context(Zip).erased()?;
    }
    file.write_all(tmp.into_inner().as_slice()).context(CreatingHybridPackfile{path: oneclick_path.display().to_string()}).erased()?;
    Ok(())
}
