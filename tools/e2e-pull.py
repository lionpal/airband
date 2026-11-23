#!/usr/bin/env python3
"""The built tower against the built relay, for the pull itself: which tower is pulling, what it got, and which kept
message it is still answering for (airband_core::relay::Pulling on the tower, the relay's pull and open). Each
half has tests against a stand-in for the other; this is the two real programs together, on this machine only.

    cargo build --release -p airband-cli -p airband-relay && tools/e2e-pull.py      about a minute, no model calls

A relay on a throwaway folder, one tower, and then a second tower with the first one's settings copied (as when he
copies his settings to another computer): the second is told another computer serves the pairing, and the first
goes on serving it. Then the first is killed and started again, under a new id, while its old pull may still be
held open: it serves again with no word of another computer. Last, what he says while it is away is kept by the
relay and reaches the agent once, when it is back. Never a live relay.
"""
import json, os, shutil, signal, subprocess, sys, tempfile, time
import wire

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BUILT = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release")
BIN, RELAY = os.path.join(BUILT, "airband"), os.path.join(BUILT, "airband-relay")
RELAY_PORT, FIRST_PORT, SECOND_PORT = wire.free_port(), wire.free_port(), wire.free_port()
BASE = f"http://127.0.0.1:{RELAY_PORT}"
work = tempfile.mkdtemp(prefix="airband-pull-e2e-")
first_home, second_home, phone, data, project, got = (os.path.join(work, d) for d in ("first", "second", "phone", "relay", "shop", "got.txt"))
for d in (os.path.join(first_home, ".airband"), os.path.join(phone, ".airband"), data, project):
    os.makedirs(d)
config = {"port": FIRST_PORT, "token": "own", "radio_token": "radio", "name": "Charles", "station": "e2e", "relay": BASE, "summarizer": {"kind": "none"}, "lan": False}
json.dump(config, open(os.path.join(first_home, ".airband", "config.json"), "w"))
env_of = lambda home: dict(os.environ, HOME=home, AIRBAND_STAND_IN_NO_PRIVATE_NETWORK="1")
phone_env = dict(os.environ, HOME=phone)
results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def log_of(name):
    path = os.path.join(work, name)
    return open(path).read() if os.path.exists(path) else ""
def until(what, seconds=40):
    end = time.time() + seconds
    while time.time() < end:
        if what(): return True
        time.sleep(0.2)
    return bool(what())
def start(home, log):
    return subprocess.Popen([BIN, "run"], env=env_of(home), stdout=open(os.path.join(work, log), "a"), stderr=subprocess.STDOUT)
def send(text, timeout=120):
    return subprocess.run([BIN, "send", text, "--keep-trying", "--link", link], env=phone_env, capture_output=True, text=True, timeout=timeout).stdout.strip()
def received():
    return open(got).read() if os.path.exists(got) else ""
SERVING = 'serving "test" through'
ANOTHER = "another computer is serving this pairing"

relay = subprocess.Popen([RELAY], env=dict(os.environ, PORT=str(RELAY_PORT), HOST="127.0.0.1", AIRBAND_RELAY_DATA=data), stdout=open(os.path.join(work, "relay.log"), "w"), stderr=subprocess.STDOUT)
towers = []
try:
    # the relay is up once it answers its health check
    import urllib.request, urllib.error
    def healthy():
        try:
            with urllib.request.urlopen(f"{BASE}/healthz", timeout=2) as r: return r.status == 200
        except (urllib.error.URLError, OSError): return False
    if not until(healthy, 20): sys.exit("The relay did not start. Its log: " + os.path.join(work, "relay.log"))
    first = start(first_home, "first.log"); towers.append(first)
    link = [w for w in subprocess.run([BIN, "pair", "test"], env=env_of(first_home), capture_output=True, text=True).stdout.split() if w.startswith("airband:")][-1]
    check("the first tower serves the new pairing through the relay", until(lambda: SERVING in log_of("first.log")), log_of("first.log")[-400:])
    subprocess.run([BIN, "report", "--id", "a1", "--harness", "script", "--cwd", project, "--deliver-command", json.dumps(["sh", "-c", f"cat>>{got}"]), "checkin"], env=env_of(first_home))
    check("what he says reaches the agent through it", "Delivered." in send("one") and "one" in received(), received())

    print("The same settings on a second computer")
    shutil.copytree(os.path.join(first_home, ".airband"), os.path.join(second_home, ".airband"), ignore=shutil.ignore_patterns("*.db*", "*.log", "*.sock", "*.lock"))
    copied = json.load(open(os.path.join(second_home, ".airband", "config.json")))
    copied["port"] = SECOND_PORT
    json.dump(copied, open(os.path.join(second_home, ".airband", "config.json"), "w"))
    second = start(second_home, "second.log"); towers.append(second)
    check("the second is told another computer is serving this pairing", until(lambda: ANOTHER in log_of("second.log")), log_of("second.log")[-600:])
    check("and the first is never told so: it is the one serving", ANOTHER not in log_of("first.log"), log_of("first.log")[-600:])
    before = received().count("two")
    check("what he says still reaches the agent, once, through the first", "Delivered." in send("two") and received().count("two") == before + 1, received())
    second.terminate(); second.wait(10)

    print("The first tower is killed and starts again, under a new id")
    first.send_signal(signal.SIGKILL); first.wait(10)
    seen = len(log_of("first.log"))
    first = start(first_home, "first.log"); towers.append(first)
    check("it serves the pairing again", until(lambda: SERVING in log_of("first.log")[seen:]), log_of("first.log")[seen:][-400:])
    check("what he says reaches the agent", "Delivered." in send("three") and "three" in received(), received())
    check("with no word of another computer, though its old pull may still have been held open", ANOTHER not in log_of("first.log")[seen:], log_of("first.log")[seen:][-600:])

    print("Away, then back")
    first.terminate(); first.wait(10)
    away = subprocess.Popen([BIN, "send", "said while away", "--keep-trying", "--link", link], env=phone_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    # the relay keeps it (the phone first waits out the relay's call, then leaves it there) before the tower is back
    check("the relay keeps what he says while the computer is away", until(lambda: os.path.exists(os.path.join(data, "held")) and any(files for _, _, files in os.walk(os.path.join(data, "held"))), 150), os.listdir(data))
    first = start(first_home, "first.log"); towers.append(first)
    away.wait(150)
    check("once it is back, the kept words reach the agent once", until(lambda: received().count("said while away") >= 1, 60) and received().count("said while away") == 1, received())
    check("and the relay lets go of what it kept", until(lambda: not any(f.isdigit() for _, _, fs in os.walk(os.path.join(data, "held")) for f in fs), 30))
    # the relay writes no more than its own facts: never the channel's whole name
    channel_names = [w for w in log_of("relay.log").split() if len(w) == 32 and all(c in "0123456789abcdef" for c in w)]
    check("the relay's log names no channel in full", not channel_names, channel_names[:3])
finally:
    for t in towers:
        if t.poll() is None:
            t.terminate()
            try: t.wait(10)
            except subprocess.TimeoutExpired: t.kill()
    relay.terminate(); relay.wait(10)

failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)}/{len(results)} passed" + (f"; failed: {failed}" if failed else ""))
if failed:
    print("Kept for a look:", work)
    sys.exit(1)
shutil.rmtree(work, ignore_errors=True)
