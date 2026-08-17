use std::path::PathBuf;

use axum::{Router, error_handling::HandleError, routing::get_service, serve};
use http::StatusCode;
use tokio::net::TcpListener;
use tonic_web::GrpcWebLayer;
use tower::Layer;
use tower_http::trace::TraceLayer;
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
};
use tracing::{Level, info_span};
use tracing::{error, info};

use crate::proxy::Proxy;

#[derive(Debug)]
pub struct ServerOptions {
    /// The address to host the HTTP/1.1 proxy/web server on.
    pub http_address: String,
    /// The address of the gRPC server to forwarded the requests to and from
    pub grpc_address: String,
    /// The directory of the static web files to host
    pub static_dir: Option<String>,
    /// The path to the CA cert used to generate the gRPC server and proxy certificates and keys. Required for TLS.
    pub grpc_ca_cert: Option<PathBuf>,
    /// The path to the gRPC proxy private key. Required for mTLS
    pub grpc_proxy_key: Option<PathBuf>,
    /// The path to the gRPC proxy certification. Required for mTLS
    pub grpc_proxy_cert: Option<PathBuf>,
}

impl ServerOptions {
    pub fn validate(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.grpc_proxy_key.is_some() ^ self.grpc_proxy_cert.is_some() {
            if self.grpc_ca_cert.is_none() {
                return Err("grpc_ca_cert must be defined for mTLS".into());
            }
            if self.grpc_proxy_key.is_none() {
                return Err("grpc_proxy_key must be defined for mTLS".into());
            }
            if self.grpc_proxy_cert.is_none() {
                return Err("grpc_proxy_cert must be defined for mTLS".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Server {
    router: Router,
    options: ServerOptions,
}

impl Server {
    pub fn new(server_options: ServerOptions) -> Result<Self, Box<dyn std::error::Error>> {
        server_options.validate()?;

        let mut router = Router::new();

        router = Self::add_proxy(&server_options, router)?;
        router = Self::add_web(&server_options, router);
        router = Self::add_cors(router);
        router = Self::add_observability(router);

        Ok(Self {
            router,
            options: server_options,
        })
    }

    fn add_proxy(
        server_options: &ServerOptions,
        router: Router,
    ) -> Result<Router, Box<dyn std::error::Error>> {
        let proxy = Proxy::new(
            server_options.grpc_address.as_str(),
            server_options.grpc_ca_cert.as_deref(),
        )?;
        let grpc_web_proxy = GrpcWebLayer::new().layer(proxy);
        // `nest_service` only accepts services whose error type is `Infallible`. The gRPC-Web proxy can fail
        // with upstream transport errors, so we map those failures into an HTTP 502 response to satisfy Axum's
        // trait bound and return a predictable client-facing error when an external backend is unavailable.
        let grpc_web_proxy = HandleError::new(
            grpc_web_proxy,
            |err: hyper_util::client::legacy::Error| async move {
                let err: &dyn std::error::Error = &err;
                let mut out = err.to_string();
                let mut cur = err.source();
                while let Some(src) = cur {
                    out.push_str(" | caused by: ");
                    out.push_str(&src.to_string());
                    cur = src.source();
                }
                error!(error = %err, error_chain = %out, "gRPC upstream unavailable");
                (StatusCode::BAD_GATEWAY, "Plugin gRPC upstream unavailable")
            },
        );
        Ok(router.nest_service("/api", grpc_web_proxy))
    }

    fn add_web(server_options: &ServerOptions, router: Router) -> Router {
        match &server_options.static_dir {
            Some(static_dir) => {
                let path = std::path::PathBuf::from(static_dir);
                let static_files =
                    ServeDir::new(path.clone()).fallback(ServeFile::new(path.join("index.html")));
                let web_service = get_service(static_files);
                router.fallback_service(web_service)
            }
            None => router,
        }
    }

    fn add_cors(router: Router) -> Router {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
            .expose_headers([
                http::HeaderName::from_static("grpc-accept-encoding"),
                http::HeaderName::from_static("grpc-encoding"),
                http::HeaderName::from_static("grpc-message"),
                http::HeaderName::from_static("grpc-status"),
                http::HeaderName::from_static("grpc-status-details-bin"),
                http::header::CONTENT_TYPE,
                http::header::SET_COOKIE,
            ]);
        router.layer(cors)
    }

    fn add_observability(router: Router) -> Router {
        router.layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &http::Request<_>| {
                    info_span!(
                    "http_request",
                    method = %req.method(),
                    uri = %req.uri(),
                    version = ?req.version()
                    )
                })
                .on_response(tower_http::trace::DefaultOnResponse::new().level(Level::INFO))
                .on_failure(tower_http::trace::DefaultOnFailure::new().level(Level::ERROR)),
        )
    }

    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(&self.options.http_address).await?;
        info!(
            "HTTP server bound: http://{}",
            listener.local_addr()?.to_string()
        );
        serve(listener, self.router.clone()).await?;
        Ok(())
    }
}
