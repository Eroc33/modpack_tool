//#![allow(redundant_closure)]
use download::{self, DownloadManager};
use futures::future;
use futures::prelude::*;
use hash_writer::HashWriter;
use http::Uri;
use slog::Logger;
use std::fs::{self, File};
use std::io::Cursor;
use std::io::prelude::*;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use util;
use cache::{Cache, Cacheable};

const CACHE_DIR: &str = "./mvn_cache/";

#[derive(Debug)]
pub enum VerifyResult {
    Good,
    Bad,
    NotInCache,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MavenArtifact {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedArtifact {
    pub artifact: MavenArtifact,
    pub repo: Uri,
}

pub struct MavenCache;

impl Cacheable for ResolvedArtifact {
    type Cache = ::cache::FileCache;
    fn cached_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(CACHE_DIR);
        p.push(&self.artifact.to_path());
        p
    }
    fn uri(&self) -> ::download::Result<Uri> {
        self.artifact.get_uri_on(&self.repo)
    }
}

impl Cache<ResolvedArtifact> for MavenCache {
    fn with(
        artifact: ResolvedArtifact,
        manager: DownloadManager,
        log: Logger,
    ) -> download::BoxFuture<PathBuf> {
        let cached_path = artifact.cached_path();
        let log = log.new(
            o!("artifact"=>artifact.artifact.to_string(),"repo"=>artifact.repo.to_string(),"cached_path"=>cached_path.as_path().to_string_lossy().into_owned()),
        );
        info!(log, "caching maven artifact");
        if Self::is_cached(&artifact) {
            info!(log, "artifact was already cached");
            Box::new(future::ok(cached_path))
        } else {
            info!(log, "artifact is not cached, downloading now");
            match artifact.uri() {
                Ok(url) => Box::new(
                    manager
                        .download(url, cached_path.clone(), false, &log)
                        .map(move |_| cached_path),
                ),
                Err(e) => Box::new(future::err(e)),
            }
        }
    }
}

impl MavenCache {
    pub fn verify_cached(
        resolved: &ResolvedArtifact,
        manager: DownloadManager,
    ) -> download::BoxFuture<VerifyResult> {
        if Self::is_cached(&resolved) {
            let cached_path = resolved.cached_path();
            let sha_url_res = resolved.sha_uri();
            Box::new(async_block!{
                let mut cached_file = File::open(cached_path)?;

                let mut sha = HashWriter::new();
                ::std::io::copy(&mut cached_file, &mut sha)?;
                let cached_sha = sha.digest();

                let sha_uri = sha_url_res?;
                let (res,_) = self::await!(manager.get(sha_uri)?)?;
                let hash_str = self::await!(res.into_body()
                    .map_err(download::Error::from)
                    .fold(String::new(),
                            |mut buf, chunk| -> Result<String, download::Error> {
                                Cursor::new(chunk).read_to_string(&mut buf)?;
                                Ok(buf)
                            }))?;
                if hash_str == format!("{}", cached_sha) {
                    Ok(VerifyResult::Good)
                } else {
                    Ok(VerifyResult::Bad)
                }
            })
        } else {
            Box::new(future::ok(VerifyResult::NotInCache)) as download::BoxFuture<VerifyResult>
        }
    }
}

impl MavenArtifact {
    fn to_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(&self.group_path());
        p.push(&self.artifact);
        p.push(&self.version);
        p.push(&self.artifact_filename());
        p
    }

    pub fn get_uri_on(&self, base: &Uri) -> ::download::Result<Uri> {
        let base = ::util::uri_to_url(base)?;
        let path = self.to_path();
        let url = base.join(path.to_str().expect("non unicode path encountered"))?;
        ::util::url_to_uri(&url)
    }

    fn group_path(&self) -> PathBuf {
        PathBuf::from_iter(self.group.split('.'))
    }

    fn artifact_filename(&self) -> String {
        let classifier_fmt = match self.classifier {
            Some(ref class) => format!("-{classifier}", classifier = class),
            None => "".to_string(),
        };
        let extension_fmt = match self.extension {
            Some(ref extension) => extension.clone(),
            None => "jar".to_string(),
        };
        format!(
            "{artifact}-{version}{classifier}.{extension}",
            artifact = self.artifact,
            version = self.version,
            classifier = classifier_fmt,
            extension = extension_fmt
        )
    }

    pub fn resolve(&self, repo_uri: Uri) -> ResolvedArtifact {
        ResolvedArtifact {
            artifact: self.clone(),
            repo: repo_uri,
        }
    }

    pub fn download_from(
        &self,
        location: &Path,
        repo_uri: Uri,
        manager: DownloadManager,
        log: Logger,
    ) -> impl Future<Item = (), Error = ::download::Error> + Send {
        MavenCache::install_at(self.resolve(repo_uri), location.to_owned(), manager, log)
    }
}

impl ResolvedArtifact {
    pub fn to_path(&self) -> PathBuf {
        self.artifact.to_path()
    }
    pub fn sha_uri(&self) -> ::download::Result<Uri> {
        let mut url = ::util::uri_to_url(&self.uri()?)?;
        let mut path = url.path().to_owned();
        path.push_str(".sha1");
        url.set_path(path.as_ref());
        ::util::url_to_uri(&url)
    }
    pub fn install_at_no_classifier(
        self,
        mut location: PathBuf,
        manager: DownloadManager,
        log: Logger,
    ) -> impl Future<Item = (), Error = ::download::Error> + Send {
        <Self as Cacheable>::Cache::with(self.clone(), manager, log.clone()).and_then(
            move |cached_path| {
                let ResolvedArtifact { artifact, repo } = self;
                let log = log.new(o!("artifact"=>artifact.to_string(),"repo"=>repo.to_string()));
                info!(log, "installing maven artifact");

                let cached_path_no_classifier = ResolvedArtifact {
                    artifact: MavenArtifact {
                        classifier: None,
                        ..artifact.clone()
                    },
                    repo,
                }.cached_path();

                fs::create_dir_all(location.to_owned())?;

                if let Some(name) = cached_path_no_classifier.file_name() {
                    location.push(name);
                }
                match util::symlink(cached_path, location, &log) {
                    Err(util::SymlinkError::Io(ioe)) => return Err(ioe.into()),
                    Err(util::SymlinkError::AlreadyExists) => {
                        //TODO: verify the file, and replace/redownload it if needed
                        warn!(log, "File already exist, assuming content is correct");
                    }
                    Ok(_) => {}
                }
                Ok(())
            },
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MavenArtifactParseError {
    BadNumberOfParts,
}

impl ToString for MavenArtifact {
    fn to_string(&self) -> String {
        let mut strn = String::new();
        strn.push_str(&self.group);
        strn.push(':');
        strn.push_str(&self.artifact);
        strn.push(':');
        strn.push_str(&self.version);
        if let Some(ref classifier) = self.classifier {
            strn.push(':');
            strn.push_str(classifier);
        }
        if let Some(ref ext) = self.extension {
            strn.push('@');
            strn.push_str(ext);
        }
        strn
    }
}

impl FromStr for MavenArtifact {
    type Err = MavenArtifactParseError;
    fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('@').into_iter().collect();
        let (s, ext): (&str, Option<String>) = match *parts.as_slice() {
            [s, ext] => (s, Some(ext.to_string())),
            _ => (s, None),
        };

        let parts = s.split(':');
        let parts: Vec<&str> = parts.into_iter().collect();
        match *parts.as_slice() {
            [grp, art, ver] => Ok(MavenArtifact {
                group: grp.into(),
                artifact: art.into(),
                version: ver.into(),
                classifier: None,
                extension: ext,
            }),
            [grp, art, ver, class] => Ok(MavenArtifact {
                group: grp.into(),
                artifact: art.into(),
                version: ver.into(),
                classifier: Some(class.into()),
                extension: ext,
            }),
            _ => Err(MavenArtifactParseError::BadNumberOfParts),
        }
    }
}

#[cfg(test)]
mod test {
    use super::MavenArtifact;
    #[test]
    fn parses_simple() {
        assert_eq!(
            "net.minecraftforge.forge:some-jar:some-version".parse(),
            Ok(MavenArtifact {
                group: "net.minecraftforge.forge".into(),
                artifact: "some-jar".into(),
                version: "some-version".into(),
                classifier: None,
                extension: None,
            })
        )
    }
    #[test]
    fn parses_with_ext() {
        assert_eq!(
            "net.minecraftforge.forge:some-jar:some-version@zip".parse(),
            Ok(MavenArtifact {
                group: "net.minecraftforge.forge".into(),
                artifact: "some-jar".into(),
                version: "some-version".into(),
                classifier: None,
                extension: Some("zip".into()),
            })
        )
    }
    #[test]
    fn parses_with_classifier() {
        assert_eq!(
            "net.minecraftforge.forge:some-jar:some-version:universal".parse(),
            Ok(MavenArtifact {
                group: "net.minecraftforge.forge".into(),
                artifact: "some-jar".into(),
                version: "some-version".into(),
                classifier: Some("universal".into()),
                extension: None,
            })
        )
    }
    #[test]
    fn parses_with_ext_and_classifier() {
        assert_eq!(
            "net.minecraftforge.forge:some-jar:some-version:universal@zip".parse(),
            Ok(MavenArtifact {
                group: "net.minecraftforge.forge".into(),
                artifact: "some-jar".into(),
                version: "some-version".into(),
                classifier: Some("universal".into()),
                extension: Some("zip".into()),
            })
        )
    }
}
