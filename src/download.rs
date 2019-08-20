use futures::{
    future,
    Future,
    TryFutureExt,
};
use hyper::{
    self,
    client::HttpConnector,
};
use http::{
    self,
    Request,
    header::{self, HeaderMap, HeaderValue},
};
use hyper_tls::HttpsConnector;
use slog::Logger;
use url;
use crate::{
    util,
    error::prelude::*,
};
use std::{
    path::PathBuf,
    task::{Poll,Context},
    pin::Pin,
};
use indicatif::ProgressBar;

pub mod error{
    use snafu::Snafu;
    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub))]
    pub enum Error {
        #[snafu(display("IO error: {}", source))]
        Io{
            source: std::io::Error
        },
        #[snafu(display("Uri error: {}", source))]
        Uri{
            source: http::uri::InvalidUri,
        },
        #[snafu(display("Url error: {}", source))]
        Url{
            source: url::ParseError,
        },
        #[snafu(display("Http error: {}", source))]
        Hyper{
            source: hyper::Error,
        },
        #[snafu(display("DurationOutOfRange error: {}", source))]
        DurationOutOfRange{
            source: time::OutOfRangeError,
        },
        #[snafu(display("StdTime error: {}", source))]
        StdTime{
            source: std::time::SystemTimeError,
        },
        #[snafu(display("Got a redirect without a location header."))]
        MalformedRedirect,
        #[snafu(display("A http client error ({:?}) occurred while fetching {}. Please check your pack.json is valid", status, url))]
        HttpClient{
            status: http::StatusCode,
            url: url::Url
        },
        #[snafu(display("A http server error occurred. Please try again later"))]
        HttpServer,
        #[snafu(display("There was a problem with the cache."))]
        Cache,
        #[snafu(display("Error while symlinking: {}", source))]
        Symlink{
            source: std::io::Error,
        }
    }
}
pub use error::Error;

pub type Result<T> = std::result::Result<T, Error>;

pub type BoxFuture<I> = futures::future::BoxFuture<'static,Result<I>>;

pub trait Downloadable: Sync {
    fn download(self, location: PathBuf, manager: Manager, log: Logger) -> BoxFuture<()>;
}

pub trait DownloadMulti: Sync {
    fn download_all(self, location: PathBuf, manager: Manager, log: Logger, progress: ProgressBar) -> BoxFuture<()>;
}

impl<D: Downloadable + Send + 'static> DownloadMulti for Vec<D> {
    fn download_all(self, location: PathBuf, manager: Manager, log: Logger, progress: ProgressBar) -> BoxFuture<()> {
        progress.set_length(self.len() as u64);
        Box::pin(
            future::try_join_all(
                self.into_iter().enumerate()
                    .map(move |(i,d)| {
                        let progress = progress.clone();
                        d.download(location.clone(), manager.clone(), log.clone()).map_ok(move |_| {progress.set_message(format!("Item {}",i).as_str());progress.inc(1)})
                    }),
            ).map_ok(|_| ()),
        )
    }
}

impl Downloadable for url::Url {
    fn download(self, location: PathBuf, manager: Manager, log: Logger) -> BoxFuture<()> {
        Box::pin(async move{
            let uri = util::url_to_uri(&self)?;
            uri.download(location, manager, log).await?;
            Ok(())
        })
    }
}

impl Downloadable for hyper::Uri {
    fn download(self, location: PathBuf, manager: Manager, log: Logger) -> BoxFuture<()> {
        Box::pin(manager.download(self, location, false, &log))
    }
}

#[derive(Clone)]
pub struct HttpSimple {
    https_client: hyper::Client<HttpsConnector<HttpConnector>>,
}

impl Default for HttpSimple {
    fn default() -> Self {
        Self {
            https_client: hyper::Client::builder()
                .build(HttpsConnector::new(4).expect("Couldn't create httpsconnector")),
        }
    }
}

impl HttpSimple {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, uri: hyper::Uri) -> hyper::client::ResponseFuture {
        self.request(
            Request::builder()
                .method(http::Method::GET)
                .uri(uri)
                .body(hyper::Body::empty())
                .expect("error constructing request"),
        )
    }

    pub fn get_following_redirects(&self, uri: hyper::Uri) -> Result<RedirectFollower> {
        self.request_following_redirects(
            Request::builder()
                .method(http::Method::GET)
                .uri(uri)
                .body(hyper::Body::empty())
                .expect("error constructing request"),
        )
    }

    pub fn request(&self, request: Request<hyper::Body>) -> hyper::client::ResponseFuture {
        self.https_client.request(request)
    }

    pub fn request_following_redirects(
        &self,
        request: Request<hyper::Body>,
    ) -> Result<RedirectFollower> {
        RedirectFollower::new(self.clone(), request)
    }
}

#[derive(Default, Clone)]
pub struct Manager {
    http_client: HttpSimple,
}

impl Manager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, url: hyper::Uri) -> Result<RedirectFollower> {
        self.http_client
            .request_following_redirects(self.request_with_base_headers(http::Method::GET, url))
    }

    pub fn download(
        &self,
        uri: hyper::Uri,
        path: PathBuf,
        append_filename: bool,
        log: &Logger,
    ) -> impl Future<Output=Result<()>> {
        self.download_internal(uri, path, append_filename, log)
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
        uri: hyper::Uri,
    ) -> http::Request<hyper::Body> {
        let mut builder = Request::builder();
        builder.method(method).uri(uri);
        let mut key = None;
        for (k, v) in self.base_headers() {
            //k may be None if there's a repeated header value
            builder.header(
                k.clone()
                    .or_else(|| key.clone())
                    .expect("one of the keys *must* be set"),
                v,
            );
            //store current repeated header key
            key = k.or(key);
        }
        builder
            .body(hyper::Body::empty())
            .expect("error building request")
    }

    fn download_internal(
        &self,
        uri: hyper::Uri,
        path: PathBuf,
        append_filename: bool,
        log: &Logger,
    ) -> impl Future<Output=Result<()>> {
        let log = log.new(o!("uri"=>uri.to_string()));
        trace!(log, "Downloading {}", path.as_path().to_string_lossy());
        let folder_path = if append_filename {
            path.clone()
        } else {
            path.with_file_name("")
        };

        let mut request = self.request_with_base_headers(http::Method::GET, uri);
        let http_client = self.http_client.clone();

        async move{
            trace!(log,"Creating dir {}",folder_path.to_string_lossy());
            tokio::fs::create_dir_all(folder_path).await.context(error::Io)?;

            // FIXME find a way to workout which mod file is which *before* downloading
            if path.exists() && path.is_file() {
                trace!(log,"Checking timestamp on file {}",path.to_string_lossy());
                let date_time = util::file_timestamp(&path).context(error::Io)?;
                let formatted = format!("{}",date_time.format("%a, %d %b %Y %T GMT"));
                let headerval = HeaderValue::from_str(formatted.as_str()).expect("formatted date was not a valid header value");
                request.headers_mut().insert(http::header::IF_MODIFIED_SINCE,headerval);
            }

            trace!(log,"Doing the request now");
            let (res,final_url) = http_client.request_following_redirects(request)?.await?;
            trace!(log,"Request done");

            if res.status() == http::StatusCode::NOT_MODIFIED {
                trace!(log, "not modified, skipping {}", path.as_path().to_string_lossy());
                Ok(())
            }else{
                let mut path = path;
                if append_filename {
                    path.push(get_url_filename(&final_url));
                }
                trace!(log,"Saving the file to {}",path.as_path().to_string_lossy());
                util::save_stream_to_file(res.into_body(), path).await?;
                Ok(())
            }
        }
    }
}

fn get_url_filename(url: &url::Url) -> String {
    match url.path_segments() {
        Some(parts) => url::percent_encoding::percent_decode(parts.last().unwrap().as_bytes())
            .decode_utf8_lossy()
            .into_owned(),
        None => unreachable!("Couldn't retrive filename as url was not relative"),
    }
}

pub struct RedirectFollower {
    current_response: Option<Pin<Box<hyper::client::ResponseFuture>>>,
    current_location: Option<url::Url>,
    starting_location: url::Url,
    client: HttpSimple,
    method: http::Method,
    headers: header::HeaderMap,
    version: http::Version,
}

///Automatically follows redirect
///#WARNING: this *only* works for bodyless requests
impl RedirectFollower {
    pub fn new(client: HttpSimple, request: Request<hyper::Body>) -> Result<Self> {
        let url = crate::util::uri_to_url(request.uri())?;
        let method = request.method().clone();
        let headers = request.headers().clone();
        let version = request.version();
        Ok(Self {
            current_response: Some(Box::pin(client.request(request))),
            starting_location: url.clone(),
            current_location: Some(url),
            client,
            method,
            headers,
            version,
        })
    }
}

impl Future for RedirectFollower {
    type Output = Result<(http::Response<hyper::Body>, url::Url)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let res = if let (Some(current_response), Some(current_location)) = (
            this.current_response.as_mut(),
            this.current_location.as_mut(),
        ) {
            loop {
                if let Poll::Ready(res) = std::future::Future::poll(current_response.as_mut(),cx).map_err(|source| error::Hyper.into_error(source))? {
                    match res.status() {
                        http::StatusCode::FOUND
                        | http::StatusCode::MOVED_PERMANENTLY
                        | http::StatusCode::TEMPORARY_REDIRECT => {
                            let next = res.headers()
                                .get(header::LOCATION)
                                .take()
                                .context(error::MalformedRedirect)?;
                            let next_url = current_location.join(&*next.to_str()
                                .expect("Location header should only ever be ascii")).context(error::Url)?;
                            let next = crate::util::url_to_uri(&next_url)?;
                            let mut req = Request::builder()
                                .method(this.method.clone())
                                .uri(next.clone())
                                .version(this.version)
                                .body(hyper::Body::empty())
                                .expect("error building request");
                            *req.headers_mut() = this.headers.clone();
                            *current_response = Box::pin(this.client.request(req));
                            *current_location = next_url;
                        }
                        status if status.is_client_error() => {
                            break Poll::Ready(error::HttpClient{status,url: this.starting_location.clone()}.fail());
                        }
                        status if status.is_server_error() => {
                            break Poll::Ready(error::HttpServer.fail());
                        }
                        hyper::StatusCode::OK => {
                            break Poll::Ready(Ok((res, current_location.clone())));
                        }
                        other => panic!(
                            "Not sure what to do with the statuscode: {:?}. This is a bug.",
                            other
                        ),
                    }
                } else {
                    break Poll::Pending;
                }
            }
        } else {
            panic!("RedirectFollower polled after return. This is a bug.")
        };
        if res.is_ready(){
            this.current_response = None;
                this.current_location = None;
        }
        res
    }
}
