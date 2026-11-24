#!/usr/bin/env python3
"""End to end, for the phone: the app's own radio core (its Desk), in the simulator, against a tower and a relay
on this machine.

    export CARGO_TARGET_DIR=target/stand-ins
    cargo build --release -p airband-cli -p airband-relay --features airband-cli/stand-ins && tools/e2e-phone.py
                                                       a few minutes the first time (the app is built), no model calls

Listening (ListeningEndToEndTests): a recording of this Mac's voice is given to the app's core as its microphone
would give it. The listening model on the phone is fetched from the catalogue's address and run by the app's own
engine on the simulator's CPU; every listening service in the catalogue is stood in for on this machine by
tools/listening_stand_ins.py, each speaking its own protocol, which the phone's core reaches in their place only
because the app is built for this run with the `stand-ins` feature (into the stand-ins target folder alone).

A relay and a tower in a throwaway folder, each on a port nothing else is on, with a stand-in agent that answers
what it is given; a simulator made for this run alone and deleted at the end, so no pairing of his is ever on it.
The checks themselves are an XCTest in the app (apps/apple/EndToEnd), run with `xcodebuild test`: it pairs from a
fresh link, sees the computer's name and picture, types to the agent and gets its answer in the chat, sees two agents
started meanwhile in a new team (one of them renamed) in its list at once, and with the relay gone has what it typed
wait and go once the relay is back. The test asks this script for what only the
computer's side can do (a new pairing link, the relay stopped and started, what the agent was given) over a small
local HTTP server whose address it is handed in its environment.

Notifications: the relay is given a signing key made for this run (never a real one) and a stand-in for the
push service on this machine. What the relay hands that stand-in, headers and payload exactly as they would go
to the push service, is delivered to the simulator with `xcrun simctl push`, and the test opens the same payload
with the extension's own code (the simulator runs no extension for a push it is handed). It checks what was
shown for a report, a question, a request for leave, a request for leave too long to allow unread, and one no
pairing on the phone can open.
"""
import http.server, json, os, shutil, socket, subprocess, sys, tempfile, threading, time, urllib.parse, wave
import wire
from listening_stand_ins import Listening
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import ec

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = os.path.abspath(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")))
BIN, RELAY = os.path.join(TARGET, "release", "airband"), os.path.join(TARGET, "release", "airband-relay")
RELAY_PORT, TOWER_PORT, CONTROL_PORT, APNS_PORT = wire.free_port(), wire.free_port(), wire.free_port(), wire.free_port()
APP = "dev.airband.app"
# this computer's name as the phone is to show it: with a space, so how the link carries it is tried too
COMPUTER = "Sim Desk"
# the simulator this run makes for itself, of this kind (any iPhone the newest iOS runtime here supports)
SIM_TYPE = os.environ.get("AIRBAND_E2E_SIMULATOR", "com.apple.CoreSimulator.SimDeviceType.iPhone-17")
DERIVED = os.environ.get("AIRBAND_E2E_DERIVED", os.path.join(TARGET, "phone-e2e-derived"))
# What the simulator keeps (gigabytes) lives in a disk image next to the build, not on the Mac's own disk. An APFS
# image, so it works on any drive: Core ML will not load a model from exFAT.
SIMS_IMAGE = os.environ.get("AIRBAND_E2E_SIMS", os.path.join(TARGET, "phone-e2e-sims.sparsebundle"))

work = tempfile.mkdtemp(prefix="airband-phone-e2e-")
home, data, project, heard = (os.path.join(work, d) for d in ("tower", "relay", "shop", "heard.jsonl"))
for d in (os.path.join(home, ".airband"), data, project):
    os.makedirs(d)
# a request asked early waits through the whole test: one that gave up would send a notification of its own
json.dump({"port": TOWER_PORT, "token": "own", "radio_token": "radio", "name": "Charles", "station": COMPUTER, "relay": f"http://127.0.0.1:{RELAY_PORT}",
           "summarizer": {"kind": "none"}, "lan": False, "permission_wait": 600}, open(os.path.join(home, ".airband", "config.json"), "w"))
# through the relay only, as in tools/e2e-relay.py: the phone is given no address of the tower's own
tower_env = dict(os.environ, HOME=home, AIRBAND_STAND_IN_NO_PRIVATE_NETWORK="1")
# a signing key made for this run alone, and the stand-in push service on this machine
push_key = ec.generate_private_key(ec.SECP256R1()).private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()
relay_env = dict(os.environ, PORT=str(RELAY_PORT), HOST="127.0.0.1", AIRBAND_RELAY_DATA=data, APNS_KEY=push_key, APNS_KEY_ID="E2EKEY0001", APNS_TEAM_ID="E2ETEAM001",
                 APNS_TOPIC=APP, APNS_HOST=f"http://127.0.0.1:{APNS_PORT}")
tower_log, relay_log = os.path.join(work, "tower.log"), os.path.join(work, "relay.log")

# The stand-in agent, as a delivery command: what it is given is kept in a file, and it answers "Done: " and the
# last line of it, reporting to the tower like any agent.
AGENT = os.path.join(work, "agent.py")
open(AGENT, "w").write(r'''
import json, subprocess, sys
BIN, ID, CWD, HEARD = sys.argv[1:5]
words = sys.stdin.read()
# what he said is the last line (the first says who it is from, the same each time), so each answer is its own
said = "Done: " + words.strip().splitlines()[-1][:60]
with open(HEARD, "a") as f: f.write(json.dumps({"heard": words, "said": said}) + "\n")
# on its own: the tower is waiting for this program to end before it counts the message delivered
subprocess.Popen([BIN, "report", "--id", ID, "--harness", "script", "--cwd", CWD, "said", said], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
''')


def listening(port):
    with socket.socket() as s:
        return s.connect_ex(("127.0.0.1", port)) == 0


def until(fact, seconds):
    end = time.time() + seconds
    while not fact() and time.time() < end:
        time.sleep(0.1)
    return fact()


def start_relay():
    p = subprocess.Popen([RELAY], env=relay_env, stdout=open(relay_log, "a"), stderr=subprocess.STDOUT)
    if not until(lambda: listening(RELAY_PORT), 20):
        raise SystemExit(f"the relay did not start: {relay_log}")
    return p


def logged():
    return open(tower_log).read()


relay = start_relay()
tower = subprocess.Popen([BIN, "run"], env=tower_env, stdout=open(tower_log, "a"), stderr=subprocess.STDOUT)
if not until(lambda: listening(TOWER_PORT), 20):
    relay.terminate()
    tower.terminate()
    raise SystemExit(f"the tower did not start: {tower_log}")
subprocess.run([BIN, "report", "--id", "a1", "--harness", "script", "--cwd", project, "--deliver-command", json.dumps([sys.executable, AGENT, BIN, "a1", project, heard]), "checkin"], env=tower_env, check=True)
model = subprocess.run(["sysctl", "-n", "hw.model"], capture_output=True, text=True).stdout.strip()
# the shape the catalogue gives this computer's model, as the phone is to show it: "" when no row lists it
rows = [json.load(open(os.path.join(ROOT, "catalog", "machines", f))) for f in sorted(os.listdir(os.path.join(ROOT, "catalog", "machines")))]
shape = next((r["shape"] for r in rows if model in r["models"]), "")
lock, radios = threading.Lock(), []
# what the relay handed the push service, in order: its headers and payload, and what `simctl push` said
pushes, sim, sims = [], None, None


class Apns(http.server.BaseHTTPRequestHandler):
    """The push service, as the relay speaks to it: each notification is kept and handed to the simulator."""
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        path = os.path.join(work, f"push-{len(pushes)}.apns")
        # as Apple would hand it to the phone; the app is named on the command line
        open(path, "wb").write(body)
        push = {"to": self.path.rsplit("/", 1)[-1], "push_type": self.headers.get("apns-push-type"), "priority": self.headers.get("apns-priority"),
                "topic": self.headers.get("apns-topic"), "collapse": self.headers.get("apns-collapse-id"), "payload": json.loads(body), "delivered": False}
        pushes.append(push)
        # accepted at once, as Apple does; the simulator is handed it after
        self.send_response(200)
        self.send_header("content-length", "0")
        self.end_headers()
        self.wfile.flush()
        done = subprocess.run(["xcrun", "simctl", "push", sim, APP, path], capture_output=True, text=True, timeout=120) if sim else None
        push["delivered"] = bool(done and done.returncode == 0)
        if done and done.returncode != 0:
            print(f"simctl push: {done.stderr.strip()}", flush=True)


apns = http.server.ThreadingHTTPServer(("127.0.0.1", APNS_PORT), Apns)
threading.Thread(target=apns.serve_forever, daemon=True).start()
# what an agent is to ask, by kind: a report, a question, and requests for leave. A request for leave waits for
# his answer, so it is asked on a thread of its own.
LONG = "cd /tmp/shop && " + " && ".join(f"echo step {n} of the long build" for n in range(1, 21))
# Each request for leave is its own agent's: one agent's newer request would take the place of its earlier one.
# The brief one's tool stops waiting after 8 seconds, and settles it itself.
ASKS = {"report": ("a1", ["said", "The checker passes on all twelve units."]), "question": ("a1", ["asked", "Rewrite the two easy questions, or leave them?"]),
        "permission": ("p1", ["permission", "run `npm run deploy`"]), "long-permission": ("p2", ["permission", f"run `{LONG}`"]),
        "brief-permission": ("p3", ["--within", "8", "permission", "run `make release`"])}


class Control(http.server.BaseHTTPRequestHandler):
    """What the test asks of the computer's side."""
    def log_message(self, *args):
        pass

    def answer(self, body, status=200):
        raw = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        if self.path == "/pushes":
            return self.answer(pushes)
        if self.path == "/wakeable":
            return self.answer({"wakeable": "can be woken with a notification" in logged()})
        if self.path == "/heard":
            return self.answer([json.loads(l) for l in open(heard)] if os.path.exists(heard) else [])
        if self.path == "/listening":
            with services.lock:
                return self.answer(services.seen)
        self.answer({"wrong": self.path}, 404)

    def do_POST(self):
        global relay
        with lock:
            if self.path == "/pair":
                # a new pairing, served through the relay before its link is handed over: the tower picks it up
                # on its next look at its settings, and says so
                radio = f"sim-{len(radios) + 1}"
                radios.append(radio)
                made = subprocess.run([BIN, "pair", radio], env=tower_env, capture_output=True, text=True)
                links = [w for w in made.stdout.split() if w.startswith("airband:")]
                if not links:
                    return self.answer({"wrong": f"airband pair gave no link: {made.stdout} {made.stderr}"}, 500)
                link = links[-1]
                if not until(lambda: wire.serving(link) in logged(), 40):
                    return self.answer({"wrong": "the tower did not serve the new pairing"}, 500)
                carried = dict(urllib.parse.parse_qsl(link.split("?", 1)[1]))
                return self.answer({"link": link, "name": COMPUTER, "model": model, "shape": shape, "carried": {"name": carried.get("name", ""), "model": carried.get("model", "")}})
            if self.path.startswith("/agent?"):
                # an agent of his checks in, in a folder of this run's (a team of its own), is renamed by him, or
                # signs off: as a session he starts in a terminal, names, and ends reports itself
                q = dict(urllib.parse.parse_qsl(self.path.split("?", 1)[1]))
                folder = os.path.join(work, q["folder"])
                os.makedirs(folder, exist_ok=True)
                titled = ["--title", q["title"]] if q.get("title") else []
                event = "signoff" if q.get("gone") else "checkin"
                done = subprocess.run([BIN, "report", "--id", q["id"], "--harness", "script", "--cwd", folder, *titled, event], env=tower_env, capture_output=True, text=True)
                return self.answer({"ok": done.returncode == 0, "said": done.stderr.strip()}, 200 if done.returncode == 0 else 500)
            if self.path == "/relay/stop":
                relay.terminate()
                relay.wait(10)
                return self.answer({"listening": listening(RELAY_PORT)})
            if self.path == "/relay/start":
                relay = start_relay()
                return self.answer({"listening": listening(RELAY_PORT)})
            if self.path.startswith("/ask/") and self.path[5:] in ASKS:
                agent, words = ASKS[self.path[5:]]
                ask = lambda: subprocess.run([BIN, "report", "--id", agent, "--harness", "script", "--cwd", project, *words], env=tower_env, capture_output=True, timeout=120)
                threading.Thread(target=ask, daemon=True).start()
                return self.answer({"asked": self.path[5:]})
        self.answer({"wrong": self.path}, 404)


services = Listening("")
control = http.server.ThreadingHTTPServer(("127.0.0.1", CONTROL_PORT), Control)
threading.Thread(target=control.serve_forever, daemon=True).start()

code = 1
try:
    # the newest iOS this Xcode has, and a simulator of its own on it
    runtimes = [r for r in json.loads(subprocess.run(["xcrun", "simctl", "list", "runtimes", "-j"], capture_output=True, text=True, check=True).stdout)["runtimes"] if r["platform"] == "iOS" and r["isAvailable"]]
    runtime = max(runtimes, key=lambda r: tuple(int(p) for p in r["version"].split(".")))
    if not any(t["identifier"] == SIM_TYPE for t in runtime["supportedDeviceTypes"]):
        raise SystemExit(f"iOS {runtime['version']} has no {SIM_TYPE}: set AIRBAND_E2E_SIMULATOR to one it has")
    if not os.path.isdir(SIMS_IMAGE):
        subprocess.run(["hdiutil", "create", "-size", "60g", "-type", "SPARSEBUNDLE", "-fs", "APFS", "-volname", "airband-e2e-sims", SIMS_IMAGE], check=True, capture_output=True)
    sims = os.path.join(work, "sims")
    os.makedirs(sims)
    subprocess.run(["hdiutil", "attach", "-nobrowse", "-mountpoint", sims, SIMS_IMAGE], check=True, capture_output=True)
    sim = subprocess.run(["xcrun", "simctl", "create", "airband-e2e-" + os.urandom(4).hex(), SIM_TYPE, runtime["identifier"]], capture_output=True, text=True, check=True).stdout.strip()
    # its data (what fills up) in the image; xcodebuild reaches only simulators in the usual set, so the device
    # stays listed there, and its data folder points into the image
    data = os.path.expanduser(f"~/Library/Developer/CoreSimulator/Devices/{sim}/data")
    kept = os.path.join(sims, sim)
    shutil.move(data, kept)
    os.symlink(kept, data)
    # no Siri and no dictation: nothing here uses Apple's speech recognition, and left on, a new simulator
    # fetches gigabytes of its assets
    subprocess.run(["xcrun", "simctl", "boot", sim], check=True, capture_output=True)
    for key in ("Assistant Enabled", "Dictation Enabled"):
        subprocess.run(["xcrun", "simctl", "spawn", sim, "defaults", "write", "com.apple.assistant.support", key, "-bool", "false"], capture_output=True)
    subprocess.run(["xcrun", "simctl", "spawn", sim, "defaults", "write", "com.apple.keyboard.preferences", "KeyboardDictationEnabled", "-bool", "false"], capture_output=True)
    # the Rust core for the simulator, the Swift it is reached by, and the Xcode project
    subprocess.run([os.path.join(ROOT, "scripts", "build-apple.sh")], env=dict(os.environ, CARGO_TARGET_DIR=TARGET, AIRBAND_STAND_INS="1"), check=True, stdout=open(os.path.join(work, "build.log"), "w"), stderr=subprocess.STDOUT)
    # what he says to the listening checks (ListeningEndToEndTests): this Mac's own voice saying it, as the
    # microphone gives it to the core (16 kHz, 16-bit, mono, no header)
    words = "please ship the new build to the store today"
    aiff, wav, speech = (os.path.join(work, f) for f in ("speech.aiff", "speech.wav", "speech.pcm"))
    subprocess.run(["say", "-o", aiff, words], check=True)
    subprocess.run(["afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", aiff, wav], check=True)
    with wave.open(wav) as w:
        open(speech, "wb").write(w.readframes(w.getnframes()))
    # every listening service in the catalogue, stood in for on this machine, each saying those words
    services.words = words.split()
    test_env = dict(os.environ, TEST_RUNNER_AIRBAND_E2E_CONTROL=f"http://127.0.0.1:{CONTROL_PORT}", TEST_RUNNER_AIRBAND_E2E_SPEECH=speech, TEST_RUNNER_AIRBAND_E2E_SPEECH_WORDS=words,
                    TEST_RUNNER_AIRBAND_STAND_IN_HOSTS=services.hosts())
    print(f"The phone's core in a simulator of its own (iOS {runtime['version']}), against this run's tower and relay", flush=True)
    r = subprocess.run(["xcodebuild", "test", "-project", os.path.join(ROOT, "apps", "apple", "Airband.xcodeproj"), "-scheme", "AirbandEndToEnd", "-destination", f"id={sim}",
                        "-derivedDataPath", DERIVED, "-collect-test-diagnostics", "never", f"AIRBAND_RUST_TARGET={TARGET}"], env=test_env, capture_output=True, text=True)
    open(os.path.join(work, "xcodebuild.log"), "w").write(r.stdout + r.stderr)
    # what XCTest says of each check, and its failures with where they are
    for line in (r.stdout + r.stderr).splitlines():
        if line.startswith("Test Case") and (" passed " in line or " failed " in line) or ": error: " in line:
            print("  " + line, flush=True)
    code = r.returncode
finally:
    control.shutdown()
    apns.shutdown()
    services.shutdown()
    for p in (tower, relay):
        p.terminate()
        try:
            p.wait(10)
        except subprocess.TimeoutExpired:
            p.kill()
    if sim:
        # what the app logged of its own (codes and ids only), for a failure
        shown = subprocess.run(["xcrun", "simctl", "spawn", sim, "log", "show", "--last", "1h", "--style", "compact", "--predicate", 'eventMessage CONTAINS "airband:"'], capture_output=True, text=True)
        open(os.path.join(work, "app.log"), "w").write(shown.stdout + shown.stderr)
        subprocess.run(["xcrun", "simctl", "shutdown", sim], capture_output=True)
        subprocess.run(["xcrun", "simctl", "delete", sim], capture_output=True)
    if sims:
        if sim:
            shutil.rmtree(os.path.join(sims, sim), ignore_errors=True)
        subprocess.run(["hdiutil", "detach", sims], capture_output=True)
print("\nThe phone's checks passed." if code == 0 else f"\nThe phone's checks failed. Logs: {work}")
if code == 0:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(code)
