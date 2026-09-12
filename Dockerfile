# syntax=docker/dockerfile:1
# Static, dependency-free `mediainfo` image: a single binary on `scratch`.
#   docker build -t mediainfo .
#   docker run --rm -v "$PWD:/data:ro" mediainfo /data/movie.mkv

FROM --platform=$BUILDPLATFORM rust:1-alpine AS build
ARG TARGETARCH
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-musl ;; \
      arm64) target=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH $TARGETARCH" && exit 1 ;; \
    esac \
    && rustup target add "$target" \
    && cargo build --release --locked --target "$target" \
    && cp "target/$target/release/mediainfo" /mediainfo

FROM scratch
COPY --from=build /mediainfo /mediainfo
COPY LICENSE /LICENSE
WORKDIR /data
ENTRYPOINT ["/mediainfo"]
CMD ["--Help"]
