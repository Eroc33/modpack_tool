use futures::prelude::*;
use http::Uri;
use slog::Logger;
use std::{
    iter::FromIterator,
    path::{Path, PathBuf},
    str::FromStr,
};
use crate::{
    cache::{self, Cacheable, Cache as _},
    download,
    error::prelude::*,
};
use tokio::io::AsyncReadExt;
mod hash_writer;
use hash_writer::HashWriter;

mod error{
    use snafu::Snafu;
    #[derive(Debug,Snafu)]
    #[snafu(visibility(pub))]
    pub enum Error{
        #[snafu(display("Invalid uri: {}", source))]
        BadUri{
            source: http::uri::InvalidUri,
        },
        #[snafu(display("Invalid url: {}", source))]
        BadUrl{
            source: url::ParseError,
        },
    }
}

#[derive(Debug)]
pub enum VerifyResult {
    Good,
    Bad,
    NotInCache,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Artifact {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedArtifact {
    pub artifact: Artifact,
    pub repo: Uri,
}

pub struct Cache;

impl Cacheable for ResolvedArtifact {
    type Cache = crate::cache::FileCache;
    fn cached_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(app_dirs::app_dir(app_dirs::AppDataType::UserCache, crate::APP_INFO, "maven_cache").expect("Cache directory must be accesible"));
        p.push(&self.artifact.to_path());
        p
    }
    fn uri(&self) -> crate::cache::Result<Uri> {
        crate::cache::ResultExt::erased(self.artifact.get_uri_on(&self.repo))
    }
}

impl cache::Cache<ResolvedArtifact> for Cache {
    fn with(
        artifact: ResolvedArtifact,
        manager: download::Manager,
        log: Logger,
    ) -> crate::cache::BoxFuture<PathBuf> {
        let cached_path = artifact.cached_path();
        let log = log.new(
            o!("artifact"=>artifact.artifact.to_string(),"repo"=>artifact.repo.to_string(),"cached_path"=>cached_path.as_path().to_string_lossy().into_owned()),
        );
        Box::pin(async move{
            info!(log, "caching maven artifact");
            if !Self::is_cached(&artifact) {
                info!(log, "artifact is not cached, downloading now");
                let uri = artifact.uri()?;
                manager
                    .download(uri.clone(), cached_path.clone(), false, &log).await.context(crate::cache::error::Downloading{uri})?;
            }
            Ok(cached_path)
        })
    }
}

impl Cache {
    pub async fn verify_cached(
        resolved: ResolvedArtifact,
        manager: download::Manager,
    ) -> download::Result<VerifyResult> {
        if Self::is_cached(&resolved) {
            let cached_path = resolved.cached_path();
            let sha_url_res = resolved.sha_uri();
            let mut cached_file = tokio::fs::File::open(cached_path).await.context(download::error::Io)?;

            let mut sha = HashWriter::new();
            cached_file.copy(&mut sha).await.context(download::error::Io)?;
            let cached_sha = sha.digest();

            let sha_uri = sha_url_res?;
            let (res,_) = manager.get(sha_uri)?.await?;
            let hash_str = res.into_body().map_ok(hyper::Chunk::into_bytes).try_concat().await.context(download::error::Hyper)?;
            if hash_str == format!("{}", cached_sha) {
                Ok(VerifyResult::Good)
            } else {
                Ok(VerifyResult::Bad)
            }
        } else {
            Ok(VerifyResult::NotInCache)
        }
    }
}

impl Artifact {
    fn to_path(&self) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(&self.group_path());
        p.push(&self.artifact);
        p.push(&self.version);
        p.push(&self.artifact_filename());
        p
    }

    pub fn get_uri_on(&self, base: &Uri) -> Result<Uri,error::Error> {
        let base = crate::util::uri_to_url(base).context(error::BadUrl)?;
        let path = self.to_path();
        let url = base.join(path.to_str().expect("non unicode path encountered")).context(error::BadUrl)?;
        crate::util::url_to_uri(&url).context(error::BadUri)
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
        manager: download::Manager,
        log: Logger,
    ) -> impl Future<Output=Result<(), crate::cache::Error>> + Send {
        Cache::install_at(self.resolve(repo_uri), location.to_owned(), manager, log)
    }
}

impl ResolvedArtifact {
    pub fn to_path(&self) -> PathBuf {
        self.artifact.to_path()
    }
    pub fn sha_uri(&self) -> crate::download::Result<Uri> {
        let mut url = crate::util::uri_to_url(&self.uri().context(download::error::Cached)?).context(download::error::BadUrl)?;
        let mut path = url.path().to_owned();
        path.push_str(".sha1");
        url.set_path(path.as_ref());
        crate::util::url_to_uri(&url).context(download::error::BadUri)
    }
    pub fn install_at_no_classifier(
        self,
        location: PathBuf,
        manager: download::Manager,
        log: Logger,
    ) -> impl Future<Output=crate::cache::Result<()>> + Send {
        async move{
            let cached_path_no_classifier = Self {
                artifact: Artifact {
                    classifier: None,
                    ..self.artifact.clone()
                },
                repo: self.repo.clone(),
            }.cached_path();

            let filename = cached_path_no_classifier.file_name().expect("Maven artifact should have a filename");
            
            <Self as Cacheable>::install_at_custom_filename(self, location, filename.to_os_string(), manager, log).await
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArtifactParseError {
    BadNumberOfParts,
}

impl ToString for Artifact {
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

impl FromStr for Artifact {
    type Err = ArtifactParseError;
    fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('@').collect();
        let (s, ext): (&str, Option<String>) = match *parts.as_slice() {
            [s, ext] => (s, Some(ext.to_string())),
            _ => (s, None),
        };

        let parts = s.split(':');
        let parts: Vec<&str> = parts.collect();
        match *parts.as_slice() {
            [grp, art, ver] => Ok(Self {
                group: grp.into(),
                artifact: art.into(),
                version: ver.into(),
                classifier: None,
                extension: ext,
            }),
            [grp, art, ver, class] => Ok(Self {
                group: grp.into(),
                artifact: art.into(),
                version: ver.into(),
                classifier: Some(class.into()),
                extension: ext,
            }),
            _ => Err(ArtifactParseError::BadNumberOfParts),
        }
    }
}

#[cfg(test)]
mod test {
    use super::Artifact;
    #[test]
    fn parses_simple() {
        assert_eq!(
            "net.minecraftforge.forge:some-jar:some-version".parse(),
            Ok(Artifact {
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
            Ok(Artifact {
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
            Ok(Artifact {
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
            Ok(Artifact {
                group: "net.minecraftforge.forge".into(),
                artifact: "some-jar".into(),
                version: "some-version".into(),
                classifier: Some("universal".into()),
                extension: Some("zip".into()),
            })
        )
    }
}
