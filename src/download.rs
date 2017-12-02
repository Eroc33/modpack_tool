use futures::future;
use futures::prelude::*;
use hyper;
use hyper::Uri;
use hyper::client::HttpConnector;
use hyper::client::Request;
use hyper::header;
use hyper_tls::HttpsConnector;
use slog::Logger;
use std::fs;
use std::path::PathBuf;
use time;
use tokio_core::reactor::Handle;
use url::{self, Url};
use util;

error_chain! {
  foreign_links {
    Io(::std::io::Error);
    Uri(hyper::error::UriError);
    Url(url::ParseError);
    Hyper(hyper::Error);
    DurationOutOfRange(time::OutOfRangeError);
    StdTimeError(::std::time::SystemTimeError);
  }
  errors{
      MalformedRedirect{
          description("Got a redirect without a location header.")
      }
      HttpClientError{
          description("A http client error occurred. Please check your pack.json is valid")
      }
      HttpServerError{
          description("A http server error occurred. Please try again later")
      }
      CacheError{
          description("There was a problem with the cache.")
      }
  }
}

pub type BoxFuture<I> = Box<Future<Item = I, Error = ::download::Error>>;

pub trait Downloadable: Sync {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()>;
}

impl<D: Downloadable + Send + 'static> Downloadable for Vec<D> {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        Box::new(
            future::join_all(
                self.into_iter()
                    .map(move |d| d.download(location.clone(), manager.clone(), log.clone()))
            ).map(|_|())
        )
    }
}

impl<'a, D: Downloadable + Send + Clone> Downloadable for &'a [D] {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        Box::new(future::collect(self.into_iter()
                .map(move |d| {
                    d.clone().download(location.clone(), manager.clone(), log.clone())
                })
                .collect::<Vec<BoxFuture<()>>>()
                .into_iter())
            .map(|_| ()))
    }
}

impl Downloadable for Url {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        Box::new(async_block!{
            let uri = util::url_to_uri(&self)?;
            Ok(await!(uri.download(location, manager, log))?)
        })
    }
}

impl Downloadable for Uri {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        Box::new(manager.download(self, location, false, &log))
    }
}

#[derive(Clone)]
pub struct HttpSimple {
    http_client: hyper::Client<HttpConnector>,
    https_client: hyper::Client<HttpsConnector<HttpConnector>>,
}

impl HttpSimple {
    pub fn new(handle: &Handle) -> Self {
        HttpSimple {
            http_client: hyper::Client::new(handle),
            https_client: hyper::Client::configure()
                .connector(HttpsConnector::new(4, handle).expect("Couldn't create httpsconnector"))
                .build(handle),
        }
    }

    pub fn get(&self, uri: Uri) -> hyper::client::FutureResponse {
        self.request(Request::new(hyper::Method::Get, uri))
    }

    pub fn request(&self, request: Request) -> hyper::client::FutureResponse {
        match request.uri().scheme() {
            Some("http") => self.http_client.request(request),
            Some("https") => self.https_client.request(request),
            _ => panic!("Invalid url scheme"),
        }
    }

    pub fn request_following_redirects(&self, request: Request) -> Result<RedirectFollower> {
        RedirectFollower::new(self.clone(), request)
    }
}

#[derive(Clone)]
pub struct DownloadManager {
    http_client: HttpSimple,
}

impl DownloadManager {
    pub fn new(handle: &Handle) -> Self {
        DownloadManager { http_client: HttpSimple::new(handle) }
    }

    pub fn get(&self, url: Uri) -> Result<RedirectFollower> {
        self.http_client
            .request_following_redirects(self.request_with_base_headers(hyper::Method::Get, url))
    }

    pub fn download(&self,
                    uri: Uri,
                    path: PathBuf,
                    append_filename: bool,
                    log: &Logger)
                    -> BoxFuture<()> {
        self._download(uri, path, append_filename, log)
    }

    fn base_headers(&self) -> hyper::header::Headers {
        let mut head = hyper::header::Headers::new();
        head.set(hyper::header::UserAgent::new("CorrosiveModpackTool/0.0.1"));
        head
    }

    fn request_with_base_headers(&self, method: hyper::Method, uri: Uri) -> hyper::client::Request {
        let mut req = Request::new(method, uri);
        *req.headers_mut() = self.base_headers();
        req
    }

    fn _download(&self,
                 uri: Uri,
                 path: PathBuf,
                 append_filename: bool,
                 log: &Logger)
                 -> BoxFuture<()> {
        let log = log.new(o!("uri"=>uri.to_string()));
        trace!(log, "Downloading {}", path.as_path().to_string_lossy());
        let folder_path = if append_filename {
            path.clone()
        } else {
            path.with_file_name("")
        };

        let mut request = self.request_with_base_headers(hyper::Method::Get, uri);
        let http_client = self.http_client.clone();

        let res = async_block!{
            trace!(log,"Creating dir {}",folder_path.to_string_lossy());
            fs::create_dir_all(folder_path)?;

            // FIXME find a way to workout which mod file is which *before* downloading
            if path.exists() && path.is_file() {
                trace!(log,"Checking timestamp on file {}",path.to_string_lossy());
                let timestamp = util::file_timestamp(&path)?;
                request.headers_mut().set(hyper::header::IfModifiedSince(hyper::header::HttpDate::from(timestamp)));
            }

            trace!(log,"Doing the request now");
            let (res,url) = await!(http_client.request_following_redirects(request)?)?;
            trace!(log,"Request done");
            
            if res.status() == hyper::StatusCode::NotModified {
                trace!(log, "not modified, skipping {}", path.as_path().to_string_lossy());
                Ok(())
            }else{
                let mut path = path;
                if append_filename {
                    path.push(get_url_filename(&url));
                }
                trace!(log,"Saving the file to {}",path.as_path().to_string_lossy());
                await!(util::save_stream_to_file(res.body(), path))?;
                Ok(())
            }
        };

        Box::new(res)
    }
}

fn get_url_filename(url: &Url) -> String {
    match url.path_segments() {
        Some(parts) => {
            url::percent_encoding::percent_decode(parts.last().unwrap().as_bytes())
                .decode_utf8_lossy()
                .into_owned()
        }
        None => unreachable!("Couldn't retrive filename as url was not relative"),
    }
}

pub struct RedirectFollower {
    current_response: Option<hyper::client::FutureResponse>,
    current_location: Option<Url>,
    client: HttpSimple,
    method: hyper::Method,
    headers: header::Headers,
    version: hyper::HttpVersion,
}

///Automatically follows redirect
///#WARNING: this *only* works for bodyless requests
impl RedirectFollower {
    pub fn new(client: HttpSimple, request: Request) -> Result<Self> {
        let url = ::util::uri_to_url(request.uri())?;
        let method = request.method().clone();
        let headers = request.headers().clone();
        let version = request.version();
        Ok(RedirectFollower {
            current_response: Some(client.request(request)),
            current_location: Some(url),
            client: client,
            method: method,
            headers: headers,
            version: version,
        })
    }
}

impl Future for RedirectFollower {
    type Item = (hyper::client::Response, Url);
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let (next_response, next_location) = if let (Some(mut current_response),
                                                     Some(current_location)) =
            (self.current_response.take(), self.current_location.take()) {
            if let Async::Ready(res) = current_response.poll()? {
                match res.status() {
                    hyper::StatusCode::Found | hyper::StatusCode::MovedPermanently | hyper::StatusCode::TemporaryRedirect => {
                        let next = res.headers().get::<header::Location>().take().ok_or_else(|| ErrorKind::MalformedRedirect)?;
                        let next_url = current_location.join(&*next)?;
                        let next = ::util::url_to_uri(&next_url)?;
                        let mut req = Request::new(self.method.clone(), next.clone());
                        req.set_version(self.version);
                        *req.headers_mut() = self.headers.clone();
                        (self.client.request(req), next_url)
                    },
                    status if status.is_client_error() => {
                        return Err(ErrorKind::HttpClientError.into())
                    }
                    status if status.is_server_error() => {
                        return Err(ErrorKind::HttpServerError.into());
                    }
                    hyper::StatusCode::Ok => {
                        return Ok(Async::Ready((res, current_location)))
                    }
                    other => panic!("Not sure what to do with the statuscode: {:?}. This is a bug.",other),
                }
            } else {
                (current_response, current_location)
            }
        } else {
            panic!("RedirectFollower polled after return. This is a bug.")
        };
        self.current_response = Some(next_response);
        self.current_location = Some(next_location);
        Ok(Async::NotReady)
    }
}
