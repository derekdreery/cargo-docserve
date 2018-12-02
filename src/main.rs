use failure::{Compat, Error, ResultExt};
use futures::{future, Async, Future, Poll};
use http::{header, response::Builder as ResponseBuilder, Request, Response, StatusCode};
use hyper::{Body, Server};
use hyper_staticfile::{Static, StaticFuture};
use log::LevelFilter;
use notify::{watcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct Cli {
    /// Which port should the documentation be served on
    #[structopt(short = "p", long = "port", default_value = "8000")]
    port: u16,
    /// Set this if you want to turn watching the source off
    #[structopt(short = "n", long = "do-not-watch")]
    no_watch: bool,
    /// Path to the cargo manifest at the root of the project (Cargo.toml)
    #[structopt(name = "MANIFEST", short = "m", long = "manifest")]
    manifest: Option<String>,
    /// Add an extra file or directory to be watched
    #[structopt(long = "watch-extra", name = "FILE")]
    watch_extra: Vec<String>,
    /// Listen on all interfaces, not just localhost
    #[structopt(short = "P", long = "public")]
    public: bool,
    /// Arguments to pass to `cargo doc`. Pass flags after a `--`
    #[structopt(name = "ARG")]
    cargo_args: Vec<String>,
}

fn main() -> Result<(), Error> {
    pretty_env_logger::formatted_builder()
        .unwrap()
        .filter(None, LevelFilter::Info)
        .init();
    let opts = Cli::from_args();
    log::debug!("Args: {:?}", opts);
    // We need to skip an extra argument when it's called as "cargo docserve"
    let cargo_args = opts
        .cargo_args
        .iter()
        .enumerate()
        .filter_map(|(idx, val)| {
            if idx == 0 && val == "docserve" {
                None
            } else {
                Some(val.clone())
            }
        })
        .collect::<Vec<_>>();
    let cargo_args = Arc::new(cargo_args);

    let metadata = Arc::new(
        cargo_metadata::metadata_run(opts.manifest.as_ref().map(Path::new), false, None)
            .map_err(failure::SyncFailure::new)
            .context("getting package metadata")?,
    );
    log::debug!("Metadata: {:#?}", &metadata);
    let doc_dir = Path::new(&metadata.target_directory).join("doc");
    log::debug!("Doc dir: {}", doc_dir.display());
    let package = &metadata
        .packages
        .get(0)
        .ok_or(failure::err_msg("crate must have at least 1 package"))?
        .name;
    let index = format!("{}/index.html", package.replace('-', "_"));
    run_cargo(
        cargo_args.clone(),
        opts.manifest.as_ref().map(String::as_str),
    )?;

    let host = if opts.public {
        [0, 0, 0, 0]
    } else {
        [127, 0, 0, 1]
    };
    let addr = (host, opts.port).into();

    if !opts.no_watch {
        std::thread::spawn(move || -> Result<(), Error> {
            let metadata = metadata.clone();
            let (tx, rx) = channel();
            let mut watcher = watcher(tx, Duration::from_secs(1)).unwrap();
            // TODO probably more efficient to not watch `target`, rather than ignore it.
            if let Err(e) = watcher.watch(&metadata.workspace_root, RecursiveMode::Recursive) {
                log::error!("Cannot watch \"{}\": {}", metadata.workspace_root, e);
            }
            for extra in opts.watch_extra.iter() {
                if let Err(e) = watcher.watch(extra, RecursiveMode::Recursive) {
                    log::error!("Cannot watch \"{}\": {}", extra, e);
                }
            }
            loop {
                use notify::DebouncedEvent::*;
                match rx.recv() {
                    Ok(Create(path)) | Ok(Write(path)) | Ok(Remove(path)) => {
                        if path.starts_with(&metadata.target_directory) {
                            // Don't rebuild on change in target or else we get into an infinite
                            // loop.
                            continue;
                        }
                        log::debug!("path {} changed, rebuilding", path.display());
                        if let Err(e) = run_cargo(
                            cargo_args.clone(),
                            opts.manifest.as_ref().map(String::as_str),
                        ) {
                            log::error!("{}", e);
                        }
                    }
                    Ok(Rename(..)) => {
                        // FIXME check if we should look at whether we are in /target/
                        if let Err(e) = run_cargo(
                            cargo_args.clone(),
                            opts.manifest.as_ref().map(String::as_str),
                        ) {
                            log::error!("{}", e);
                        }
                    }
                    Ok(Error(e, ..)) => log::error!("{}", e),
                    Err(e) => log::error!("{}", e),
                    _ => (),
                }
            }
        });
    }

    let server = Server::bind(&addr)
        .serve(move || {
            future::ok::<_, Compat<Error>>(DocService::new(doc_dir.clone(), index.clone()))
        })
        .map_err(|e| eprintln!("server errror: {}", e));

    log::info!("Server running on {}", addr);
    hyper::rt::run(server);
    Ok(())
}

enum RoutesFuture {
    Root(Arc<String>),
    Docs(StaticFuture<Body>),
}

impl Future for RoutesFuture {
    type Item = Response<Body>;
    type Error = Compat<Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match *self {
            RoutesFuture::Root(ref index) => {
                let res = ResponseBuilder::new()
                    .status(StatusCode::SEE_OTHER)
                    .header(header::LOCATION, AsRef::as_ref(index).as_str())
                    .body(Body::empty())
                    .map_err(|e| Error::compat(e.into()))?;
                Ok(Async::Ready(res))
            }
            RoutesFuture::Docs(ref mut future) => {
                future.poll().map_err(|e| Error::compat(e.into()))
            }
        }
    }
}

/// The object serving the docs.
struct DocService {
    static_: Static,
    redirect: Arc<String>,
}

impl DocService {
    pub fn new(doc_path: impl Into<PathBuf>, start_page: impl Into<String>) -> Self {
        Self {
            static_: Static::new(doc_path),
            redirect: Arc::new(start_page.into()),
        }
    }
}

impl hyper::service::Service for DocService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Compat<Error>;
    type Future = RoutesFuture;

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        // Redirect root requests
        if req.uri().path() == "/" {
            RoutesFuture::Root(self.redirect.clone())
        } else {
            RoutesFuture::Docs(self.static_.serve(req))
        }
    }
}

fn run_cargo(cargo_args: impl AsRef<Vec<String>>, manifest: Option<&str>) -> Result<(), Error> {
    let mut cmd = Command::new("cargo");
    cmd.arg("doc");
    if let Some(m) = manifest {
        cmd.args(&["--manifest-path", m]);
    }
    cmd.args(cargo_args.as_ref())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    log::debug!("running `{:?}`", cmd);
    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(failure::format_err!(
            "cargo doc failed with error code {:?}",
            status.code()
        ))
    }
}
