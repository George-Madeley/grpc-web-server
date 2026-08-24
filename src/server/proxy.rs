use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::task::{Context, Poll};

use http::{Request, Response, Uri};
use hyper::body::Incoming;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rustls::{ClientConfig, RootCertStore};
use rustls_pemfile::{certs, private_key};
use tonic::body::Body as TonicBody;
use tower::Service;
use tracing::debug;

/// gRPC-Web to gRPC proxy for external gRPC servers
///
/// `GrpcWebProxy` is the transport bridge used by the host to forward requests from an HTTP/1.1 gRPC-Web
/// endpoint into an external backend gRPC server over HTTP/2.
///
/// The proxy is intentionally payload-agnostic:
/// - It does not decode protobuf messages.
/// - It preserves request headers/body and response body/trailers.
/// - It only rewrites the request URI authority/scheme to target the selected plugin backend.
///
/// `GrpcWebLayer` is applied in [`new`](Self::new), so callers receive a ready-to-mount gRPC-Web service.
#[derive(Clone)]
pub struct GrpcWebProxy {
    /// HTTP/2 client used to reach plugin upstreams.
    client: Client<hyper_rustls::HttpsConnector<HttpConnector>, TonicBody>,
    /// Upstream authority in host:port form.
    authority: http::uri::Authority,
    use_tls: bool,
}

impl GrpcWebProxy {
    /// Builds a gRPC-Web capable proxy service.
    ///
    /// # Arguments
    /// - `authority`: Upstream plugin gRPC endpoint as `host:port` (for example `127.0.0.1:50051`).
    ///
    /// # Returns
    /// A `GrpcWebService<Self>` ready to mount in an Axum router.
    ///
    /// # Notes
    /// This constructor enforces an HTTP/2-only client for upstream communication.
    /// CORS and error-to-response adaptation are intentionally left to the caller's outer middleware stack.
    ///
    /// # Errors
    /// Returns an error when `authority` is not a valid HTTP authority value.
    pub fn new(
        authority: &str,
        ca_cert: Option<&Path>,
        proxy_cert: Option<&Path>,
        proxy_key: Option<&Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let authority = authority.parse::<http::uri::Authority>()?;

        let mut roots = RootCertStore::empty();
        let use_tls = if let Some(path) = ca_cert {
            let mut reader = BufReader::new(File::open(path)?);
            let parsed = certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
            let (added, _) = roots.add_parsable_certificates(parsed);
            if added == 0 {
                return Err("no CA certs loaded from grpc_ca_cert".into());
            }
            true
        } else {
            false
        };

        let tls_config = ClientConfig::builder().with_root_certificates(roots);
        let tls_config = if let Some(proxy_cert) = proxy_cert
            && let Some(proxy_key) = proxy_key
        {
            let mut cert_reader = BufReader::new(File::open(proxy_cert)?);
            let cert_parsed = certs(&mut cert_reader).collect::<Result<Vec<_>, _>>()?;

            let mut key_reader = BufReader::new(File::open(proxy_key)?);
            let key_parsed = private_key(&mut key_reader)?
                .ok_or(format!("Failed to read {}", proxy_key.display()))?;

            tls_config.with_client_auth_cert(cert_parsed, key_parsed)?
        } else {
            tls_config.with_no_client_auth()
        };

        let mut http = HttpConnector::new();
        http.enforce_http(false);

        let https = HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_or_http()
            .enable_http2()
            .wrap_connector(http);

        let client = Client::builder(TokioExecutor::new())
            .http2_only(true)
            .build(https);

        Ok(Self {
            client,
            authority,
            use_tls,
        })
    }
}

impl Service<Request<TonicBody>> for GrpcWebProxy {
    type Response = Response<Incoming>;
    type Error = hyper_util::client::legacy::Error;
    type Future = <Client<hyper_rustls::HttpsConnector<HttpConnector>, TonicBody> as Service<
        Request<TonicBody>,
    >>::Future;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // The underlying hyper client is cloneable and immediately ready for dispatch.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<TonicBody>) -> Self::Future {
        let (mut parts, body) = req.into_parts();

        let path_and_query = parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");

        let scheme = if self.use_tls { "https" } else { "http" };

        debug!(
            method = %parts.method,
            path = %path_and_query,
            upstream = %self.authority,
            scheme = %scheme,
            "Forwarding request to gRPC upstream"
        );

        let upstream_uri = Uri::builder()
            .scheme(scheme)
            .authority(self.authority.as_str())
            .path_and_query(path_and_query)
            .build()
            .expect("valid upstream URI");

        // Preserve method, headers, and path while retargeting the request to the external server backend.
        parts.uri = upstream_uri;

        // Forward raw body frames so tonic-web can translate framing/trailers for browser clients.
        self.client.call(Request::from_parts(parts, body))
    }
}
