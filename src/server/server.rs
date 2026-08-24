use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::{Router, error_handling::HandleError, routing::get_service, serve};
use axum_server::tls_rustls::RustlsConfig;
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

use super::proxy::GrpcWebProxy;

/// Configuration for one gRPC-Web server instance.
#[derive(Debug, Clone)]
pub struct GrpcWebServerOptions {
    /// The address to host the HTTP/1.1 proxy/web server on.
    pub http_address: String,
    /// Address of the upstream gRPC server that receives proxied requests.
    pub grpc_address: String,
    /// Directory containing static web files to serve.
    pub static_dir: Option<String>,
    /// CA certificate path used to verify the upstream gRPC server for mTLS.
    pub grpc_ca_cert: Option<PathBuf>,
    /// Client private-key path used for upstream gRPC mTLS.
    pub grpc_proxy_key: Option<PathBuf>,
    /// Client certificate path used for upstream gRPC mTLS.
    pub grpc_proxy_cert: Option<PathBuf>,
    /// Private-key path used to enable TLS for the HTTP server.
    pub http_key: Option<PathBuf>,
    /// Certificate path used to enable TLS for the HTTP server.
    pub http_cert: Option<PathBuf>,
}

impl GrpcWebServerOptions {
    /// Validates TLS and mTLS option combinations before server startup.
    ///
    /// # Returns
    /// Ok when all option dependencies are satisfied.
    ///
    /// # Errors
    /// - `grpc_ca_cert` is missing when gRPC mTLS options are partially set.
    /// - `grpc_proxy_key` is missing when gRPC mTLS options are partially set.
    /// - `grpc_proxy_cert` is missing when gRPC mTLS options are partially set.
    /// - `http_key` is missing when HTTP TLS options are partially set.
    /// - `http_cert` is missing when HTTP TLS options are partially set.
    pub fn validate(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.grpc_proxy_key.is_some() ^ self.grpc_proxy_cert.is_some() {
            if self.grpc_ca_cert.is_none() {
                return Err("grpc_ca_cert must be defined for gRPC mTLS".into());
            }
            if self.grpc_proxy_key.is_none() {
                return Err("grpc_proxy_key must be defined for gRPC mTLS".into());
            }
            if self.grpc_proxy_cert.is_none() {
                return Err("grpc_proxy_cert must be defined for gRPC mTLS".into());
            }
        }

        if self.http_key.is_some() ^ self.http_cert.is_some() {
            if self.http_cert.is_none() {
                return Err("http_cert must be defined for HTTP TLS".into());
            }
            if self.http_key.is_none() {
                return Err("http_key must be defined for mTLS".into());
            }
        }

        Ok(())
    }
}

/// Configured gRPC-Web server instance containing its router and options.
#[derive(Debug)]
pub struct GrpcWebServer {
    router: Router,
    options: GrpcWebServerOptions,
}

impl GrpcWebServer {
    /// Builds a configured server instance from validated options.
    ///
    /// # Arguments
    /// - `server_options`: Startup configuration for addresses, web assets, and optional TLS settings.
    ///
    /// # Returns
    /// Server instance when validation succeeds and routes are configured.
    ///
    /// # Errors
    /// - Any error returned by `ServerOptions::validate`.
    /// - Any error returned while constructing the upstream proxy service.
    pub fn new(server_options: GrpcWebServerOptions) -> Result<Self, Box<dyn std::error::Error>> {
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

    /// Mounts the gRPC-Web proxy service at `/api`.
    ///
    /// # Arguments
    /// - `server_options`: Source of upstream gRPC connection settings.
    /// - `router`: Router to extend with the proxy service.
    ///
    /// # Returns
    /// - Router instance with the `/api` proxy route mounted.
    ///
    /// # Errors
    /// - Upstream authority parsing fails.
    /// - TLS or certificate setup for the proxy fails.
    fn add_proxy(
        server_options: &GrpcWebServerOptions,
        router: Router,
    ) -> Result<Router, Box<dyn std::error::Error>> {
        let proxy = GrpcWebProxy::new(
            server_options.grpc_address.as_str(),
            server_options.grpc_ca_cert.as_deref(),
            server_options.grpc_proxy_cert.as_deref(),
            server_options.grpc_proxy_key.as_deref(),
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

    /// Configures static file hosting fallback when `static_dir` is set.
    ///
    /// # Arguments
    /// - `server_options`: Source of the optional static directory path.
    /// - `router`: Router to extend with static file fallback handling.
    ///
    /// # Returns
    /// - A router that either includes static file fallback handling or is returned unchanged.
    fn add_web(server_options: &GrpcWebServerOptions, router: Router) -> Router {
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

    /// Applies permissive CORS suitable for browser gRPC-Web clients.
    ///
    /// # Arguments
    /// - `router`: Router to wrap with CORS middleware.
    ///
    /// # Returns
    /// - A router with CORS middleware attached.
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

    /// Attaches request tracing middleware for observability.
    ///
    /// # Arguments
    /// - `router`: Router to wrap with tracing middleware.
    ///
    /// # Returns
    /// - A router with HTTP request tracing enabled.
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

    /// Runs the HTTP/HTTPS server until it exits or a shutdown signal resolves.
    ///
    /// # Arguments
    /// - `shutdown_signal`: Future that resolves when shutdown should begin.
    ///
    /// # Returns
    /// Ok when the server exits normally or shutdown is requested.
    ///
    /// # Errors
    /// - `http_address` cannot be parsed into a socket address.
    /// - HTTP listener binding fails.
    /// - HTTPS certificate or key loading fails.
    /// - HTTP or HTTPS serving fails with an I/O or runtime server error.
    pub async fn start<F>(&self, shutdown_signal: F) -> Result<(), Box<dyn std::error::Error>>
    where
        F: Future<Output = ()> + Send,
    {
        let addr: SocketAddr = self.options.http_address.parse()?;
        tokio::pin!(shutdown_signal);

        if let (Some(cert_path), Some(key_path)) = (
            self.options.http_cert.as_deref(),
            self.options.http_key.as_deref(),
        ) {
            let tls_config = RustlsConfig::from_pem_file(cert_path, key_path).await?;
            info!("HTTPS server bound: https://{}", addr);
            let server = axum_server::bind_rustls(addr, tls_config)
                .serve(self.router.clone().into_make_service());
            tokio::pin!(server);
            tokio::select! {
                res = &mut server => {
                    res?;
                }
                _ = &mut shutdown_signal => {
                    info!("Shutdown signal received for HTTPS server");
                }
            }
            return Ok(());
        }

        let listener = TcpListener::bind(addr).await?;
        info!("HTTP server bound: http://{}", listener.local_addr()?);
        let server = serve(listener, self.router.clone()).into_future();
        // Pins a future to a stable memory location on the stack. Needed when a future may be polled multiple times and
        // is no Unpin. This enables passing mutable references to select safely.
        tokio::pin!(server);
        // Waits on multiple async branches concurrently. FIrst branch that completes wins; other branches are
        // cancelled. Either the server exits, or shutdown signal arrives.
        tokio::select! {
            res = &mut server => {
                res?;
            }
            _ = &mut shutdown_signal => {
                info!("Shutdown signal received for HTTP server");
            }
        }
        Ok(())
    }
}
