#!/usr/bin/env bash
# Regenerates the synthetic fixture corpus with ffmpeg. Files are tiny (short, low-res) so they can be committed.
set -euo pipefail
cd "$(dirname "$0")"
V="-f lavfi -i testsrc2=size=64x48:rate=25:duration=1"
A="-f lavfi -i sine=frequency=440:sample_rate=48000:duration=1"
A44="-f lavfi -i sine=frequency=440:sample_rate=44100:duration=1"
FF="ffmpeg -hide_banner -loglevel error -y"

# chapters metadata
cat > /tmp/chapters.txt <<'C'
;FFMETADATA1
title=Fixture Title
[CHAPTER]
TIMEBASE=1/1000
START=0
END=500
title=Intro
[CHAPTER]
TIMEBASE=1/1000
START=500
END=1000
title=Outro
C
printf '1\n00:00:00,000 --> 00:00:00,500\nHello\n\n2\n00:00:00,500 --> 00:00:01,000\nWorld\n' > sub.srt
printf '[Script Info]\nScriptType: v4.00+\nPlayResX: 64\nPlayResY: 48\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,2,2,10,10,10,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:00.50,Default,,0,0,0,,Hello\nDialogue: 0,0:00:00.50,0:00:01.00,Default,,0,0,0,,World\n' > sub.ass
printf 'WEBVTT\n\n00:00.000 --> 00:00.500\nHello\n\n00:00.500 --> 00:01.000\nWorld\n' > sub.vtt

# Matroska family
$FF $V $A -i sub.srt -i /tmp/chapters.txt -map 0 -map 1 -map 2 -map_metadata 3 -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -c:s srt \
  -metadata:s:a:0 language=eng -metadata:s:s:0 language=jpn -metadata:s:v:0 title="Video Track" -disposition:s:0 default+forced h264_aac_srt.mkv
$FF $V $A -c:v libx265 -preset ultrafast -pix_fmt yuv420p10le -c:a flac -x265-params log-level=none hevc10_flac.mkv
$FF $V $A -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -b:v 50k -c:a libopus -b:a 32k vp9_opus.webm
$FF $V $A -c:v libvpx -deadline realtime -cpu-used 8 -b:v 50k -c:a libvorbis vp8_vorbis.webm
$FF $V -c:v libaom-av1 -cpu-used 8 -usage realtime -b:v 50k av1.mkv || $FF $V -c:v libsvtav1 -preset 12 av1.mkv
$FF $A -c:a aac -b:a 32k aac.mka
$FF -i sub.srt -c:s srt -f matroska srt.mks
$FF $V $A -c:v mpeg4 -q:v 10 -c:a libmp3lame -b:a 32k mpeg4_mp3.mkv
$FF $V $A -c:v mpeg2video -q:v 10 -c:a ac3 -b:a 64k mpeg2_ac3.mkv
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -r 25 -vf "fps=25" -x264-params "keyint=5" -c:a aac -metadata title="Flagged" -flags +bitexact -fflags +bitexact tags.mkv

# MP4 family
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -movflags +faststart h264_aac.mp4
$FF $V $A -c:v libx265 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -tag:v hvc1 -x265-params log-level=none hevc_aac.mp4
$FF $A -c:a aac -b:a 32k aac.m4a
$FF $A -c:a alac alac.m4a
$FF $V -c:v libx264 -preset ultrafast -pix_fmt yuv420p h264.m4v
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a pcm_s16le h264_pcm.mov
$FF $V $A -c:v mpeg4 -q:v 10 -c:a aac -b:a 32k mpeg4_aac.mp4
$FF $V $A -c:v h263 -s 176x144 -r 25 -c:a aac -b:a 32k h263_aac.3gp || true
$FF $V $A -i /tmp/chapters.txt -map 0 -map 1 -map_metadata 2 -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k chapters.mp4
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -movflags frag_keyframe+empty_moov fragmented.mp4

# AVI
$FF $V $A -c:v mpeg4 -vtag xvid -q:v 10 -c:a libmp3lame -b:a 32k xvid_mp3.avi
$FF $V $A -c:v mjpeg -q:v 10 -c:a pcm_s16le mjpeg_pcm.avi
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a ac3 -b:a 64k h264_ac3.avi
$FF $V $A -c:v rawvideo -pix_fmt yuv420p -c:a pcm_s16le raw_pcm.avi
$FF $V $A -c:v mpeg4 -q:v 10 -c:a libmp3lame -b:a 32k -f avi odml.avi

# Ogg
$FF $A -c:a libvorbis vorbis.ogg
$FF $A -c:a libopus -b:a 32k opus.opus
$FF $A -c:a flac flac.oga
$FF $V $A -c:v libtheora -q:v 3 -c:a libvorbis theora_vorbis.ogv
$FF $A -c:a libspeex speex.ogg || true

# Audio elementary streams
$FF $A -c:a flac flac.flac
$FF $A -c:a pcm_s16le wav16.wav
$FF $A -c:a pcm_s24le -ac 2 wav24.wav
$FF $A -c:a pcm_f32le -ac 6 wav_f32_6ch.wav
$FF $A -c:a pcm_s16le -rf64 always rf64.wav
$FF $A44 -c:a libmp3lame -b:a 64k mp3_cbr.mp3
$FF $A44 -c:a libmp3lame -q:a 9 -metadata title="MP3 Title" -metadata artist="Artist" -id3v2_version 3 mp3_vbr_id3v2.mp3
$FF $A -c:a mp2 -b:a 64k mp2.mp2
$FF $A -c:a aac -b:a 32k -f adts aac.aac
$FF $A -c:a aac -b:a 32k -f latm aac.latm || true
$FF $A -c:a ac3 -b:a 64k ac3.ac3
$FF $A -c:a eac3 -b:a 64k eac3.eac3
$FF $A -c:a dca -strict -2 -b:a 768k dts.dts
$FF $A -c:a mlp -strict -2 mlp.mlp || true
$FF $A -c:a truehd -strict -2 truehd.thd || true
$FF $A -c:a wavpack wavpack.wv
$FF $A -c:a tta tta.tta
$FF $A -c:a pcm_s16be aiff.aiff
$FF $A -c:a pcm_alaw -f au alaw.au
$FF $A -c:a ape ape.ape 2>/dev/null || true
$FF $A -c:a wmav2 -b:a 32k wma.wma
$FF $A -c:a pcm_s16le -f caf caf.caf
$FF $A44 -c:a libmp3lame -b:a 64k -write_xing 0 mp3_noxing.mp3
$FF $A -c:a ac3 -b:a 64k -f wav ac3_in_wav.wav || true
$FF $A -c:a amr_nb -ar 8000 -ac 1 -b:a 12.2k amr.amr || true
$FF $A -c:a libopus -b:a 32k -f ogg opus.ogg

# Video elementary streams
$FF $V -c:v libx264 -preset ultrafast -pix_fmt yuv420p -f h264 raw.h264
$FF $V -c:v libx265 -preset ultrafast -pix_fmt yuv420p -x265-params log-level=none -f hevc raw.h265
$FF $V -c:v mpeg2video -q:v 10 -f mpeg2video raw.m2v
$FF $V -c:v mpeg1video -q:v 10 -f mpeg1video raw.m1v
$FF $V -c:v mpeg4 -q:v 10 -f m4v raw.m4v
$FF $V -c:v libaom-av1 -cpu-used 8 -usage realtime -b:v 50k -f obu raw.obu || true
$FF $V -c:v rawvideo -pix_fmt yuv420p -f rawvideo raw.yuv
$FF $V -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -b:v 50k -f ivf vp9.ivf
$FF $V -c:v mjpeg -q:v 10 -f mjpeg raw.mjpeg
$FF $V -c:v dnxhd -s 1920x1080 -pix_fmt yuv422p -b:v 36M -r 25 -t 0.2 -f dnxhd raw.dnxhd || true

# WMV / ASF
$FF $V $A -c:v wmv2 -q:v 10 -c:a wmav2 -b:a 32k wmv2_wma.wmv
$FF $V $A -c:v msmpeg4 -q:v 10 -c:a wmav2 -b:a 32k msmpeg4.asf

# MPEG PS/TS
$FF $V $A -c:v mpeg1video -q:v 10 -c:a mp2 -b:a 64k -f mpeg mpeg1.mpg
$FF $V $A -c:v mpeg2video -q:v 10 -c:a ac3 -b:a 64k -f vob mpeg2_ac3.vob
$FF $V $A -c:v mpeg2video -q:v 10 -c:a mp2 -b:a 64k -f dvd dvd.mpg
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -f mpegts h264_aac.ts
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a ac3 -b:a 64k -f mpegts -mpegts_m2ts_mode 1 h264_ac3.m2ts
$FF $V $A -c:v mpeg2video -q:v 10 -c:a mp2 -b:a 64k -f mpegts mpeg2_mp2.ts
$FF $V $A -c:v libx265 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -x265-params log-level=none -f mpegts hevc_aac.ts
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -f mpegts -mpegts_flags +resend_headers -bsf:a aac_adtstoasc h264_latm.ts 2>/dev/null || true

# Others
$FF $V $A -c:v flv1 -q:v 10 -c:a libmp3lame -b:a 32k -ar 44100 flv1_mp3.flv
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k h264_aac.flv
$FF $V $A -c:v rv20 -b:v 50k -c:a ac3 -b:a 64k rv20_ac3.rm || $FF $V -c:v rv20 -b:v 50k rv20.rm || true
$FF $V $A -c:v mpeg2video -q:v 10 -c:a pcm_s16le -f mxf mpeg2_pcm.mxf || true
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -f nut h264.nut || true
$FF $V -c:v gif -f gif anim.gif
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -f matroska nonstandard_ext.bin
$FF $V $A -c:v mpeg4 -q:v 10 -c:a libmp3lame -b:a 32k -f avi ext_mismatch.mkv
$FF $V -c:v dvvideo -s 720x576 -pix_fmt yuv420p -r 25 -t 0.2 -f dv dv.dv || true
$FF $V $A -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k -f mp4 -brand mp42 -movflags +faststart brand_mp42.mp4

# Images
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 image.png
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -q:v 5 image.jpg
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 image.bmp
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 image.gif
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -c:v libwebp image.webp
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 image.tiff
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -pix_fmt gray image_gray.png
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -pix_fmt rgba image_rgba.png
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -c:v ljpeg -pix_fmt bgr24 image_ljpeg.jpg || true
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -c:v mjpeg -pix_fmt yuvj444p image_444.jpg
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -c:v mjpeg -pix_fmt yuvj420p -huffman optimal image_420.jpg
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -c:v mjpeg -pix_fmt yuvj420p -q:v 5 image_prog.jpg
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -c:v png -pix_fmt rgb24 -pred mixed image_rgb24.png
$FF -f lavfi -i testsrc2=size=64x48:rate=1:duration=1 -frames:v 1 -c:v jpeg2000 image.jp2 || true

# Misc / edge
head -c 4096 /dev/urandom > random.bin
: > empty.bin
printf 'just some text\n' > text.txt
rm -f /tmp/chapters.txt
ls -la | wc -l
