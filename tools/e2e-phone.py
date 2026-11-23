#!/usr/bin/env python3
"""End to end, for the phone: the app's own radio core (its Desk), in the simulator, against a tower and a relay
on this machine.

    cargo build --release -p airband-cli -p airband-relay --features airband-cli/stand-ins && tools/e2e-phone.py
                                                       a few minutes the first time (the app is built), no model calls

A relay and a tower in a throwaway folder, each on a port nothing else is on, with a stand-in agent that answers
what it is given; a simulator made for this run alone and deleted at the end, so no pairing of his is ever on it.
The checks themselves are an XCTest in the app (apps/apple/EndToEnd), run with `xcodebuild test`: it pairs from a
fresh link, sees the computer's name and picture, types to the agent and gets its answer in the chat, and with the
relay gone has what it typed wait and go once the relay is back. The test asks this script for what only the
computer's side can do (a new pairing link, the relay stopped and started, what the agent was given) over a small
local HTTP server whose address it is handed in its environment.
"""
import http.server, json, os, shutil, socket, subprocess, sys, tempfile, threading, time, urllib.parse
import wire

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = os.path.abspath(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")))
BIN, RELAY = os.path.join(TARGET, "release", "airband"), os.path.join(TARGET, "release", "airband-relay")
RELAY_PORT, TOWER_PORT, CONTROL_PORT = wire.free_port(), wire.free_port(), wire.free_port()
# this computer's name as the phone is to show it: with a space, so how the link carries it is tried too
COMPUTER = "Sim Desk"
# the simulator this run makes for itself, of this kind (any iPhone the newest iOS runtime here supports)
SIM_TYPE = os.environ.get("AIRBAND_E2E_SIMULATOR", "com.apple.CoreSimulator.SimDeviceType.iPhone-17")
DERIVED = os.environ.get("AIRBAND_E2E_DERIVED", os.path.join(TARGET, "phone-e2e-derived"))

work = tempfile.mkdtemp(prefix="airband-phone-e2e-")
home, data, project, heard = (os.path.join(work, d) for d in ("tower", "relay", "shop", "heard.jsonl"))
for d in (os.path.join(home, ".airband"), data, project):
    os.makedirs(d)
json.dump({"port": TOWER_PORT, "token": "own", "radio_token": "radio", "name": "Charles", "station": COMPUTER, "relay": f"http://127.0.0.1:{RELAY_PORT}",
           "summarizer": {"kind": "none"}, "lan": False}, open(os.path.join(home, ".airband", "config.json"), "w"))
# through the relay only, as in tools/e2e-relay.py: the phone is given no address of the tower's own
tower_env = dict(os.environ, HOME=home, AIRBAND_STAND_IN_NO_PRIVATE_NETWORK="1")
relay_env = dict(os.environ, PORT=str(RELAY_PORT), HOST="127.0.0.1", AIRBAND_RELAY_DATA=data)
tower_log, relay_log = os.path.join(work, "tower.log"), os.path.join(work, "relay.log")

# The stand-in agent, as a delivery command: what it is given is kept in a file, and it answers "Done: " and the
# first line of it (as the stand-in of tools/e2e-held.py does), reporting to the tower like any agent.
AGENT = os.path.join(work, "agent.py")
open(AGENT, "w").write(r'''
import json, subprocess, sys
BIN, ID, CWD, HEARD = sys.argv[1:5]
words = sys.stdin.read()
said = "Done: " + words.strip().splitlines()[0][:60]
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
        if self.path == "/heard":
            return self.answer([json.loads(l) for l in open(heard)] if os.path.exists(heard) else [])
        self.answer({"wrong": self.path}, 404)

    def do_POST(self):
        global relay
        with lock:
            if self.path == "/pair":
                # a new pairing, served through the relay before its link is handed over: the tower picks it up
                # on its next look at its settings, and says so
                radio = f"sim-{len(radios) + 1}"
                radios.append(radio)
                out = subprocess.run([BIN, "pair", radio], env=tower_env, capture_output=True, text=True).stdout.split()
                link = [w for w in out if w.startswith("airband:")][-1]
                if not until(lambda: wire.serving(link) in logged(), 40):
                    return self.answer({"wrong": "the tower did not serve the new pairing"}, 500)
                carried = dict(urllib.parse.parse_qsl(link.split("?", 1)[1]))
                return self.answer({"link": link, "name": COMPUTER, "model": model, "shape": shape, "carried": {"name": carried.get("name", ""), "model": carried.get("model", "")}})
            if self.path == "/relay/stop":
                relay.terminate()
                relay.wait(10)
                return self.answer({"listening": listening(RELAY_PORT)})
            if self.path == "/relay/start":
                relay = start_relay()
                return self.answer({"listening": listening(RELAY_PORT)})
        self.answer({"wrong": self.path}, 404)


control = http.server.ThreadingHTTPServer(("127.0.0.1", CONTROL_PORT), Control)
threading.Thread(target=control.serve_forever, daemon=True).start()

sim = None
code = 1
try:
    # the newest iOS this Xcode has, and a simulator of its own on it
    runtimes = [r for r in json.loads(subprocess.run(["xcrun", "simctl", "list", "runtimes", "-j"], capture_output=True, text=True, check=True).stdout)["runtimes"] if r["platform"] == "iOS" and r["isAvailable"]]
    runtime = max(runtimes, key=lambda r: tuple(int(p) for p in r["version"].split(".")))
    if not any(t["identifier"] == SIM_TYPE for t in runtime["supportedDeviceTypes"]):
        raise SystemExit(f"iOS {runtime['version']} has no {SIM_TYPE}: set AIRBAND_E2E_SIMULATOR to one it has")
    sim = subprocess.run(["xcrun", "simctl", "create", "airband-e2e-" + os.urandom(4).hex(), SIM_TYPE, runtime["identifier"]], capture_output=True, text=True, check=True).stdout.strip()
    # the Rust core for the simulator, the Swift it is reached by, and the Xcode project
    subprocess.run([os.path.join(ROOT, "scripts", "build-apple.sh")], env=dict(os.environ, CARGO_TARGET_DIR=TARGET), check=True, stdout=open(os.path.join(work, "build.log"), "w"), stderr=subprocess.STDOUT)
    test_env = dict(os.environ, TEST_RUNNER_AIRBAND_E2E_CONTROL=f"http://127.0.0.1:{CONTROL_PORT}")
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
    for p in (tower, relay):
        p.terminate()
        try:
            p.wait(10)
        except subprocess.TimeoutExpired:
            p.kill()
    if sim:
        subprocess.run(["xcrun", "simctl", "shutdown", sim], capture_output=True)
        subprocess.run(["xcrun", "simctl", "delete", sim], capture_output=True)
print("\nThe phone's checks passed." if code == 0 else f"\nThe phone's checks failed. Logs: {work}")
if code == 0:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(code)
