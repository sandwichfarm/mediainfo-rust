# mediainfo-rust

Pure-Rust [MediaInfo](https://mediaarea.net/en/MediaInfo): library + `mediainfo` CLI, no GUI, no
dependencies. A cleanroom reimplementation (written from format specs and MediaInfoLib's output, not
its source) that keeps MediaInfoLib's field names, ordering, string formats and text/XML/JSON reports.

## Install

```
cargo install mediainfo-rust                                   # crates.io
docker run --rm -v "$PWD:/data:ro" ghcr.io/sandwichfarm/mediainfo movie.mkv
```

Prebuilt binaries (Linux, macOS, Windows) are on the [releases page](https://github.com/sandwichfarm/mediainfo-rust/releases).

## Use

```
mediainfo movie.mkv                          # text report
mediainfo --Output=JSON movie.mkv            # or XML
mediainfo --Full movie.mkv                   # every field, raw names
mediainfo --Inform="General;%FileName% %Duration/String%" *.mkv
mediainfo -R /media                          # recursive
```

```rust
let mut mi = mediainfo::MediaInfo::new();
if mi.open("movie.mkv") {
    let width = mi.get(StreamKind::Video, 0, "Width", InfoKind::Text);
    println!("{}", mi.inform());
}
```

`get`/`get_i`/`count_get`/`option`/`inform` behave like the `MediaInfo_*` C API.

## Formats

Matroska/WebM, MP4/MOV/3GP, AVI, WAV, AIFF, AU, CAF, Ogg, ASF, MPEG-PS/TS, FLV, RealMedia, MXF, Nut,
IVF, DV · AVC, HEVC, MPEG-1/2/4, VP8/9, AV1, Theora, H.263, VC-1, ProRes, DNxHD, MJPEG · AAC, MP1/2/3,
AC-3/E-AC-3, DTS, TrueHD, FLAC, Vorbis, Opus, Speex, PCM, ALAC, WMA, WavPack, TTA, APE, AMR ·
PNG, JPEG, GIF, BMP, TIFF, WebP, JP2 · SRT, ASS/SSA, VTT, VobSub.

## Benchmark

Wall-clock per file, median of 9 warm-cache runs, i7-11700K, Linux. MediaInfoLib 20.08 is driven
through its C API (`tools/oracle`), the same work the `mediainfo` CLI does. `scripts/bench.sh`
reproduces the table.

| File | Size | MediaInfoLib 20.08 | mediainfo-rust | Speed-up |
|---|---|---|---|---|
| HandBrake MKV (AVC + AAC, chapters) | 34 MB | 12.4 ms | 7.5 ms | 1.6× |
| VFR MP4 (AVC + AAC, chapters) | 25 MB | 12.9 ms | 1.7 ms | 7.6× |
| 2 min 720p MKV (AVC + AAC) | 122 MB | 43.5 ms | 8.2 ms | 5.3× |
| MP4 (AVC + AAC) | 75 MB | 22.0 ms | 2.8 ms | 8.0× |
| MPEG-TS (AVC + AAC) | 16 KB | 9.9 ms | 1.4 ms | 7.2× |
| AVI (Xvid + MP3) | 20 KB | 8.6 ms | 1.4 ms | 6.0× |
| MP3 with ID3v2 | 8 KB | 5.6 ms | 1.4 ms | 4.0× |
| JPEG | 4 KB | 4.7 ms | 1.3 ms | 3.5× |

## Develop

```
cargo test                          # unit, CLI, robustness and reference-comparison tests
cargo run --example compare [name]  # field-by-field diff against tests/oracle (MediaInfoLib 20.08 dumps)
docker build -t mediainfo .
```

Releases: push a `vX.Y.Z` tag (matching `Cargo.toml`) — CI publishes to crates.io
(`CARGO_REGISTRY_TOKEN` secret), GHCR and the releases page.

## License

BSD-2-Clause.
