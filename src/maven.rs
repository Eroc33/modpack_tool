#![allow(redundant_closure)]
use download::{self, Downloadable, DownloadManager};
use futures::{self, Future};
use hash_writer::HashWriter;
use slog::Logger;
use std::fs::File;
use std::io::prelude::*;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use url;
use util;

const CACHE_DIR: &'static str = "./mvn_cache/";

#[derive(Debug)]
pub enum VerifyResult {
    Good,
    Bad,
    NotInCache,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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
    pub repo: Url,
}

use hyper::Url;

struct MavenCache;

impl MavenCache {
    pub fn cached(artifact: &MavenArtifact) -> bool {
        Self::cached_path(artifact).exists()
    }
    
    pub fn with(artifact: &ResolvedArtifact,
                 manager: DownloadManager,
                 log: Logger) -> download::BoxFuture<PathBuf>
    {
        let cached_path = Self::cached_path(&artifact.artifact);
        let log = log.new(o!("artifact"=>artifact.artifact.to_string(),"repo"=>artifact.repo.to_string(),"cached_path"=>cached_path.as_path().to_string_lossy().into_owned()));
        info!(log,"caching maven artifact");
        if !Self::cached(&artifact.artifact) {
            info!(log,"artifact is not cached, downloading now");
            match artifact.url() {
                Ok(url) => manager.download(&url, cached_path.clone(), false, log).map(move |_| cached_path).boxed(),
                Err(e) => futures::failed(download::Error::from(e)).boxed(),
            }
        }else{
            info!(log,"artifact was cached");
            futures::finished(cached_path).boxed()
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
                         -> download::Result<VerifyResult> {
        if !Self::cached(&resolved.artifact) {
            Ok(VerifyResult::NotInCache)
        } else {
            let cached_path = Self::cached_path(&resolved.artifact);
            let mut cached_file = File::open(cached_path)?;
            let mut sha = HashWriter::new();
            ::std::io::copy(&mut cached_file, &mut sha)?;
            let cached_sha = sha.digest();
            let mut res = manager.get(resolved.sha_url()?).send()?;
            let mut res_str = String::new();
            res.read_to_string(&mut res_str)?;
            if res_str == format!("{}", cached_sha) {
                Ok(VerifyResult::Good)
            } else {
                Ok(VerifyResult::Bad)
            }
        }
    }

    // FIXME Annoying work around for the forge libs not really being where they claim to be
    fn install_at_no_classifier<'a>(artifact: ResolvedArtifact,
                                    mut location: PathBuf,
                                    manager: DownloadManager,
                                    log: Logger)
                                    -> impl download::Future<()> + 'a {
        Self::with(&artifact,manager,log.clone()).and_then(move |cached_path|{
            let ResolvedArtifact { artifact, repo } = artifact;
            let log = log.new(o!("artifact"=>artifact.to_string(),"repo"=>repo.to_string()));
            info!(log,"installing maven artifact");
        
            let mut artifact_no_classifier = artifact.clone();
            artifact_no_classifier.classifier = None;
            let cached_path_no_classifier = Self::cached_path(&artifact_no_classifier);
            cached_path_no_classifier.file_name().map(|n| location.push(n));
            util::create_dir(location.to_owned())?;
            util::symlink(cached_path, location, &log)?;
            Ok(())
        })
    }

    fn install_at<'a>(artifact: ResolvedArtifact,
                      mut location: PathBuf,
                      manager: DownloadManager,
                      log: Logger)
                      -> impl download::Future<()> + 'a{
        Self::with(&artifact,manager,log.clone()).and_then(move |cached_path|{
            let ResolvedArtifact { artifact, repo } = artifact;
            let log = log.new(o!("artifact"=>artifact.to_string(),"repo"=>repo.to_string()));
            info!(log,"installing maven artifact");
            
            cached_path.file_name().map(|n| location.push(n));
            util::create_dir(location.to_owned())?;
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

    pub fn get_url_on(&self, base: &Url) -> ::std::result::Result<Url, url::ParseError> {
        let path = self.to_path();
        base.join(path.to_str().expect("non unicode path encountered"))
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

    pub fn resolve(&self, repo_url: Url) -> ResolvedArtifact {
        ResolvedArtifact {
            artifact: self.clone(),
            repo: repo_url,
        }
    }

    pub fn download_from<'a>(&self,
                         location: &Path,
                         repo_url: Url,
                         manager: DownloadManager,
                         log: Logger)
                         -> impl download::Future<()> + 'a {
        MavenCache::install_at(self.resolve(repo_url), location.to_owned(), manager, log)
    }
}

impl ResolvedArtifact {
    pub fn to_path(&self) -> PathBuf {
        self.artifact.to_path()
    }
    pub fn url(&self) -> ::std::result::Result<Url, url::ParseError> {
        self.artifact.get_url_on(&self.repo)
    }
    pub fn sha_url(&self) -> ::std::result::Result<Url, url::ParseError> {
        let mut url = self.url()?;
        let mut path = url.path().to_owned();
        path.push_str(".sha1");
        url.set_path(path.as_ref());
        Ok(url)
    }
    pub fn install_at(&self,
                          location: &Path,
                          manager: DownloadManager,
                          log: Logger)
                          -> download::BoxFuture<()> {
        MavenCache::install_at((*self).clone(), location.to_owned(), manager, log).boxed()
    }
    pub fn reader(&self, manager: DownloadManager, log: Logger) -> download::BoxFuture<::std::fs::File> {
        MavenCache::with(self, manager, log).and_then(move |path| {
            Ok(::std::fs::File::open(path)?)
        }).boxed()
    }
    pub fn install_at_no_classifier(&self,
                                        location: &Path,
                                        manager: DownloadManager,
                                        log: Logger)
                                        -> download::BoxFuture<()>{
        MavenCache::install_at_no_classifier((*self).clone(), location.to_owned(), manager, log).boxed()
    }
}

impl Downloadable for ResolvedArtifact {
    fn download(self,
                location: PathBuf,
                manager: DownloadManager,
                log: Logger)
                -> download::BoxFuture<()> {
        MavenCache::install_at(self, location, manager, log).boxed()
    }
}

#[derive(Debug)]
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
        if let Some(ref classifier) = self.classifier{
            strn.push(':');
            strn.push_str(classifier);
        }
        if let Some(ref ext) = self.extension{
            strn.push('@');
            strn.push_str(ext);
        }
        strn
    }
}

impl FromStr for MavenArtifact {
    type Err = MavenArtifactParseError;
    fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
        let parts = s.split(':');
        let parts: Vec<&str> = parts.into_iter().collect();
        match *parts.as_slice() {
            [grp, art, ver] => {
                let parts: Vec<&str> = ver.split('@').into_iter().collect();
                let (ver, ext): (&str, Option<String>) = match *parts.as_slice() {
                    [ver, ext] => (ver, Some(ext.to_string())),
                    _ => (ver, None),
                };
                Ok(MavenArtifact {
                    group: grp.to_string(),
                    artifact: art.to_string(),
                    version: ver.to_string(),
                    classifier: None,
                    extension: ext,
                })
            }
            [grp, art, ver, class] => {
                let parts: Vec<&str> = class.split('@').into_iter().collect();
                let (class, ext) = match *parts.as_slice() {
                    [class, ext] => (Some(class.to_string()), Some(ext.to_string())),
                    _ => (Some(class.to_string()), None),
                };
                Ok(MavenArtifact {
                    group: grp.to_string(),
                    artifact: art.to_string(),
                    version: ver.to_string(),
                    classifier: class,
                    extension: ext,
                })
            }
            _ => Err(MavenArtifactParseError::BadNumberOfParts),
        }
    }
}
