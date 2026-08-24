cmake_minimum_required(VERSION 3.23)
project(GrpcWebServer LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)
FetchContent_Declare(
  Corrosion
  GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
  GIT_TAG v0.6
)
FetchContent_MakeAvailable(Corrosion)

corrosion_import_crate(MANIFEST_PATH ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml)

if(TARGET grpc_web_server)
  set(GRPC_WEB_SERVER_RUST_TARGET grpc_web_server)
elseif(TARGET grpc-web-server)
  set(GRPC_WEB_SERVER_RUST_TARGET grpc-web-server)
else()
  message(FATAL_ERROR "Could not find imported Rust target from Corrosion.")
endif()

add_library(GrpcWebServer ALIAS ${GRPC_WEB_SERVER_RUST_TARGET})

target_sources(${GRPC_WEB_SERVER_RUST_TARGET} INTERFACE
  FILE_SET HEADERS
  BASE_DIRS ${CMAKE_CURRENT_SOURCE_DIR}/gen/include
  FILES
  ${CMAKE_CURRENT_SOURCE_DIR}/gen/include/server.h
  ${CMAKE_CURRENT_SOURCE_DIR}/gen/include/tls.h
)