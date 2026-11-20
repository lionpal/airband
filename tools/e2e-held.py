#!/usr/bin/env python3
"""End to end, for agents the tower runs itself: a real tower, a throwaway home, and a stand-in agent.

    tools/e2e-held.py

The stand-in is a small program that does what a held agent's program must: it reads one message a line on
standard input for as long as it runs, and reports to the tower like any other agent (`airband report`). Asked
to "work N", it is busy for N seconds first. So everything about holding an agent can be tried in seconds and
with no model: his words arriving as they are, what he says meanwhile waiting its turn, taking one back,
stopping it, the rest then read together, the agent going on through a restart of the tower, and ending it.
"""
import json, os, shutil, signal, subprocess, sys, tempfile, time, urllib.request, urllib.error

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "airband")
PORT, OWN, RADIO = 7397, "held-own", "held-radio"
work = tempfile.mkdtemp(prefix="airband-held-")
home, project, got = os.path.join(work, "home"), os.path.join(work, "code", "shop"), os.path.join(work, "got")
for d in (os.path.join(home, ".airband"), project, got):
    os.makedirs(d)

# The stand-in agent. Each message it is given is appended to a file, so the test can read exactly what arrived.
AGENT = os.path.join(work, "agent.py")
open(AGENT, "w").write(r'''
import json, os, subprocess, sys, threading, time
BIN, ID, CWD, GOT = sys.argv[1:5]
def report(*words):
    subprocess.run([BIN, "report", "--id", ID, "--harness", "stand-in", "--cwd", CWD, "--held", *words])
report("checkin")
stop = threading.Event()
turns = []
lock = threading.Lock()
def reader():
    for line in sys.stdin:
        try: m = json.loads(line)
        except ValueError: continue
        if m.get("stop"):
            stop.set()
            continue
        with lock: turns.append(m["text"])
    with lock: turns.append(None)
threading.Thread(target=reader, daemon=True).start()
while True:
    with lock: text = turns.pop(0) if turns else ""
    if text is None: break
    if text == "":
        time.sleep(0.05)
        continue
    open(GOT, "a").write(json.dumps(text) + "\n")
    report("working")
    busy = float(text.split("work ")[1].split()[0]) if "work " in text else 0
    # a stop that came after this message was read stops this turn, even if the turn had not begun yet
    stopped = stop.wait(busy) if busy else False
    stop.clear()
    report("said", "Stopped part way." if stopped else "Done: " + text.splitlines()[0][:60])
report("signoff")
''')
json.dump({
    "port": PORT, "token": OWN, "radio_token": RADIO, "name": "Charles", "station": "held", "summarizer": {"kind": "none"}, "projects": {"shop": project},
    "launcher": "stand-in",
    "launchers": {"stand-in": {"argv": [sys.executable, AGENT, BIN, "{id}", "{cwd}", os.path.join(got, "said")],
                               "held": {"fresh": [], "resume": [], "line": '{"text":{text}}', "stop": '{"stop":"{id}"}'}}},
}, open(os.path.join(home, ".airband", "config.json"), "w"))
env = dict(os.environ, HOME=home)
env.pop("AIRBAND_INTERNAL", None)

results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def http(method, path, body=None, token=RADIO):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json", "x-airband-radio": "held"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        return e.code, None
    except urllib.error.URLError:
        return 0, None
say = lambda text: http("POST", "/v1/radio/utterance", {"text": text, "id": os.urandom(6).hex()})[1]["say"]
feed = lambda: http("GET", "/v1/radio/feed?since=0")[1]
arrived = lambda: [json.loads(l) for l in open(os.path.join(got, "said"))] if os.path.exists(os.path.join(got, "said")) else []
def agent():
    held = [a for a in feed()["roster"] if a.get("held")]
    return held[0] if held else None
def until(what, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        v = what()
        if v: return v
        time.sleep(0.2)
    return what()
def start_tower():
    t = subprocess.Popen([BIN, "run"], env=env, stdout=open(os.path.join(work, "tower.log"), "a"), stderr=subprocess.STDOUT)
    for _ in range(100):
        if http("GET", "/v1/status", token=OWN)[0] == 200: return t
        time.sleep(0.1)
    sys.exit("The tower did not start. Its log: " + os.path.join(work, "tower.log"))

tower = start_tower()
try:
    print("An agent the tower runs")
    check("it is started by the tower and reports in, marked as held", say("/launch shop say hello to the shop") == "Launching in shop." and until(agent), feed()["roster"])
    me = agent()
    check("its task reaches it as his words and nothing else", until(lambda: arrived() == ["say hello to the shop"]), arrived())
    until(lambda: agent()["state"] != "working")
    check("what he says next reaches it the same way", say("@shop1 the second thing") == "Sent to shop one." and until(lambda: arrived()[-1:] == ["the second thing"]), arrived())

    print("While it is in the middle of something")
    until(lambda: agent()["state"] != "working")
    say("@shop1 work 30 on the long job")
    until(lambda: agent()["state"] == "working")
    say("@shop1 FIRST afterthought")
    say("@shop1 SECOND afterthought")
    mine = lambda has: [m for m in feed()["messages"] if m["from"] == "you" and has in m["text"]]
    check("what he says waits its turn, and says so", mine("FIRST")[0].get("queued") and mine("SECOND")[0].get("queued") and "FIRST afterthought" not in arrived(), (mine("FIRST"), arrived()))
    check("he can take one back", http("POST", "/v1/radio/unsend", {"message": mine("FIRST")[0]["id"]})[1] == {} and mine("FIRST")[0].get("cancelled") and not mine("FIRST")[0].get("queued"), mine("FIRST"))
    check("but only once", "error" in http("POST", "/v1/radio/unsend", {"message": mine("FIRST")[0]["id"]})[1])
    began = time.time()
    check("he can stop it, at once", http("POST", "/v1/radio/stop", {"agent": me["id"]})[1] == {} and time.time() - began < 3, time.time() - began)
    check("what was still waiting is then given to it, and what he took back is not", until(lambda: arrived()[-1:] == ["SECOND afterthought"]) and "FIRST afterthought" not in arrived(), arrived())
    check("and his message no longer says it is waiting", until(lambda: not mine("SECOND")[0].get("queued")), mine("SECOND"))
    until(lambda: agent()["state"] != "working")
    check("one that is doing nothing cannot be stopped, in words", "error" in http("POST", "/v1/radio/stop", {"agent": me["id"]})[1])
    say("@shop1 work 2 on a short job")
    until(lambda: agent()["state"] == "working")
    say("@shop1 LATER: one more thing")
    check("with nobody stopping it, what waited is given to it when it finishes", until(lambda: arrived()[-1:] == ["LATER: one more thing"], 12), arrived())

    print("Through a restart of the tower")
    until(lambda: agent()["state"] != "working")
    say("@shop1 work 6 across the restart")
    until(lambda: agent()["state"] == "working")
    say("@shop1 SAID BEFORE THE RESTART")
    tower.terminate(); tower.wait(timeout=10)
    time.sleep(1)
    tower = start_tower()
    check("the agent is still there, and still held", until(agent) and agent()["id"] == me["id"], feed()["roster"])
    check("it finishes what it was doing, and reports it to the new tower", until(lambda: [m for m in feed()["messages"] if m.get("agent") == me["id"] and "across the restart" in m["text"]], 15), [m["text"] for m in feed()["messages"]][-3:])
    check("and what he said before the restart is given to it afterwards", until(lambda: arrived()[-1:] == ["SAID BEFORE THE RESTART"], 12), arrived())
    check("each thing he said reached it once", len(arrived()) == len(set(arrived())), arrived())

    print("Ending it")
    until(lambda: agent() and agent()["state"] != "working")
    check("he can end it, and it leaves the radio", http("POST", "/v1/radio/end", {"agent": me["id"]})[1] == {} and until(lambda: agent() is None), feed()["roster"])
    check("one that has gone cannot be ended again, in words", "error" in http("POST", "/v1/radio/end", {"agent": me["id"]})[1])
    check("nothing of it is left running", not subprocess.run(["pgrep", "-f", AGENT], capture_output=True, text=True).stdout.strip())

    print("Started with nothing to do yet, as the + on a team does")
    before = len(arrived())
    check("it is there at once, having been told nothing", say("/launch shop") == "A new agent is ready in shop." and until(agent) and len(arrived()) == before, (feed()["roster"], arrived()))
    fresh = agent()
    check("and it is waiting, not working", fresh and fresh["state"] not in ("working", "asking"), fresh)
    check("his first word reaches it as his own", say("@" + fresh["callsign"].replace(" ", "") + " start with the tests").startswith("Sent to") and until(lambda: arrived()[-1:] == ["start with the tests"]), arrived())
    until(lambda: agent() and agent()["state"] != "working")
    check("it can be ended like any other", http("POST", "/v1/radio/end", {"agent": fresh["id"]})[1] == {} and until(lambda: agent() is None), feed()["roster"])

    print("A new team, made from the phone")
    os.makedirs(os.path.join(home, "work", "garden"))
    top = http("POST", "/v1/radio/folders", {"under": ""})[1]
    check("his home folder is listed, without the hidden folders", top and "work" in top["folders"] and not any(n.startswith(".") for n in top["folders"]) and top.get("up") is None and top["shown"] == "~", top)
    inside = http("POST", "/v1/radio/folders", {"under": top["path"] + "/work"})[1]
    check("a folder inside it is walked into", inside and inside["folders"] == ["garden"] and inside["shown"] == "~/work" and inside["up"] == top["path"], inside)
    check("nothing outside his home folder is listed", "error" in http("POST", "/v1/radio/folders", {"under": "/etc"})[1])
    check("a team's name is one plain word", "error" in http("POST", "/v1/radio/project", {"name": "my team!", "folder": inside["path"]})[1])
    check("a team is not made outside his home folder", "error" in http("POST", "/v1/radio/project", {"name": "etc", "folder": "/etc"})[1])
    made = http("POST", "/v1/radio/project", {"name": "Garden", "folder": inside["path"] + "/garden"})[1]
    check("a new team is made in the folder he chose, and its first agent is there with nothing to do", made and "error" not in made and until(lambda: any(a["group"] == "garden" for a in feed()["roster"])), (made, feed()["roster"]))
    made = http("POST", "/v1/radio/project", {"name": "orchard", "folder": inside["path"] + "/orchard", "make": True})[1]
    check("a folder that is not there yet is made for it", made and "error" not in made and os.path.isdir(os.path.join(home, "work", "orchard")), made)
    check("a name another folder already has is refused", "error" in http("POST", "/v1/radio/project", {"name": "garden", "folder": inside["path"]})[1])
finally:
    tower.terminate()
    try: tower.wait(timeout=10)
    except subprocess.TimeoutExpired: tower.kill()
    # the stand-in's holder, if a failed run left one: found by this run's own file, and no other way
    for pid in subprocess.run(["pgrep", "-f", AGENT], capture_output=True, text=True).stdout.split():
        try: os.kill(int(pid), signal.SIGTERM)
        except (ProcessLookupError, ValueError): pass
failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)} of {len(results)} passed." + ("" if not failed else "  Failed:\n  - " + "\n  - ".join(failed) + f"\nThe tower's log: {os.path.join(work, 'tower.log')}"))
if not failed and "--keep" not in sys.argv:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(1 if failed else 0)
