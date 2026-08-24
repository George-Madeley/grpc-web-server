# grpc-web-server

A gRPC-Web proxy and web server implemented in Rust, with first-class Rust and C/C++ APIs.

## Why This Project Exists

This project solves a packaging and toolchain gap:

- Goal: provide a gRPC-Web proxy as a reusable library for C/C++ and Rust projects.
- Constraint: the Go gRPC-Web proxy is straightforward to run as an executable, but it is not practical to build and consume as a native library with MSVC toolchains.
- Approach: implement the proxy in Rust, then expose stable C-compatible interfaces.
- Result: you can use the same implementation as:
  - a standalone executable,
  - a Rust library,
  - or a C/C++ FFI library (including TLS certificate generation helpers).

## Solution Structure

Source layout is split by domain:

- `src/server/`
  - `proxy.rs`: upstream gRPC transport bridge.
  - `server.rs`: HTTP server, gRPC-Web routing, CORS policy, observability, TLS serving.
  - `handle.rs`: lifecycle handle (`start`, `stop`, `wait`, `is_running`) for background runtime management.
  - `ffi.rs`: C-compatible API for server configuration and lifecycle.
- `src/tls/`
  - `ca.rs`: certificate authority creation/writing.
  - `leaf.rs`: leaf certificate issuance/writing.
  - `ffi.rs`: C-compatible API for certificate generation.
- `src/bin/`
  - `grpc-web-server.rs`: standalone runtime executable.
  - `gen-certs.rs`: standalone certificate generation tool.
- `gen/include/`
  - `server.h`: generated C/C++ header for server FFI.
  - `tls.h`: generated C/C++ header for TLS FFI.

## Usage

### 1) Executable

Run the server directly:

```bash
cargo run --bin grpc-web-server -- \
  --http-address 127.0.0.1:8080 \
  --grpc-address 127.0.0.1:50051
```

Generate development certificates:

```bash
cargo run --bin gen-certs -- ./certs
```

### 2) Rust Library

Use the server API from Rust:

```rust
use grpc_web_server::server::server::{CorsPolicy, GrpcWebServer, GrpcWebServerOptions};

let options = GrpcWebServerOptions {
    http_address: "127.0.0.1:8080".to_string(),
    grpc_address: "127.0.0.1:50051".to_string(),
    static_dir: None,
    grpc_ca_cert: None,
    grpc_proxy_key: None,
    grpc_proxy_cert: None,
    http_key: None,
    http_cert: None,
    cors_policy: CorsPolicy::default(),
};

let server = GrpcWebServer::new(options)?;
server.start(std::future::pending()).await?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use TLS helpers from Rust:

```rust
use grpc_web_server::tls;
use rcgen::ExtendedKeyUsagePurpose;

let ca = tls::ca::make("grpc-web-dev-ca")?;
tls::ca::write(std::path::Path::new("./certs"), &ca)?;

let leaf = tls::leaf::issue(
    &ca,
    "grpc-server",
    vec!["localhost".to_string(), "127.0.0.1".to_string()],
    ExtendedKeyUsagePurpose::ServerAuth,
)?;
tls::leaf::write(std::path::Path::new("./certs"), "server", &leaf)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### 3) C/C++ FFI Library

Generated headers:

- `gen/include/server.h`
- `gen/include/tls.h`

Server lifecycle API (from `server.h`):

- `create`
- `start`
- `wait`
- `is_running`
- `stop`
- `destroy`

TLS API (from `tls.h`):

- `create_certificate_authority`
- `write_certificate_authority`
- `issue_leaf_certificate`
- `write_leaf_certificate`
- `destroy_certificate_authority`
- `destroy_leaf_certificate`

Ownership rule for all FFI handles:

- Any non-null handle returned by `create*` / `issue*` is owned by the caller.
- Release it exactly once with the corresponding `destroy*` function.

## CORS Configuration Through FFI

`GrpcWebCorsOptions` in `server.h` supports:

- origins: explicit list or wildcard (`allow_any_origin`),
- methods: explicit list or wildcard (`allow_any_method`),
- request headers: explicit list or wildcard (`allow_any_header`),
- credentials (`allow_credentials`).

Validation is strict:

- invalid origin/method/header tokens fail server creation,
- wildcard origin with credentials is rejected.

## CMake Integration

### Build This Project with CMake + Corrosion

This repository includes `CMakeLists.txt` and `CMakePresets.json` and uses Corrosion to import the Rust crate.

Typical flow:

```bash
cmake --preset "gRPC Web Server (Debug)"
cmake --build --preset "gRPC Web Server (Debug)"
```

or for release:

```bash
cmake --preset "gRPC Web Server (Release)"
cmake --build --preset "gRPC Web Server (Release)"
```

### Consume the Rust Library in Another CMake Project

If you want to embed this crate in another CMake project, add as a subdirectory:

```cmake
add_subdirectory(grpc-web-server)
```

Please note that the CMake code uses the Corrosion library to get CMake to build the Rust library.

### Linking Notes for C/C++

When using the generated FFI APIs from C/C++:

- include `server.h` and/or `tls.h`,
- link against the imported Rust static library target,
- ensure runtime dependencies for your platform/toolchain are available.

## Regenerating FFI Headers

Headers are generated via `build.rs` into `gen/include/` during Cargo builds.

If you change FFI signatures or structs, rebuild the crate:

```bash
cargo build
```

Then consume updated `server.h` and `tls.h` from `gen/include/`.
