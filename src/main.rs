use hyper::{Body, Server};
use hyper::service::service_fn_ok;
use http::{Request, Response, StatusCode, header, response::Builder as ResponseBuilder};
use failure::{Compat, Error, ResultExt};
use structopt::StructOpt;
use std::process::{Stdio, Command};
use std::path::Path;
use log::LevelFilter;
use hyper_staticfile::{Static, StaticFuture};
use futures::{Async, Future, Poll, future};

const PHRASE: &str = "Hello, world";

#[derive(StructOpt, Debug)]
struct Cli {
    #[structopt(short = "p", long = "port", default_value = "8000")]
    port: u16,
    #[structopt(short = "w", long = "watch")]
    watch: bool,
    cargo_args: Vec<String>,
}

fn hello_world(_: Request<Body>) -> Response<Body> {
    Response::new(Body::from(PHRASE))
}

fn main() -> Result<(), Error> {
    pretty_env_logger::formatted_builder().unwrap()
        .filter(None, LevelFilter::Debug)
        .init();
    let opts = Cli::from_args();
    log::debug!("{:?}", opts);

    let metadata = cargo_metadata::metadata_run(None, false, None)
        .map_err(failure::SyncFailure::new)
        .context("getting package metadata")?;
    let doc_dir = Path::new(&metadata.target_directory).join("doc");
    log::debug!("Doc dir: {}", doc_dir.display());
    run_cargo(&opts)?;

    let addr = ([127, 0, 0, 1],  opts.port).into();

    let new_svc = || {
        service_fn_ok(hello_world)
    };

    let server = Server::bind(&addr)
        .serve(new_svc)
        .map_err(|e| eprintln!("server errror: {}", e));

    hyper::rt::run(server);
    Ok(())
}

enum RoutesFuture {
    Root,
    Docs(StaticFuture<Body>),
}

impl Future for RoutesFuture {
    type Item = Response<Body>;
    type Error = Compat<Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match *self {
            RoutesFuture::Root => {
                let res = ResponseBuilder::new()
                    .status(StatusCode::SEE_OTHER)
                    .header(header::LOCATION, "package/index.html")
                    .body(Body::empty())
                    .map_err(|e| Error::compat(e.into()))?;
                Ok(Async::Ready(res))
            },
            RoutesFuture::Docs(ref mut future) => future.poll()
                .map_err(|e| Error::compat(e.into()))
        }
    }
}

/// The object serving the docs.
struct DocService {
    static_: Static
}

impl hyper::service::Service for DocService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Compat<Error>;
    type Future = RoutesFuture;

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        // Redirect root requests
        if req.uri().path() == "/" {
            RoutesFuture::Root
        } else {
            RoutesFuture::Docs(self.static_.serve(req))
        }
    }
}

fn run_cargo(opts: &Cli) -> Result<(), Error> {
    let mut cmd = Command::new("cargo");
    cmd.arg("doc")
        .args(&opts.cargo_args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    log::debug!("running `{:?}`", cmd);
    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(failure::format_err!("cargo doc failed with error code {:?}", status.code()))
    }
}

