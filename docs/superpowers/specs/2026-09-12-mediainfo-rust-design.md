# mediainfo-rust — design

A cleanroom, pure-Rust reimplementation of MediaInfoLib's *behaviour* (not its code) as a library
crate plus a `mediainfo` command line tool. No GUI. Written from public format specifications and
black-box comparison against a reference `MediaInfo.so` (v20.08); no MediaInfoLib source is read.

## Goals

1. Drop-in replacement for the `MediaInfo_*` C API surface that AVDump3 uses:
   `New/Open/Close/Count_Get/Get/GetI/Option/Inform/Delete` — same stream kinds, same info kinds,
   same field names, same field *ordering* (per-kind schema), same string formats.
2. `mediainfo <file>` CLI with `--Full`, `--Output=Text|XML|JSON`, `--Inform=<template>`,
   `--Language=raw`, `--Version`, `--Help`, and multi-file/recursive input — matching the reference
   tool's output for supported files.
3. Zero non-std dependencies, so avdump-rust can depend on it without "weird dependencies".
4. Consumed by `avdump-rust` in place of `libloading` + system `libmediainfo`.

Non-goals: EBUCore/PBCore/FIMS/MPEG-7 outputs, HTML output, cover art extraction, trace/`--Details`,
network/URL input, DRM/encrypted content, every one of MediaInfo's ~150 formats. Formats are added
in priority order (below); unknown files yield a General stream with file info only.

## Architecture

```
mediainfo-rust/
  Cargo.toml              package "mediainfo-rust": lib "mediainfo", bin "mediainfo"
  src/
    lib.rs                pub API: MediaInfo, StreamKind, InfoKind, Stream, Field
    model/
      mod.rs              Stream (per-kind Vec<Option<String>> in schema order + dynamic tail)
      schema.rs           static field tables per kind: (name, measure, options) — from oracle schema
      labels.rs           field name -> Inform display label (English)
    finish/
      mod.rs              derived-field pass run after parsing (all */String*, lists, counts, bit rates)
      format.rs           number/size/duration/bitrate/framerate formatting helpers
      tables.rs           Format -> Info/Url/Extensions/Commercial/MIME; CodecID -> Info/Hint;
      language.rs         ISO 639 code -> name/String1..4
    io/
      mod.rs              Reader: random-access, buffered, bounded reads (never loads whole file)
      bits.rs             BitReader (msb-first, exp-golomb) + LE/BE helpers
    parsers/
      mod.rs              Parser trait { probe(header, ext) -> Score; parse(&mut Reader, &mut Doc) };
                          detection = try parsers in priority order, best probe score wins
      matroska.rs mp4.rs riff.rs (AVI/WAV/RF64) aiff.rs au.rs caf.rs ogg.rs asf.rs
      mpeg_ps.rs mpeg_ts.rs flv.rs rm.rs mxf.rs nut.rs ivf.rs dv.rs
      audio/ {mpeg_audio,id3,aac,ac3,dts,mlp,flac,wavpack,tta,ape,amr,vorbis,opus,speex,alac,pcm,wma}.rs
      video/ {avc,hevc,mpeg4v,mpegv,vp8,vp9,av1,theora,mjpeg,dnxhd,h263,flv1,rv,vc1,prores}.rs
      image/ {png,jpeg,gif,bmp,webp,tiff,jp2}.rs
      text/  {srt,ass,vtt,vobsub,plain}.rs
    output/
      text.rs xml.rs json.rs template.rs     Inform renderers
    cli.rs                argument parsing + main loop (used by src/main.rs)
  tests/
    fixtures/             synthetic corpus (generate.sh) — committed
    oracle/               reference dumps from MediaInfo.so: <file>.raw.txt (all fields),
                          <file>.txt (Inform), _schema.tsv, _labels.tsv, _Info_Parameters.txt
    compare.rs            per-fixture comparison against oracle with an allow-list of known diffs
```

### Data model

* `StreamKind { General, Video, Audio, Text, Other, Image, Menu }` — numeric values 0..6 as in the
  C API. `InfoKind { Name, Text, Measure, Options, NameText, MeasureText, Info, HowTo }`.
* `Stream { kind, values: Vec<Option<String>>, extra: Vec<Field> }` — `values` is indexed by the
  static schema position; `extra` holds dynamic fields (chapters, `ErrorDetectionType`, custom).
  `Count_Get(kind, i)` = schema length + extra length. `GetI` past the schema reads `extra`.
* `Doc { streams: [Vec<Stream>; 7], file: FileInfo }` is what parsers fill. Parsers set *base*
  fields only (`FileSize`, `Duration`, `Width`, `Format`, `CodecID`, `Language`, ...). The finish
  pass derives everything else (`FileSize/String*`, `Duration/String*`, `Width/String`, lists,
  counts, `OverallBitRate`, `StreamSize_Proportion`, `Format/Info`, ...). This mirrors the observed
  split in the reference and keeps parsers small.
* Base numeric fields are stored as MediaInfo stores them (e.g. `Duration` in ms, `FileSize` in
  bytes, `FrameRate` with 3 decimals, `BitRate` in bps).

### Detection

Each parser has `probe(&[u8] first 64 KiB, extension) -> u8` (0 = no, 100 = certain). Containers
check magic (`1A45DFA3`, `ftyp`, `RIFF`, `OggS`, `30 26 B2 75`, `00 00 01 BA`, `47` TS sync,
`FLV`, `.RMF`, ...). Elementary streams check sync words at offset 0 with at least two consecutive
valid frames. Extension only breaks ties. Text formats are probed last and must be valid UTF-8/16.

### Output

* Text: reference layout — per stream a header (`General`, `Video #1` when >1), then every field
  whose options[0]=='Y' and text is non-empty as `label padded to 41 + ": " + text`; blank line between
  streams; `--Full` shows all options[2]=='Y' fields (raw names with `/String` variants). `--Language=raw`
  uses field names as labels.
* XML: `<MediaInfo xmlns="https://mediaarea.net/mediainfo" version="2.0"><media ref=".."><track type="Video">`
  with elements named by field name (`/` and `(`, `)` mapped as the reference does), only fields with
  options[4]=='Y' (the "in XML" flag observed in the schema).
* JSON: same field selection as XML, `{"media":{"@ref":..,"track":[{"@type":"Video",...}]}}`.
* Template (`--Inform=`): `General;%FileName% %Duration/String%` syntax with `%Name%` substitution
  and per-kind sections separated by `\n` / `\r\n`, as the reference tool accepts.

### Error handling

Parsing never panics on malformed input: every read is bounds-checked through `Reader`, integer
decoding is saturating/checked, loops are bounded by file size and iteration caps. A parser error
degrades to "whatever was already extracted"; `open()` returns `true` when a format was recognised
(as the C API does) and `false` otherwise while still filling General with file info.

### Testing

* Unit tests per parser on hand-built byte arrays (bit readers, headers, exp-golomb, etc.).
* `tests/compare.rs`: for every fixture, run the library and diff the raw field set against
  `tests/oracle/<file>.raw.txt` for a curated set of *must-match* fields per kind (Format, Format_Profile,
  CodecID, Duration, Width, Height, FrameRate, BitRate, Channel(s), SamplingRate, BitDepth, Language,
  Default/Forced, Title, Encoded_Library, chapters). Fields outside the set are compared and reported
  as warnings, not failures. Fields known to be intentionally different are listed per fixture.
* Inform text comparison for a subset of fixtures where the whole block is expected to match.
* CLI tests: `--Version`, `--Output=XML|JSON` well-formedness, template, missing file exit code.

### Format priority

1. Matroska/WebM, MP4/MOV/3GP, AVI, Ogg, MPEG-PS/TS, ASF, FLV, WAV/AIFF/AU/CAF, IVF (containers)
2. AVC, HEVC, MPEG-4 Visual, MPEG-1/2 Video, VP8/VP9/AV1, Theora, MJPEG, H.263/FLV1/RV/VC-1/WMV, DV
3. AAC (ADTS/LATM/ASC), MPEG Audio + ID3, AC-3/E-AC-3, DTS, MLP/TrueHD, FLAC, Vorbis, Opus, Speex,
   PCM (all container flavours), ALAC, WMA, WavPack, TTA, APE, AMR
4. PNG, JPEG, GIF, BMP, WebP, TIFF, JP2; SRT, ASS/SSA, VTT, VobSub
5. RM, MXF (minimal), NUT (minimal)

## avdump-rust integration

Replace `src/info/providers/mediainfo.rs`'s `libloading` binding with the crate: keep `StreamKind`,
`InfoKind`, `MediaInfo` wrapper names so `MediaInfoLibProvider` and `xml_report` are unchanged
except for constructors. Remove `libloading` dependency and the `AVD3_MEDIAINFO` lookup; `--Version`
reports `MediaInfoLib: mediainfo-rust <version>`. Add the crate as a git dependency (path dependency
for local dev via `[patch]`). Update README.
