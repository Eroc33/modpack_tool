use futures::future;
use futures::prelude::*;
use hyper;
use http::{self, Request};
use hyper::Uri;
use hyper::client::HttpConnector;
use http::header::{self, HeaderMap, HeaderValue};
use hyper_tls::HttpsConnector;
use slog::Logger;
use std::fs;
use std::path::PathBuf;
use time;
use url::{self, Url};
use util;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "IO error: {}", _0)]
    Io(#[cause] ::std::io::Error),
    #[fail(display = "Uri error: {}", _0)]
    Uri(#[cause] http::uri::InvalidUri),
    #[fail(display = "Url error: {}", _0)]
    Url(#[cause] url::ParseError),
    #[fail(display = "Hyper error: {}", _0)]
    Hyper(#[cause] hyper::Error),
    #[fail(display = "DurationOutOfRange error: {}", _0)]
    DurationOutOfRange(#[cause] time::OutOfRangeError),
    #[fail(display = "StdTimeError error: {}", _0)]
    StdTimeError(#[cause] ::std::time::SystemTimeError),
    #[fail(display = "Got a redirect without a location header.")]
    MalformedRedirect,
    #[fail(display = "A http client error occurred. Please check your pack.json is valid")]
    HttpClientError,
    #[fail(display = "A http server error occurred. Please try again later")]
    HttpServerError,
    #[fail(display = "There was a problem with the cache.")]
    CacheError,
}

impl From<http::uri::InvalidUri> for Error {
    fn from(err: http::uri::InvalidUri) -> Self {
        Error::Uri(err)
    }
}

impl From<url::ParseError> for Error {
    fn from(err: url::ParseError) -> Self {
        Error::Url(err)
    }
}

impl From<hyper::Error> for Error {
    fn from(err: hyper::Error) -> Self {
        Error::Hyper(err)
    }
}

impl From<::std::io::Error> for Error {
    fn from(err: ::std::io::Error) -> Self {
        Error::Io(err)
    }
}

pub type Result<T> = ::std::result::Result<T, ::download::Error>;

pub type BoxFuture<I> = Box<Future<Item = I, Error = ::download::Error> + Send>;

pub trait Downloadable: Sync {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()>;
}

impl<D: Downloadable + Send + 'static> Downloadable for Vec<D> {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        Box::new(
            future::join_all(
                self.into_iter()
                    .map(move |d| d.download(location.clone(), manager.clone(), log.clone())),
            ).map(|_| ()),
        )
    }
}

impl<'a, D: Downloadable + Send + Clone> Downloadable for &'a [D] {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        Box::new(
            future::collect(
                self.into_iter()
                    .map(move |d| {
                        d.clone()
                            .download(location.clone(), manager.clone(), log.clone())
                    })
                    .collect::<Vec<BoxFuture<()>>>()
                    .into_iter(),
            ).map(|_| ()),
        )
    }
}

impl Downloadable for Url {
    fn download(self, location: PathBuf, manager: DownloadManager, log: Logger) -> BoxFuture<()> {
        let url = self.clone();
        Box::new(async_block!{
            let uri = util::url_to_uri(&url)?;
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
    pub fn new() -> Self {
        HttpSimple {
            http_client: hyper::Client::new(),
            https_client: hyper::Client::builder()
                .build(HttpsConnector::new(4).expect("Couldn't create httpsconnector")),
        }
    }

    pub fn get(&self, uri: Uri) -> hyper::client::ResponseFuture {
        self.request(
            Request::builder()
                .method(http::Method::GET)
                .uri(uri)
                .body(hyper::Body::empty())
                .expect("error constructing request"),
        )
    }

    pub fn request(&self, request: Request<hyper::Body>) -> hyper::client::ResponseFuture {
        match request.uri().scheme_part().cloned() {
            Some(ref scheme) if scheme == &http::uri::Scheme::HTTP => {
                self.http_client.request(request)
            }
            Some(ref scheme) if scheme == &http::uri::Scheme::HTTPS => {
                self.https_client.request(request)
            }
            _ => panic!("Invalid url scheme"),
        }
    }

    pub fn request_following_redirects(
        &self,
        request: Request<hyper::Body>,
    ) -> Result<RedirectFollower> {
        RedirectFollower::new(self.clone(), request)
    }
}

#[derive(Clone)]
pub struct DownloadManager {
    http_client: HttpSimple,
}

impl DownloadManager {
    pub fn new() -> Self {
        DownloadManager {
            http_client: HttpSimple::new(),
        }
    }

    pub fn get(&self, url: Uri) -> Result<RedirectFollower> {
        self.http_client
            .request_following_redirects(self.request_with_base_headers(http::Method::GET, url))
    }

    pub fn download(
        &self,
        uri: Uri,
        path: PathBuf,
        append_filename: bool,
        log: &Logger,
    ) -> BoxFuture<()> {
        self._download(uri, path, append_filename, log)
    }

    fn base_headers(&self) -> HeaderMap {
        let mut head = HeaderMap::new();
        head.insert(
            http::header::USER_AGENT,
            HeaderValue::from_static("CorrosiveModpackTool/0.0.1"),
        );
        head
    }

    fn request_with_base_headers(
        &self,
        method: http::Method,
        uri: Uri,
    ) -> http::Request<hyper::Body> {
        let mut builder = Request::builder();
        builder.method(method).uri(uri);
        let mut key = None;
        for (k, v) in self.base_headers() {
            builder.header(
                k.clone()
                    .or_else(|| key.clone())
                    .expect("one of the keys *must* be set"),
                v,
            );
            //replace key with with k if it's not None
            key = k.or(key);
        }
        builder
            .body(hyper::Body::empty())
            .expect("error building request")
    }

    fn _download(
        &self,
        uri: Uri,
        path: PathBuf,
        append_filename: bool,
        log: &Logger,
    ) -> BoxFuture<()> {
        let log = log.new(o!("uri"=>uri.to_string()));
        trace!(log, "Downloading {}", path.as_path().to_string_lossy());
        let folder_path = if append_filename {
            path.clone()
        } else {
            path.with_file_name("")
        };

        let mut request = self.request_with_base_headers(http::Method::GET, uri);
        let http_client = self.http_client.clone();

        let res = async_block!{
            trace!(log,"Creating dir {}",folder_path.to_string_lossy());
            fs::create_dir_all(folder_path)?;

            // FIXME find a way to workout which mod file is which *before* downloading
            if path.exists() && path.is_file() {
                trace!(log,"Checking timestamp on file {}",path.to_string_lossy());
                let date_time = util::file_timestamp(&path)?;
                let formatted = format!("{}",date_time.format("%a, %d %b %Y %T GMT"));
                let headerval = HeaderValue::from_str(formatted.as_str()).expect("formatted date was not a valid header value");
                request.headers_mut().insert(http::header::IF_MODIFIED_SINCE,headerval);
            }

            trace!(log,"Doing the request now");
            let (res,url) = await!(http_client.request_following_redirects(request)?)?;
            trace!(log,"Request done");

            if res.status() == http::StatusCode::NOT_MODIFIED {
                trace!(log, "not modified, skipping {}", path.as_path().to_string_lossy());
                Ok(())
            }else{
                let mut path = path;
                if append_filename {
                    path.push(get_url_filename(&url));
                }
                trace!(log,"Saving the file to {}",path.as_path().to_string_lossy());
                await!(util::save_stream_to_file(res.into_body(), path))?;
                Ok(())
            }
        };

        Box::new(res)
    }
}

fn get_url_filename(url: &Url) -> String {
    match url.path_segments() {
        Some(parts) => url::percent_encoding::percent_decode(parts.last().unwrap().as_bytes())
            .decode_utf8_lossy()
            .into_owned(),
        None => unreachable!("Couldn't retrive filename as url was not relative"),
    }
}

pub struct RedirectFollower {
    current_response: Option<hyper::client::ResponseFuture>,
    current_location: Option<Url>,
    client: HttpSimple,
    method: http::Method,
    headers: header::HeaderMap,
    version: http::Version,
}

///Automatically follows redirect
///#WARNING: this *only* works for bodyless requests
impl RedirectFollower {
    pub fn new(client: HttpSimple, request: Request<hyper::Body>) -> Result<Self> {
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
    type Item = (http::Response<hyper::Body>, Url);
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let res = if let (Some(current_response), Some(current_location)) = (
            self.current_response.as_mut(),
            self.current_location.as_mut(),
        ) {
            loop {
                if let Async::Ready(res) = current_response.poll()? {
                    match res.status() {
                        http::StatusCode::FOUND
                        | http::StatusCode::MOVED_PERMANENTLY
                        | http::StatusCode::TEMPORARY_REDIRECT => {
                            let next = res.headers()
                                .get(header::LOCATION)
                                .take()
                                .ok_or_else(|| Error::MalformedRedirect)?;
                            let next_url = current_location.join(&*next.to_str()
                                .expect("Location header should only ever be ascii"))?;
                            let next = ::util::url_to_uri(&next_url)?;
                            let mut req = Request::builder()
                                .method(self.method.clone())
                                .uri(next.clone())
                                .version(self.version)
                                .body(hyper::Body::empty())
                                .expect("error building request");
                            *req.headers_mut() = self.headers.clone();
                            *current_response = self.client.request(req);
                            *current_location = next_url;
                        }
                        status if status.is_client_error() => {
                            break Err(Error::HttpClientError);
                        }
                        status if status.is_server_error() => {
                            break Err(Error::HttpServerError);
                        }
                        hyper::StatusCode::OK => {
                            break Ok(Async::Ready((res, current_location.clone())));
                        }
                        other => panic!(
                            "Not sure what to do with the statuscode: {:?}. This is a bug.",
                            other
                        ),
                    }
                } else {
                    break Ok(Async::NotReady);
                }
            }
        } else {
            panic!("RedirectFollower polled after return. This is a bug.")
        };
        match &res {
            &Ok(Async::NotReady) => {}
            _ => {
                self.current_response = None;
                self.current_location = None;
            }
        }
        res
    }
}
