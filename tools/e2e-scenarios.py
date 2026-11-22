#!/usr/bin/env python3
"""End to end, for the paths the audits keep finding faults in: the tower kept across a restart, files of its
own that are damaged, a report handed over twice, and his reply to something said a while back.

    cargo build --release && tools/e2e-scenarios.py      about a minute, no model calls

A real tower in a throwaway home and stand-in agents that report with `airband report`, as in tools/e2e-held.py.
Each check names the audit-5 finding it covers (N24, A-5, ...), so a failure says which one is back. What the
phone keeps across a reconnect or a relaunch, and two computers at once, are tried in the phone core's own tests
(crates/airband-radio/src/desk/tests/scenarios.rs), where the desk can be driven without a screen.
"""
import json, os, shutil, signal, subprocess, sys, tempfile, time, urllib.request, urllib.error

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "airband")
PORT, OWN, RADIO = 7398, "scenarios-own", "scenarios-radio"
work = tempfile.mkdtemp(prefix="airband-scenarios-")
home, got = os.path.join(work, "home"), os.path.join(work, "got")
shop, docs = os.path.join(work, "code", "shop"), os.path.join(work, "code", "docs")
for d in (os.path.join(home, ".airband"), got, shop, docs):
    os.makedirs(d)
CONFIG = {"port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "scenarios", "summarizer": {"kind": "none"}, "lan": False}
json.dump(CONFIG, open(os.path.join(home, ".airband", "config.json"), "w"))
env = dict(os.environ, HOME=home)
LOG = os.path.join(work, "tower.log")

results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def http(method, path, body=None, token=RADIO):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json", "x-airband-radio": "scenarios", "x-airband-presence": '{"on_call":false,"showing_chat":true}'})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        return e.code, None
    except urllib.error.URLError:
        return 0, None
def up():
    """The tower, started and answering."""
    t = subprocess.Popen([BIN, "run"], env=env, stdout=open(LOG, "a"), stderr=subprocess.STDOUT)
    for _ in range(150):
        if http("GET", "/v1/status", token=OWN)[0] == 200: return t
        if t.poll() is not None: break
        time.sleep(0.1)
    return t
def down(t):
    t.terminate()
    try: t.wait(10)
    except subprocess.TimeoutExpired: t.kill(); t.wait()
def report(agent, cwd, *words, timeout=30):
    """An agent saying something, as its tool's hook says it. What it is sent arrives in got/<agent>."""
    deliver = json.dumps(["sh", "-c", f"cat >> {os.path.join(got, agent)}; echo >> {os.path.join(got, agent)}"])
    return subprocess.run([BIN, "report", "--id", agent, "--harness", "script", "--cwd", cwd, "--deliver-command", deliver, *words], env=env, capture_output=True, text=True, timeout=timeout)
def feed():
    return http("GET", "/v1/radio/feed?since=0")[1] or {}
def messages():
    return feed().get("messages", [])
def roster():
    return feed().get("roster", [])
def received(agent):
    path = os.path.join(got, agent)
    return open(path).read() if os.path.exists(path) else ""
def until(what, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        v = what()
        if v: return v
        time.sleep(0.2)
    return what()
def say(text, **more):
    """He says something; wait for what came of it."""
    body = {"text": text, "id": os.urandom(6).hex(), **more}
    for _ in range(300):
        took = http("POST", "/v1/radio/utterance", body)[1] or {}
        if took.get("state") == "answered": return took["reply"].get("say", "")
        time.sleep(0.1)
    return None
def said_by(agent, words):
    return next((m for m in messages() if words in m.get("text", "") and m.get("agent") == agent), None)

tower = up()
try:
    check("the tower starts in a throwaway home", http("GET", "/v1/status", token=OWN)[0] == 200, open(LOG).read()[-400:])
    report("a1", shop, "checkin"); report("b1", docs, "checkin")
    until(lambda: len(roster()) >= 2)

    print("His reply to something said a while back (the quote line)")
    report("a1", shop, "said", "Should the cart keep items for a week or a month?")
    asked = until(lambda: said_by("a1", "a week or a month"))
    report("a1", shop, "said", "Meanwhile I tidied the checkout tests.")
    until(lambda: said_by("a1", "tidied the checkout"))
    open(os.path.join(got, "a1"), "w").close()
    say("A month.", re=asked["id"] if asked else "none")
    got_a1 = until(lambda: received("a1"))
    check("an agent that spoke since its question gets the question quoted with his answer", 'This answers what you asked earlier' in got_a1 and "a week or a month" in got_a1 and "A month." in got_a1, got_a1[-300:])
    latest = said_by("a1", "tidied the checkout")
    open(os.path.join(got, "a1"), "w").close()
    say("Thanks.", re=latest["id"] if latest else "none")
    got_a1 = until(lambda: received("a1"))
    check("a reply to its newest message carries no quote", "Thanks." in got_a1 and "This answers what you asked earlier" not in got_a1, got_a1[-300:])

    print("Kept across a restart of the tower")
    report("b1", docs, "said", "Is the guide to go out today?")
    question = until(lambda: said_by("b1", "guide to go out"))
    team = next((a.get("team") for a in roster() if a["id"] == "b1"), None)
    renamed = http("POST", "/v1/radio/team/rename", {"team": team, "name": "handbook"})
    before = [m["id"] for m in messages()]
    down(tower); tower = up()
    after = [m["id"] for m in messages()]
    check("the chat is all there after a restart", before and all(i in after for i in before), (len(before), len(after)))
    check("both agents are still on the radio", {"a1", "b1"} <= {a["id"] for a in roster()}, [a["id"] for a in roster()])
    b1 = next((a for a in roster() if a["id"] == "b1"), {})
    check("a team he renamed keeps its name", renamed[0] == 200 and "handbook" in json.dumps(b1), (renamed, b1.get("callsign"), b1.get("team")))
    still = next((m for m in messages() if question and m["id"] == question["id"]), {})
    check("a question that was open is still open", still.get("open") or still.get("needs_you"), still)
    # N12: an agent that has gone is let go, and that is kept: it does not come back with the next restart
    gone = subprocess.Popen(["sleep", "30"]); report("c1", shop, "--pid", str(gone.pid), "checkin")
    until(lambda: any(a["id"] == "c1" for a in roster()))
    gone.kill(); gone.wait()
    let_go = until(lambda: not any(a["id"] == "c1" for a in roster()), 40)
    down(tower); tower = up()
    check("N12: an agent whose process ended is let go, and stays gone after a restart", let_go and not any(a["id"] == "c1" for a in roster()), [a["id"] for a in roster()])

    print("A report the tower took late is taken once (N43)")
    tower.send_signal(signal.SIGSTOP)
    late = report("a1", shop, "said", "Said while the tower was slow.")
    tower.send_signal(signal.SIGCONT)
    time.sleep(8)  # the tower takes what was kept for it every five seconds
    times = sum("Said while the tower was slow." in m.get("text", "") for m in messages())
    check("N43: a report that timed out while the tower was slow, and that it then took, is in the chat once", times == 1, f"{times} times; report said: {late.stdout.strip()} {late.stderr.strip()}")

    print("Files of the tower's own that are damaged (A-5, N14)")
    down(tower)
    kept = sorted(os.path.relpath(os.path.join(d, f), os.path.join(home, ".airband")) for d, _, fs in os.walk(os.path.join(home, ".airband")) for f in fs if f.endswith(".json") and f != "config.json" and "spool" not in d)
    check("the tower keeps some files of its own to try this on", kept, os.listdir(os.path.join(home, ".airband")))
    for rel in kept:
        path = os.path.join(home, ".airband", rel)
        damaged = b'{"damaged by the test ' + os.urandom(4).hex().encode() + b'"'
        open(path, "wb").write(damaged)
        tower = up()
        alive = http("GET", "/v1/status", token=OWN)[0] == 200
        # something for it to write
        report("a1", shop, "said", f"After {rel} was damaged.")
        until(lambda: said_by("a1", f"After {rel}"), 8)
        down(tower)
        folder = os.path.dirname(path)
        aside = any(open(os.path.join(folder, f), "rb").read() == damaged for f in os.listdir(folder) if os.path.isfile(os.path.join(folder, f)))
        check(f"A-5/N14: {rel} damaged: the tower still starts, and what was there is kept, never written over", alive and aside, f"started: {alive}, kept: {aside}")
    tower = up()

    print("His settings damaged")
    down(tower)
    cfg_path = os.path.join(home, ".airband", "config.json")
    good = open(cfg_path, "rb").read()
    open(cfg_path, "wb").write(b'{"port": 7398, "tok')
    try: refused = subprocess.run([BIN, "run"], env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20)
    except subprocess.TimeoutExpired as e: refused = subprocess.CompletedProcess(e.cmd, 0, "", "it ran on, with settings it could not read")
    check("a settings file that does not read stops the tower, with a reason, and is left as it was",
          refused.returncode != 0 and (refused.stderr or refused.stdout).strip() and open(cfg_path, "rb").read() == b'{"port": 7398, "tok', (refused.returncode, (refused.stderr or refused.stdout)[-200:]))
    open(cfg_path, "wb").write(good)
    tower = up()
finally:
    try: tower.send_signal(signal.SIGCONT)
    except Exception: pass
    down(tower)
failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)} of {len(results)} passed." + ("" if not failed else "  Failed:\n  - " + "\n  - ".join(failed) + f"\nLogs: {work}"))
if not failed:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(1 if failed else 0)
