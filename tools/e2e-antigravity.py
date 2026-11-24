#!/usr/bin/env python3
"""End to end, with a real Antigravity CLI (`agy`) and real model calls: a real tower in a throwaway home.

    GEMINI_API_KEY=… tools/e2e-antigravity.py

It needs `agy` on the PATH and a Gemini API key in the environment, which agy is told to use
(`"modelProvider": "gemini"` in its settings, in the throwaway home), and which the tower's summaries are written
with (Gemini Flash-Lite, as tools/e2e.py --gemini has it: in the throwaway tower's settings, private to this
user, and taken out of them when the run ends, kept folder or not). It is never printed or put on a command line.
Nobody is signed in to anything.

- An agent the tower starts and holds (`agy acp`), on what agy itself starts on: it says what it runs on, its request to run a
  command is put to him as the command itself, his yes lets it run and his no stops it, what it says arrives, and
  a long report arrives summarized with the whole of it kept.
- A session he opened himself in a terminal (here a tmux of the test's own): it reports through airband's entry in
  Antigravity's hooks file, is on the radio while its lock in `presence/` is held, says its last answer when its
  turn is over, and leaves the radio when it closes.
"""
import json, os, shutil, subprocess, sys, tempfile, time, urllib.request, urllib.error
import wire

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "airband")
if not os.environ.get("GEMINI_API_KEY"):
    sys.exit("This needs a Gemini API key in GEMINI_API_KEY, for agy to use.")
if not shutil.which("agy"):
    sys.exit("This needs Antigravity CLI (agy) on the PATH.")
PORT, OWN, RADIO = wire.free_port(), "agy-own", "agy-radio"
work = tempfile.mkdtemp(prefix="airband-agy-")
home, shop, barn = os.path.join(work, "home"), os.path.join(work, "code", "shop"), os.path.join(work, "code", "barn")
for d in (os.path.join(home, ".airband"), shop, barn, os.path.join(home, ".gemini", "antigravity-cli", "cache"), os.path.join(home, ".gemini", "config")):
    os.makedirs(d)
shop, barn = os.path.realpath(shop), os.path.realpath(barn)
# agy's own settings in the throwaway home: the key in the environment is what it runs on, the two folders are
# trusted, and its first-run questions have been answered, so a terminal session opens straight to its prompt
state = os.path.join(home, ".gemini", "antigravity-cli")
json.dump({"modelProvider": "gemini", "trustedWorkspaces": [shop, barn]}, open(os.path.join(state, "settings.json"), "w"))
json.dump({"consumerOnboardingComplete": True, "enterpriseOnboardingComplete": True, "onboardingComplete": True}, open(os.path.join(state, "cache", "onboarding.json"), "w"))
# airband's entry in its hooks file, as `airband install` writes it (adapters/hooks.rs, Shape::Named)
command = f'"{BIN}" hook antigravity-cli --put-there-by-airband'
ours = lambda: {"type": "command", "command": command, "timeout": 5}
json.dump({"airband": {"SessionStart": [ours()], "PreInvocation": [ours()], "Stop": [ours()]}}, open(os.path.join(home, ".gemini", "config", "hooks.json"), "w"))
CONFIG = os.path.join(home, ".airband", "config.json")
with open(os.open(CONFIG, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
    json.dump({"port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "agy",
               "summarizer": {"kind": "gemini", "offer": "flash-lite", "key": os.environ["GEMINI_API_KEY"]},
               "projects": {"shop": shop, "barn": barn}, "launcher": "antigravity-cli"}, f)
env = dict(os.environ, HOME=home)
env.pop("AIRBAND_INTERNAL", None)
TMUX = ["tmux", "-L", "airband-e2e-" + os.urandom(4).hex(), "-f", "/dev/null"]

results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def http(method, path, body=None, token=RADIO):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json", wire.SCHEMA_HEADER: wire.SCHEMA, "x-airband-radio": "agy", "x-airband-presence": '{"on_call":false,"showing_chat":true}'})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        raw = e.read()
        try: return e.code, (json.loads(raw) if raw else None)
        except ValueError: return e.code, None
    except urllib.error.URLError:
        return 0, None
def say(text, **more):
    body = {"text": text, "id": os.urandom(6).hex(), **more}
    for _ in range(1800):
        took = http("POST", "/v1/radio/utterance", body)[1]
        if took["state"] == "answered": return took["reply"]["say"]
        time.sleep(0.1)
def named(f):
    for a in f.get("roster", []):
        if isinstance(a["callsign"], dict):
            parts = a["callsign"]
            a["callsign"], a["group"] = f"{parts['project']} {parts['number']}", parts["project"]
    return f
feed = lambda: named(http("GET", "/v1/radio/feed?since=0")[1])
def until(what, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        v = what()
        if v: return v
        time.sleep(0.3)
    return what()
def start_tower():
    t = subprocess.Popen([BIN, "run"], env=env, stdout=open(os.path.join(work, "tower.log"), "a"), stderr=subprocess.STDOUT)
    for _ in range(300):
        if http("GET", "/v1/status", token=OWN)[0] == 200: return t
        time.sleep(0.1)
    sys.exit("The tower did not start. Its log: " + os.path.join(work, "tower.log"))

agy = lambda held: next((a for a in feed()["roster"] if a.get("harness") == "antigravity-cli" and bool(a.get("held")) == held), None)
said_by = lambda a: [m for m in feed()["messages"] if a and m.get("agent") == a["id"] and m.get("from") != "you"]
cards = lambda a: [m for m in feed()["messages"] if a and m.get("permission") and m.get("agent") == a["id"]]

tower = start_tower()
tmux_up = False
try:
    print("An agent the tower runs (agy acp), on what agy itself starts on")
    told = say("New agent in shop", act={"kind": "launch", "team": shop, "tool": "antigravity-cli",
                                         "task": "Use your shell tool to run exactly this command, and nothing else: echo airband-probe > made.txt\nThen reply with exactly the word done."})
    # nothing of agy has been seen on this tower yet: nothing is offered for it until an agent of its has said
    check("before any agy agent has said what it can be set to, it is started on its own settings", told.startswith("Launching in shop") and " with " not in told.split(".")[0], told)
    held = until(lambda: agy(True), 60)
    check("it is on the radio, held, and says the model it runs on", held and until(lambda: bool((agy(True) or {}).get("model")), 60), agy(True))
    card = until(lambda: cards(agy(True)), 120)
    check("its request to run a command is put to him as the command itself", card and card[0]["text"] == "Wants to run `echo airband-probe > made.txt`. Allow?" and card[0]["options"] == ["Allow", "Deny"], card)
    check("nothing has run while it waits for him", not os.path.exists(os.path.join(shop, "made.txt")))
    if card:
        say("Allow", re=card[0]["id"], act={"kind": "verdict", "allow": True})
    check("his yes lets it run", until(lambda: os.path.exists(os.path.join(shop, "made.txt")), 60))
    check("and what it says when it is done arrives", until(lambda: [m for m in said_by(agy(True)) if m["text"].strip().lower().rstrip(".") == "done"], 120), [m["text"] for m in said_by(agy(True))])
    until(lambda: agy(True) and agy(True)["state"] != "working", 30)
    call = "@" + held["callsign"].replace(" ", "") + " "
    say(call + "Use your shell tool to run exactly this command, and nothing else: touch denied.txt\nThen reply with exactly the word over.")
    asked = lambda: [c for c in cards(agy(True)) if "denied.txt" in c["text"]]
    card = until(asked, 120)
    check("his next words are its next turn, and its next request is put to him", card and card[0]["text"] == "Wants to run `touch denied.txt`. Allow?", cards(agy(True)))
    if card:
        say("Deny", re=card[0]["id"], act={"kind": "verdict", "allow": False})
    check("his no stops it: nothing ran, and its turn ends", until(lambda: agy(True) and agy(True)["state"] not in ("working", "asking"), 120) and not os.path.exists(os.path.join(shop, "denied.txt")), agy(True))
    until(lambda: agy(True) and agy(True)["state"] not in ("working", "asking"), 30)
    say(call + "With no tools, write a status report of at least 180 words, in four paragraphs, on the state of a made-up online shop's checkout: what was fixed, what was tested, what is left, and end by asking whether to deploy it today.")
    long = until(lambda: [m for m in said_by(agy(True)) if m.get("long")], 180)
    whole = long and (http("GET", "/v1/radio/message/" + long[-1]["id"])[1] or {})
    check("a long report it writes arrives summarized, shorter, with the whole of it kept (model)", long and whole and len(long[-1]["text"]) < len(whole.get("text", "")), [(m["text"][:80], bool(m.get("long"))) for m in said_by(agy(True))])
    check("it can be ended, and it leaves the radio", http("POST", "/v1/radio/end", {"agent": held["id"]})[1] == {} and until(lambda: agy(True) is None, 30), feed()["roster"])

    print("A session he opened himself, in a terminal")
    subprocess.run(TMUX + ["new-session", "-d", "-s", "barn", "-x", "200", "-y", "50", "-c", barn, "agy", "--model", "gemini-4-neon", "--effort", "low"], env=env, check=True)
    tmux_up = True
    time.sleep(6)
    subprocess.run(TMUX + ["send-keys", "-t", "barn", "-l", "Reply with exactly the word finished, and use no tools."], env=env, check=True)
    time.sleep(1)
    subprocess.run(TMUX + ["send-keys", "-t", "barn", "Enter"], env=env, check=True)
    mine = until(lambda: agy(False), 90)
    check("it reports in by its hooks and is on the radio in its folder's team, not held", mine and mine["group"] == "barn", feed()["roster"])
    check("its last answer arrives when its turn is over", until(lambda: [m for m in said_by(agy(False)) if m["text"].strip().lower().rstrip(".") == "finished"], 120), [m["text"] for m in said_by(agy(False))])
    check("it says the model it runs on, as its hooks name it", (agy(False) or {}).get("model") == "gemini-4-neon-low", agy(False))
    check("and it is waiting for him, not working", until(lambda: agy(False) and agy(False)["state"] not in ("working", "asking"), 30), agy(False))
    subprocess.run(TMUX + ["kill-server"], env=env)
    tmux_up = False
    check("when it closes it leaves the radio, though no hook said so", until(lambda: agy(False) is None, 30), feed()["roster"])
finally:
    if tmux_up:
        subprocess.run(TMUX + ["kill-server"], env=env)
    # a held agent a failed run left behind is ended the way he would end it, so its program goes with it
    for a in (feed() or {}).get("roster", []):
        if a.get("held"):
            http("POST", "/v1/radio/end", {"agent": a["id"]})
    tower.terminate()
    try: tower.wait(timeout=10)
    except subprocess.TimeoutExpired: tower.kill()
    # the key leaves the throwaway settings whatever happens to the folder
    kept = json.load(open(CONFIG))
    kept["summarizer"] = {"kind": "none"}
    json.dump(kept, open(CONFIG, "w"))
failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)} of {len(results)} passed." + ("" if not failed else "  Failed:\n  - " + "\n  - ".join(failed) + f"\nThe tower's log: {os.path.join(work, 'tower.log')}"))
if not failed and "--keep" not in sys.argv:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(1 if failed else 0)
