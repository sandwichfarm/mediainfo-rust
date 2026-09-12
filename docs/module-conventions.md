# Writing a parser module

This crate is a cleanroom reimplementation of MediaInfo. **Never read MediaInfoLib source code.**
Work from public format specifications and from the black-box reference dumps under `tests/oracle/`.

## Layout

* `src/model` — `Stream` (fields by name, schema order), `Doc` (all streams), `StreamKind`.
* `src/io` — `Reader` (bounded random access: `read_at`, `read_u32be`, `peek`, `seek`…),
  `bits::BitReader` (MSB-first, `bits(n)`, `ue()`, `se()`), slice helpers `be16/le32/...`, `cstr`, `utf16`.
* `src/finish` — derivation pass. **Parsers set base fields only**; every `*/String*`, list, count,
  `OverallBitRate`, `Format/Info`, `Format/Url`, `Language/String*`, `Channel(s)/String`, `Bits-(Pixel*Frame)`,
  `StreamSize_Proportion`, `FrameCount` (video, from Duration×FrameRate) etc. is filled in by `finish`.
  Lookup tables for Format → Info/Url/Extensions/Commercial/MIME and CodecID → Info/Hint/Url live in
  `src/finish/tables.rs` — add entries there when a format you produce is missing.
* `src/parsers/mod.rs` — registry (`FORMATS`), `Probe { head (first 64 KiB), ext (lower-case), size }`.
* `src/parsers/<container>.rs`, `src/parsers/{audio,video,image,text}/<codec>.rs`.

## Contract of a module

Signatures already exist as stubs (`//! TODO` files); keep them, they are called by other modules:

* `probe(&Probe) -> u8` — 0 = not this format, 100 = certain. Magic at offset 0 with a plausible header
  → 90–100; extension only → 20–40. Elementary streams must verify at least two consecutive valid frames.
* `parse(&mut Reader, &mut Doc) -> bool` — fill `doc.general()` (`Format`, `Duration`, `OverallBitRate` when
  known, tags…) and push streams with `doc.add(kind)` / `doc.streams[kind as usize].push(stream)`.
  Return `false` only if the file turns out not to be this format (the next candidate is tried).
* `apply_*(&mut Stream, &[u8]) -> bool` — fill a stream from a codec configuration or frame; `true` if parsed.

## Field conventions (match the reference exactly — check `tests/oracle/<fixture>.raw.txt`)

* Numbers are stored as plain strings: `Width` `"1920"`, `Duration` in **milliseconds** (`"1021"`, or
  `"1000.000000"` when the source is exact), `BitRate` in bps, `SamplingRate` in Hz, `FrameRate` with 3
  decimals (`"25.000"`), ratios with 3 decimals (`"1.333"`).
* `Format` names: `AVC`, `HEVC`, `MPEG-4 Visual`, `MPEG Video`, `VP8`, `VP9`, `AV1`, `Theora`, `AAC`
  (+`Format_AdditionalFeatures` `LC`/`HE-AAC`…), `MPEG Audio` (+`Format_Profile` `Layer 3`), `AC-3`, `E-AC-3`,
  `DTS`, `MLP FBA` (TrueHD), `MLP`, `FLAC`, `Vorbis`, `Opus`, `PCM`, `ALAC`, `WMA`, `WavPack`, `TTA`,
  `Monkey's Audio`, `AMR`, `PNG`, `JPEG`, `GIF`, `Bitmap`, `TIFF`, `WebP`, `JPEG 2000`, `SubRip`, `ASS`,
  `SSA`, `WebVTT`, `VobSub`, `UTF-8`. Containers: `Matroska`, `WebM`, `MPEG-4`, `QuickTime`, `AVI`, `Wave`,
  `AIFF`, `AU`, `CAF`, `Ogg`, `Windows Media`, `MPEG-PS`, `MPEG-TS`, `BDAV`, `Flash Video`, `RealMedia`,
  `MXF`, `Nut`, `IVF`, `DV`, `ADTS`, `LATM`.
* `Format_Profile` strings: AVC `Baseline@L1`, HEVC `Main 10@L1@Main`, MPEG-4 Visual `Simple@L1`,
  MPEG Video `Main@Main`, AV1 `Main@L2.0`, MPEG Audio `Layer 3`, AMR `Narrow band`. `Format_Version`:
  MPEG Audio `Version 1`, MPEG Video `Version 2`, GIF `89a`.
* `Format_Settings*`: AVC `CABAC / 4 Ref Frames`, PCM `Little / Signed`, MPEG Audio `Joint stereo` (`Format_Settings_Mode`),
  AAC `Format_Settings_SBR` `No (Explicit)`/`Yes`.
* `CodecID`: container-specific (`V_MPEG4/ISO/AVC`, `avc1`, `mp4a-40-2`, `55`, `1`, `A_AAC-2`, `27`...).
* Audio: `Channel(s)`, `ChannelPositions` (`Front: L C R, Back: L R, LFE`), `ChannelLayout` (`L R C LFE Lb Rb`) —
  helpers in `src/parsers/audio/mod.rs` (`layout_for_count`, `layout_from_mask`); `SamplesPerFrame`
  (1024 AAC, 1152 MP3, 1536 AC-3, 960/1920 Opus at 48k…) — `finish` derives `FrameRate` from it;
  `BitRate_Mode` `CBR`/`VBR`; `Compression_Mode` `Lossy`/`Lossless`; `BitDepth` for PCM/lossless only.
* Video: `Width`, `Height`, `PixelAspectRatio` **or** `DisplayAspectRatio` (finish computes the other),
  `FrameRate`, `FrameRate_Mode` (`CFR`/`VFR`), `ColorSpace` `YUV`/`RGB`, `ChromaSubsampling` `4:2:0`,
  `BitDepth`, `ScanType` `Progressive`/`Interlaced`, `ScanOrder` `TFF`/`BFF`, `Standard` `PAL`/`NTSC`,
  colour metadata as dynamic fields via `s.set_extra("colour_primaries", ..., "", "Y YTY")` (names from
  `src/parsers/video/colour.rs`).
* Encoder info: `Encoded_Library` (`x264 - core 165 r3222 b35605a`, `LAME3.100`, `Lavc63.1.101`),
  `Encoded_Library_Settings` (`a=1 / b=2`), `Encoded_Application` (writing application).
* Text streams: `Format`, `CodecID`, `Duration`, `Language`, `Title`, `Default`, `Forced`.
* Images: `Format`, `Width`, `Height`, `ColorSpace` (`RGB`/`YUV`/`Y`), `ChromaSubsampling`, `BitDepth`,
  `Compression_Mode`, `Format_Profile` (JPEG: `Baseline`/`Progressive`; GIF: `89a`).
* Elementary streams: `General.Format` = stream format; `General.Duration`/`OverallBitRate` when the
  stream knows its duration. For MP3 with ID3 the tags go to General (`Title`, `Performer`, `Album`,
  `Track/Position`, `Genre`, `Recorded_Date`, `Comment`, `Encoded_Library`...).
* Dynamic (non-schema) fields: `s.set_extra(name, value, measure, options)` with options `"Y YTY"`
  (shown in text report) or `"N YTY"` (hidden, exported to XML). Do not invent fields the reference
  does not show.

## Verifying

```
cargo run -q --example compare <fixture-name-substring> [-v]   # diff against tests/oracle/*.raw.txt
cargo run -q -- tests/fixtures/<file>                          # text report
cargo test
```

`tests/oracle/<fixture>.txt` is the reference text report; `<fixture>.raw.txt` lists every field
(index, name, text, measure, options). Aim for all *base* fields of your streams matching; derived
`/String` differences usually mean a base field is wrong (or `finish` needs a rule — say so in your
report rather than hacking around it in the parser).

## Rules

* **No new crate dependencies. No `unsafe`. No panics on any input**: bounds-check every read
  (`Reader` and slice helpers return `Option`), cap loops by file size and by an iteration limit, never
  allocate from an untrusted length without capping it (`min(16 << 20)`).
* Never read the whole file when a header suffices; for frame counting scan at most a few MiB from the
  start (and, if useful, the end) and extrapolate.
* Unit tests on hand-built byte arrays for every header parser (`#[cfg(test)] mod tests`).
* Keep to your assigned files. If you need a change elsewhere (a table entry in `finish/tables.rs`, a
  helper in `io`), describe it in your final report instead of editing shared files, unless told otherwise.
* `cargo build` must stay warning-free; run `cargo fmt` is not configured — match the surrounding style
  (4-space indent, `//!` module doc, short `///` docs on public functions).
