#!/usr/bin/env python3
"""Builds the whole-song test set for the ignored tests (ADR 0025) with the OFFICIAL `wavpack` / `wvunpack` tools.

    python make_official_test_files.py OUT_DIR CD16.flac HIRES24_96K.flac HIRES24_44K.flac

Needs `wavpack` and `wvunpack` 5.9.0 (for example `scoop install wavpack`) and `ffmpeg` on the PATH. The three FLAC
files are a 16-bit song and two 24-bit songs (one at 96 kHz); their covers are embedded in every .wv.

It writes OUT_DIR/*.wv (+ .wvc for the two-file hybrid sets), OUT_DIR/src/*.wav (the originals as 16/24/32-bit integer
and 32-bit float WAV), OUT_DIR/ref/*.wav (what `wvunpack` decodes each file to, with and without the correction file)
and OUT_DIR/manifest.tsv, whose path is `MOE_WV_MANIFEST` for:

    cargo test -p player-wavpack --release --test official_encoder -- --ignored --nocapture
    cargo test -p player-tags --release --test wavpack_official -- --ignored --nocapture --test-threads=1
    cargo test -p player-platform-windows --lib -- official_encoder --ignored --nocapture
    MOE_WV_PLAY="a.wv;b.wv" cargo test -p player-audio-windows --release --lib real_wavpack -- --ignored --nocapture
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time

if len(sys.argv) != 5:
    sys.exit(__doc__)
OUT = os.path.abspath(sys.argv[1]).replace("\\", "/")
SONGS = {"cd16": sys.argv[2], "hires24_96k": sys.argv[3], "hires24_44k": sys.argv[4]}
SRC, REF = OUT + "/src", OUT + "/ref"
os.makedirs(SRC, exist_ok=True)
os.makedirs(REF, exist_ok=True)
LRC = OUT + "/lyrics.lrc"
with open(LRC, "w", encoding="utf-8", newline="\n") as lrc:
    lrc.write("[00:01.00]第一行：官方編碼器\n[00:02.50]Second line\n[00:04.00]第三行\n[00:06.00]Fourth line ♪\n")
ALONE = tempfile.mkdtemp(prefix="wv-alone-")  # the lone .wv copies live outside OUT so a library scan sees no duplicates


def run(args):
    result = subprocess.run(args, capture_output=True)
    if result.returncode != 0:
        sys.exit(f"FAILED {args}\n{result.stderr.decode('utf-8', 'replace')}")
    return result


# the originals: integer 16/24/32-bit and 32-bit float WAV, and each song's cover
KINDS = {"cd16": ["s16"], "hires24_96k": ["s24", "s32", "f32"], "hires24_44k": ["s24", "f32"]}
CODECS = {"s16": "pcm_s16le", "s24": "pcm_s24le", "s32": "pcm_s32le", "f32": "pcm_f32le"}
wavs, covers = {}, {}
for key, flac in SONGS.items():
    cover = f"{SRC}/{key}.jpg"
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", flac, "-an", "-c:v", "copy", cover],
                   capture_output=True)
    covers[key] = cover if os.path.exists(cover) and os.path.getsize(cover) > 0 else None
    for kind in KINDS[key]:
        wav = f"{SRC}/{key}_{kind}.wav"
        if not os.path.exists(wav):
            run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", flac, "-vn", "-c:a", CODECS[kind], wav])
        wavs[(key, kind)] = wav

# (name, song, source kind, wavpack options, lossless result?, lossy .wv + .wvc pair?)
JOBS = [
    ("cd16_lossless", "cd16", "s16", [], True, False),
    ("cd16_fast", "cd16", "s16", ["-f"], True, False),
    ("cd16_hh_x3", "cd16", "s16", ["-hh", "-x3"], True, False),
    ("cd16_hybrid256", "cd16", "s16", ["-b256"], False, False),
    ("cd16_hybrid256_wvc", "cd16", "s16", ["-c256"], True, True),
    ("cd16_hybrid96_dns", "cd16", "s16", ["-b96"], False, False),
    ("hires24_lossless", "hires24_96k", "s24", [], True, False),
    ("hires24_hh", "hires24_96k", "s24", ["-hh"], True, False),
    ("hires24_hybrid512", "hires24_96k", "s24", ["-b512"], False, False),
    ("hires24_hybrid512_wvc", "hires24_96k", "s24", ["-c512"], True, True),
    ("hires24_44k_lossless", "hires24_44k", "s24", [], True, False),
    ("int32_lossless", "hires24_96k", "s32", [], True, False),
    ("float32_lossless", "hires24_96k", "f32", [], True, False),
    ("float32_44k_lossless", "hires24_44k", "f32", [], True, False),
    ("float32_hybrid", "hires24_96k", "f32", ["-b768"], False, False),
]
rows = []
for name, key, kind, options, lossless, two_file in JOBS:
    wav, wv = wavs[(key, kind)], f"{OUT}/{name}.wv"
    for extension in (".wv", ".wvc"):
        if os.path.exists(OUT + "/" + name + extension):
            os.remove(OUT + "/" + name + extension)
    args = ["wavpack", "-q", "-y", "-m", "--allow-huge-tags"] + options
    args += ["--no-utf8-convert", "-w", f"Title={name}", "-w", "Artist=MoeTest", "-w", f"Album=WavPack {key}", "-w", "Year=2026"]
    args += ["-w", f"Lyrics=@{LRC}"]  # an LRC in the APEv2 `Lyrics` item (the UTF-8 file is stored as it is)
    if covers[key]:
        args += ["--write-binary-tag", f"Cover Art (Front)=@{covers[key]}"]
    started = time.time()
    run(args + [wav, wv])
    alone = f"{ALONE}/{name}.wv"
    shutil.copy(wv, alone)
    run(["wvunpack", "-q", "-y", "-w", alone, f"{REF}/{name}.wv-only.wav"])
    run(["wvunpack", "-q", "-y", "-w", wv, f"{REF}/{name}.wav"])
    if lossless:
        run(["wvunpack", "-q", "-v", "-m", wv])  # the official MD5 check of the lossless result
    print(f"{name}: {options} {time.time() - started:.1f}s, {os.path.getsize(wv) / 1048576:.1f} MiB", flush=True)
    rows.append((name, wv, wav, f"{REF}/{name}.wav", f"{REF}/{name}.wv-only.wav", "1" if lossless else "0",
                 "1" if two_file else "0"))
with open(OUT + "/manifest.tsv", "w", encoding="utf-8") as manifest:
    for row in rows:
        manifest.write("\t".join(row) + "\n")
shutil.rmtree(ALONE, ignore_errors=True)
print(f"{len(rows)} files; MOE_WV_MANIFEST={OUT}/manifest.tsv")
