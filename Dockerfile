# syntax=docker/dockerfile:1
# Static, dependency-free `mediainfo` image: a single binary on `scratch`.
#   docker build -t mediainfo .
#   docker run --rm -v "$PWD:/data:ro" mediainfo /data/movie.mkv

# Built natively on each target platform (buildx runs the arm64 stage under QEMU); rust on Alpine
# links a static musl binary by default.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked && cp target/release/mediainfo /mediainfo

FROM scratch
COPY --from=build /mediainfo /mediainfo
COPY LICENSE /LICENSE
WORKDIR /data
ENTRYPOINT ["/mediainfo"]
CMD ["--Help"]
