# mediainfo-rust

A cleanroom, pure-Rust reimplementation of [MediaInfo](https://mediaarea.net/en/MediaInfo): a library
and a command line tool that report the container and stream properties of media files — no GUI,
no C++ dependency, no runtime library to install.

The output is designed to match MediaInfoLib's: the same stream kinds, field names, field order and
string formats (`10.0 KiB`, `1 s 21 ms`, `80.6 kb/s`, `Baseline@L1`, …), the same text report layout,
and the same `MediaInfo_*` API shape (`Open`, `Count_Get`, `Get`, `GetI`, `Option`, `Inform`), so it can
stand in for the C library. It was written from public format specifications and black-box comparison
against MediaInfoLib's output; no MediaInfoLib source was consulted.

```
$ mediainfo movie.mkv
General
Unique ID                                : 146509503074622442766829437946688180369 (0x6E38B4…)
Complete name                            : movie.mkv
Format                                   : Matroska
Format version                           : Version 4
File size                                : 33.9 MiB
Duration                                 : 4 min 23 s
Overall bit rate                         : 1 082 kb/s
…
```

## Building

```
cargo build --release      # target/release/mediainfo
cargo test                 # unit, CLI and reference-comparison tests
```

There are no dependencies outside the Rust standard library.

## Command line

```
mediainfo [options] FILE...
  --Full, -f            all fields (raw names included)
  --Output=XML|JSON     MediaInfo XML 2.0 / JSON instead of text
  --Inform=TEMPLATE     e.g. "General;%FileName% %Duration/String%\nVideo;%Width%x%Height%"
  --Language=raw        raw field names as labels
  --Recursive, -R       descend into directories
  --Info-Parameters     list every field name
  --Version, --Help
```

## Library

```rust
use mediainfo::{MediaInfo, StreamKind, InfoKind};

let mut mi = MediaInfo::new();
if mi.open("movie.mkv") {
    let width = mi.get(StreamKind::Video, 0, "Width", InfoKind::Text);
    let audio_streams = mi.count_get(StreamKind::Audio, None);
    mi.option("Output", "JSON");
    println!("{}", mi.inform());
}
```

`Get`/`GetI` return the same names, texts, measures and option flags as MediaInfoLib; fields beyond
the static schema (chapters, `ErrorDetectionType`, colour metadata …) are appended per stream exactly
like the reference does, so `Chapters_Pos_Begin`/`Chapters_Pos_End` iteration works unchanged.

## Formats

Containers: Matroska/WebM, MP4/MOV/3GP (incl. fragmented), AVI (OpenDML), WAV/RF64, AIFF, AU, CAF, Ogg,
ASF/WMV/WMA, MPEG-PS/VOB, MPEG-TS/M2TS, FLV, RealMedia, MXF, Nut, IVF, DV.
Video: AVC, HEVC, MPEG-1/2 Video, MPEG-4 Visual, VP8, VP9, AV1, Theora, H.263, VC-1/WMV, ProRes,
DNxHD/VC-3, DV, Motion JPEG, raw video.
Audio: AAC (ADTS/LATM/MP4), MPEG Audio (MP1/2/3 with ID3/Xing/LAME), AC-3, E-AC-3, DTS, TrueHD/MLP,
FLAC, Vorbis, Opus, Speex, PCM (all container flavours), ALAC, WMA, WavPack, TTA, Monkey's Audio, AMR.
Images: JPEG, PNG, GIF, BMP, TIFF, WebP, JPEG 2000. Text: SubRip, ASS/SSA, WebVTT, VobSub.

Unknown files still yield a General stream with file information.

## Tests against the reference

`tests/fixtures/` holds a small synthetic corpus (`generate.sh` recreates it with ffmpeg) and
`tests/oracle/` the corresponding dumps of every field produced by MediaInfoLib 20.08.
`cargo run --example compare [name]` diffs this implementation against those dumps field by field;
`tests/compare.rs` asserts the important fields for every fixture.

## Status

Not everything MediaInfo does is implemented — see `docs/superpowers/specs/` for the design and the
list of non-goals (EBUCore/PBCore outputs, trace mode, DRM, network input, and the long tail of exotic
formats). Bug reports with a small sample file are welcome.

## License

BSD-2-Clause.
