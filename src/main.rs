// TODO use `camino` for paths
use anyhow::Error;
use cargo_metadata::Metadata;
use hyper::{
    header::{self, HeaderValue},
    service::{make_service_fn, service_fn},
    Body, Method, Request, Response, Server, StatusCode,
};
use hyper_staticfile::Static;
use qu::ick_use::*;
use std::{
    convert::{Infallible, TryFrom, TryInto},
    net::SocketAddr,
    path::Path,
    sync::Arc,
};
use structopt::StructOpt;

mod cargo_doc;

pub type Result<T = (), E = anyhow::Error> = std::result::Result<T, E>;

#[derive(StructOpt, Debug)]
struct Opt {
    /// Which port should the documentation be served on
    #[structopt(short = "p", long, default_value = "8000")]
    port: u16,
    /// Set this if you want to turn watching the source on
    #[structopt(short = "w", long)]
    watch: bool,
    /// Path to the cargo manifest at the root of the project (Cargo.toml)
    #[structopt(name = "MANIFEST", short = "m", long)]
    manifest: Option<String>,
    /// Add an extra file or directory to be watched
    #[structopt(long = "watch-extra", name = "FILE")]
    watch_extra: Vec<String>,
    /// Listen on all interfaces, not just localhost
    #[structopt(short = "P", long)]
    public: bool,
    /// Arguments to pass to `cargo doc`. Pass flags after a `--`
    #[structopt(name = "ARG")]
    cargo_args: Vec<String>,
}

impl Opt {
    fn metadata(&self) -> Result<Metadata> {
        let mut cmd = cargo_metadata::MetadataCommand::new();
        if let Some(path) = self.manifest() {
            cmd.manifest_path(path);
        }
        cmd.exec().map_err(Into::into)
    }

    fn manifest(&self) -> Option<&Path> {
        self.manifest.as_ref().map(Path::new)
    }
}

/// Build this from arguments.
#[derive(Debug)]
struct Config {
    /// The address to serve on.
    address: SocketAddr,
    /// If None, don't watch. If Some, watch the files listed as well as src.
    watch: Option<Vec<String>>,
    /// Arguments to pass to `cargo doc`
    cargo_args: Vec<String>,
    /// The location of the manifest if it was supplied.
    manifest: Option<String>,
    /// Cargo metadata.
    metadata: Metadata,
    /// Location of output of `cargo doc`
    doc_dir: String,
}

impl TryFrom<Opt> for Config {
    type Error = Error;
    fn try_from(mut opt: Opt) -> Result<Self, Self::Error> {
        let host = if opt.public {
            [0, 0, 0, 0]
        } else {
            [127, 0, 0, 1]
        };

        // We need to skip an extra argument when it's called as "cargo docserve"
        if matches!(opt.cargo_args.get(0).map(|s| s.as_str()), Some("docserve")) {
            // Cannot panic because we just checked it exists.
            opt.cargo_args.remove(0);
        }

        let metadata = opt.metadata()?;
        let doc_dir = format!("{}/doc", metadata.target_directory);
        Ok(Config {
            address: (host, opt.port).into(),
            watch: if opt.watch {
                Some(opt.watch_extra.into_iter().map(Into::into).collect())
            } else {
                None
            },
            cargo_args: opt.cargo_args,
            manifest: opt.manifest,
            metadata,
            doc_dir,
        })
    }
}

impl Config {
    /// Where to open the browser at.
    fn open_at(&self) -> Result<String> {
        let name = self
            .metadata
            .root_package()
            .or_else(|| self.metadata.packages.get(0))
            .map(|pkg| pkg.name.replace('-', "_"))
            .ok_or_else(|| format_err!("could not find any packages"))?;
        Ok(format!("/{}/index.html", name))
    }
}

#[qu::ick]
fn main(opt: Opt) -> Result<(), Error> {
    log::debug!("Args: {:?}", opt);
    let config: Config = opt.try_into()?;
    let config = Arc::new(config);
    log::trace!("Config: {:?}", config);
    log::debug!("Doc dir: {}", config.doc_dir);

    cargo_doc::run(&config)?;

    let shutdown = if config.watch.is_some() {
        Some(cargo_doc::watch(config.clone())?)
    } else {
        None
    };

    // serve target/doc
    log::info!("Running doc server on http://{}", config.address);
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?
        .block_on(async move {
            let address = config.address;
            let make_service = make_service_fn(move |_conn| {
                // clone for each connection
                let config = config.clone();
                async move {
                    Ok::<_, Infallible>(service_fn(move |req: Request<Body>| {
                        // clone for each request
                        let config = config.clone();
                        handle(config, req)
                    }))
                }
            });
            let server = Server::bind(&address).serve(make_service);

            if let Err(e) = server.await {
                log::error!("server errror: {}", e);
            }
        });

    if let Some(shutdown) = shutdown {
        shutdown();
    }
    Ok(())
}

async fn handle(config: Arc<Config>, req: Request<Body>) -> Result<Response<Body>> {
    if matches!((req.method(), req.uri().path()), (&Method::GET, "/")) {
        // Redirect "/" to the docs for the root package.
        let mut res = Response::new(Body::empty());
        let redirect = config.open_at().unwrap();
        *res.status_mut() = StatusCode::MOVED_PERMANENTLY;
        res.headers_mut()
            .insert(header::LOCATION, HeaderValue::from_str(&redirect).unwrap());
        return Ok(res);
    }
    Static::new(config.doc_dir.clone())
        .serve(req)
        .await
        .map_err(Into::into)
}

/*
enum RoutesFuture {
    Root(Arc<String>),
    Docs(StaticFuture<Body>),
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
    type Error = Error;
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
*/
