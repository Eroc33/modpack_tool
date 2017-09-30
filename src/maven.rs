//#![allow(redundant_closure)]
use download::{self, Downloadable, DownloadManager};
use futures::future;
use futures::prelude::*;
use hash_writer::HashWriter;
use hyper::Uri;
use slog::Logger;
use std::fs::{self, File};
use std::io::Cursor;
use std::io::prelude::*;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use util;

const CACHE_DIR: &'static str = "./mvn_cache/";

#[derive(Debug)]
pub enum VerifyResult {
    Good,
    Bad,
    NotInCache,
}

#[derive(Serialize, Deserialize, Debug, Clone,PartialEq,Eq)]
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

struct MavenCache;

impl MavenCache {
    pub fn cached(artifact: &MavenArtifact) -> bool {
        Self::cached_path(artifact).exists()
    }

    pub fn with(artifact: &ResolvedArtifact,
                manager: DownloadManager,
                log: Logger)
                -> download::BoxFuture<PathBuf> {
        let cached_path = Self::cached_path(&artifact.artifact);
        let log = log.new(o!("artifact"=>artifact.artifact.to_string(),"repo"=>artifact.repo.to_string(),"cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));
        info!(log, "caching maven artifact");
        if Self::cached(&artifact.artifact) {
            info!(log, "artifact was already cached");
            Box::new(future::ok(cached_path))
        } else {
            info!(log, "artifact is not cached, downloading now");
            match artifact.uri() {
                Ok(url) => {
                    Box::new(manager.download(url, cached_path.clone(), false, log)
                        .map(move |_| cached_path))
                }
                Err(e) => Box::new(future::err(download::Error::from(e))),
            }
        }
    }

    fn cached_path(artifact: &MavenArtifact) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(CACHE_DIR);
        p.push(&artifact.to_path());
        p
    }

    pub fn verify_cached(resolved: &ResolvedArtifact,
                         manager: DownloadManager)
                         -> download::BoxFuture<VerifyResult> {
        if Self::cached(&resolved.artifact) {
            let cached_path = Self::cached_path(&resolved.artifact);
            let sha_url_res = resolved.sha_uri();
            Box::new(async_block!{
                let mut cached_file = File::open(cached_path)?;

                let mut sha = HashWriter::new();
                ::std::io::copy(&mut cached_file, &mut sha)?;
                let cached_sha = sha.digest();

                let sha_uri = sha_url_res?;
                let (res,_) = await!(manager.get(sha_uri)?)?;
                let hash_str = await!(res.body()
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

    // FIXME Annoying work around for the forge libs not really being where they claim to be
    fn install_at_no_classifier<'a>(artifact: ResolvedArtifact,
                                    mut location: PathBuf,
                                    manager: DownloadManager,
                                    log: Logger)
                                    -> impl Future<Item = (), Error = ::download::Error> + 'a {
        Self::with(&artifact, manager, log.clone()).and_then(move |cached_path| {
            let ResolvedArtifact { artifact, repo } = artifact;
            let log = log.new(o!("artifact"=>artifact.to_string(),"repo"=>repo.to_string()));
            info!(log, "installing maven artifact");

            let mut artifact_no_classifier = artifact.clone();
            artifact_no_classifier.classifier = None;
            let cached_path_no_classifier = Self::cached_path(&artifact_no_classifier);

            fs::create_dir_all(location.to_owned())?;

            cached_path_no_classifier.file_name().map(|n| location.push(n));
            util::symlink(cached_path, location, &log)?;
            Ok(())
        })
    }

    fn install_at<'a>(artifact: ResolvedArtifact,
                      mut location: PathBuf,
                      manager: DownloadManager,
                      log: Logger)
                      -> impl Future<Item = (), Error = ::download::Error> + 'a {
        Self::with(&artifact, manager, log.clone()).and_then(move |cached_path| {
            let ResolvedArtifact { artifact, repo } = artifact;
            let log = log.new(o!("artifact"=>artifact.to_string(),"repo"=>repo.to_string()));
            info!(log, "installing maven artifact");

            fs::create_dir_all(location.to_owned())?;

            cached_path.file_name().map(|n| location.push(n));
            util::symlink(cached_path, location, &log)?;
            Ok(())
        })
    }
}

impl MavenArtifact {
    fn to_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(&self.group_path());
        p.push(&self.artifact);
        p.push(&self.version);
        p.push(&self.artifact_filename());
        p.into()
    }

    pub fn get_uri_on(&self, base: &Uri) -> ::download::Result<Uri> {
        let base = ::util::uri_to_url(base)?;
        let path = self.to_path();
        let url = base.join(path.to_str().expect("non unicode path encountered"))?;
        ::util::url_to_uri(&url)
    }

    fn group_path(&self) -> PathBuf {
        PathBuf::from_iter(self.group.split('.')).into()
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
        format!("{artifact}-{version}{classifier}.{extension}",
                artifact = self.artifact,
                version = self.version,
                classifier = classifier_fmt,
                extension = extension_fmt)
    }

    pub fn resolve(&self, repo_uri: Uri) -> ResolvedArtifact {
        ResolvedArtifact {
            artifact: self.clone(),
            repo: repo_uri,
        }
    }

    pub fn download_from<'a>(&self,
                             location: &Path,
                             repo_uri: Uri,
                             manager: DownloadManager,
                             log: Logger)
                             -> impl Future<Item = (), Error = ::download::Error> + 'a {
        MavenCache::install_at(self.resolve(repo_uri), location.to_owned(), manager, log)
    }
}

impl ResolvedArtifact {
    pub fn to_path(&self) -> PathBuf {
        self.artifact.to_path()
    }
    pub fn uri(&self) -> ::download::Result<Uri> {
        self.artifact.get_uri_on(&self.repo)
    }
    pub fn sha_uri(&self) -> ::download::Result<Uri> {
        let mut url = ::util::uri_to_url(&self.uri()?)?;
        let mut path = url.path().to_owned();
        path.push_str(".sha1");
        url.set_path(path.as_ref());
        ::util::url_to_uri(&url)
    }
    pub fn install_at(&self,
                      location: &Path,
                      manager: DownloadManager,
                      log: Logger)
                      -> download::BoxFuture<()> {
        Box::new(MavenCache::install_at((*self).clone(), location.to_owned(), manager, log))
    }
    pub fn reader(&self,
                  manager: DownloadManager,
                  log: Logger)
                  -> download::BoxFuture<::std::fs::File> {
        Box::new(MavenCache::with(self, manager, log)
            .and_then(move |path| Ok(::std::fs::File::open(path)?)))
    }
    pub fn install_at_no_classifier(&self,
                                    location: &Path,
                                    manager: DownloadManager,
                                    log: Logger)
                                    -> download::BoxFuture<()> {
        Box::new(MavenCache::install_at_no_classifier((*self).clone(),
                                                      location.to_owned(),
                                                      manager,
                                                      log))
    }
}

impl Downloadable for ResolvedArtifact {
    fn download(self,
                location: PathBuf,
                manager: DownloadManager,
                log: Logger)
                -> download::BoxFuture<()> {
        Box::new(MavenCache::install_at(self, location, manager, log))
    }
}

#[derive(Debug,PartialEq,Eq)]
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
            [grp, art, ver] => {
                Ok(MavenArtifact {
                    group: grp.into(),
                    artifact: art.into(),
                    version: ver.into(),
                    classifier: None,
                    extension: ext,
                })
            }
            [grp, art, ver, class] => {
                Ok(MavenArtifact {
                    group: grp.into(),
                    artifact: art.into(),
                    version: ver.into(),
                    classifier: Some(class.into()),
                    extension: ext,
                })
            }
            _ => Err(MavenArtifactParseError::BadNumberOfParts),
        }
    }
}

#[cfg(test)]
mod test {
    use super::MavenArtifact;
    #[test]
    fn parses_simple() {
        assert_eq!("net.minecraftforge.forge:some-jar:some-version".parse(),
                   Ok(MavenArtifact {
                       group: "net.minecraftforge.forge".into(),
                       artifact: "some-jar".into(),
                       version: "some-version".into(),
                       classifier: None,
                       extension: None,
                   }))
    }
    #[test]
    fn parses_with_ext() {
        assert_eq!("net.minecraftforge.forge:some-jar:some-version@zip".parse(),
                   Ok(MavenArtifact {
                       group: "net.minecraftforge.forge".into(),
                       artifact: "some-jar".into(),
                       version: "some-version".into(),
                       classifier: None,
                       extension: Some("zip".into()),
                   }))
    }
    #[test]
    fn parses_with_classifier() {
        assert_eq!("net.minecraftforge.forge:some-jar:some-version:universal".parse(),
                   Ok(MavenArtifact {
                       group: "net.minecraftforge.forge".into(),
                       artifact: "some-jar".into(),
                       version: "some-version".into(),
                       classifier: Some("universal".into()),
                       extension: None,
                   }))
    }
    #[test]
    fn parses_with_ext_and_classifier() {
        assert_eq!("net.minecraftforge.forge:some-jar:some-version:universal@zip".parse(),
                   Ok(MavenArtifact {
                       group: "net.minecraftforge.forge".into(),
                       artifact: "some-jar".into(),
                       version: "some-version".into(),
                       classifier: Some("universal".into()),
                       extension: Some("zip".into()),
                   }))
    }
}
