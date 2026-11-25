#!/bin/sh
# His words as he speaks, from the phone's own listening model, measured on this Mac's GPU or Neural Engine:
#   AIRBAND_MODEL_DIR=<the model's folder, as the phone fetches it> tools/phonon-live.sh gpu|ane [<speech.wav>…]
# The app's NeuralEngineListening.swift is built for the Mac (tools/phonon-live, never part of the app) and run
# under the core's own listening (crates/airband-radio/src/ears/phonon_live.rs), with a recording fed as fast
# as it was spoken, as dictation: the time to the first words shown, how often they change, how far behind him
# they are, and the words he is taken to have said. Each recording is 16 kHz 16-bit mono; with none given, two
# are spoken by the Mac's voice (7 s, and 24 s, which is heard in windows as a phone hears anything over 15 s).
#
# Core ML will not load a model from an exFAT disk, so the model is copied into an APFS image beside the build
# and mounted at the same place each run: Core ML keeps what it compiled for a model by where it is, and the
# first run on a Mac compiles it (minutes on the Neural Engine, longer on the GPU), as a phone does when it gets
# the model ready.
set -eu
cd "$(dirname "$0")/.."
units=${1:-}
case "$units" in gpu|ane) shift ;; *) echo "usage: AIRBAND_MODEL_DIR=<folder> tools/phonon-live.sh gpu|ane [<speech.wav>…]" >&2; exit 1 ;; esac
[ -n "${AIRBAND_MODEL_DIR:-}" ] || { echo "AIRBAND_MODEL_DIR is to name the model's folder" >&2; exit 1; }
model=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["model"])' "$AIRBAND_MODEL_DIR/manifest.json")
build="${CARGO_TARGET_DIR:-target}/phonon-live"
mkdir -p "$build"

# the package the app builds with, and no other
app=$(ruby -ryaml -e 'puts YAML.load_file(ARGV[0])["packages"]["PhononCoreML"]["exactVersion"]' apps/apple/project.yml)
here=$(swift package --package-path tools/phonon-live dump-package | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["dependencies"][0]["sourceControl"][0]["requirement"]["exact"][0])')
[ "$app" = "$here" ] || { echo "tools/phonon-live builds PhononCoreML $here and the app $app: make them the same" >&2; exit 1; }
swift build -c release --package-path tools/phonon-live --scratch-path "$build/swift"
lib="$build/swift/release/libPhononHost.dylib"

# the model on APFS
image="$build/model.sparsebundle"
[ -d "$image" ] || hdiutil create -size 2g -type SPARSEBUNDLE -fs APFS -volname airband-phonon-live "$image" >/dev/null
mnt="${TMPDIR:-/tmp}/airband-phonon-live"
mkdir -p "$mnt"
hdiutil attach -nobrowse -mountpoint "$mnt" "$image" >/dev/null
trap 'hdiutil detach "$mnt" >/dev/null' EXIT
rsync -a --delete "$AIRBAND_MODEL_DIR/" "$mnt/$model/"

if [ $# -eq 0 ]; then
  say -o "$build/short.wav" --data-format=LEI16@16000 --channels=1 "Hey shop two, can you run the release script again and tell me which of the bottles failed to upload? Then open a pull request."
  say -o "$build/long.wav" --data-format=LEI16@16000 --channels=1 "Okay, here is what I want for the listening work today. First, check that the phone shows my words while I am still talking, not only at the end. Second, measure how long the first words take to appear, and how often they update after that. Third, if anything lags badly, find the cause in the core and fix it there. When you are done, send me the numbers, the list of files you changed, and the test counts."
  set -- "$build/short.wav" "$build/long.wav"
fi
# all of them in one run, heard one after another by the model got ready once, as on a phone
wavs=$(printf '%s:' "$@")
PHONON_LIVE_LIB="$lib" PHONON_LIVE_UNITS="$units" AIRBAND_MODEL="$model" AIRBAND_MODEL_DIR="$mnt/$model" SPEECH_WAV="${wavs%:}" \
  cargo test -q --release -p airband-radio --lib phonon_live -- --ignored --nocapture
