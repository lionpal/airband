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
# A second stand-in, for an agent driven through the Agent Client Protocol: JSON-RPC on its standard input and
# output, and nothing reported by itself. It takes a second to open (as a real one takes a moment), keeps what it
# was asked in a file, and does what its prompt says: "say X", "nothing", "fail", "ask" (for leave), "work N".
ACP = os.path.join(work, "acp.py")
open(ACP, "w").write(r'''
import json, sys, threading, time
note = lambda *words: open(sys.argv[1], "a").write(" ".join(words) + "\n")
out = lambda m: (sys.stdout.write(json.dumps(m) + "\n"), sys.stdout.flush())
chunk = lambda sid, text: out({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})
cancel, asked = threading.Event(), {}
def turn(mid, sid, text):
    if text.startswith("say "): chunk(sid, text[4:])
    elif text == "fail": return out({"jsonrpc": "2.0", "id": mid, "error": {"code": -32000, "message": "the model refused the request"}})
    elif text == "ask":
        asked[900] = (mid, sid)
        return out({"jsonrpc": "2.0", "id": 900, "method": "session/request_permission", "params": {"sessionId": sid, "toolCall": {"toolCallId": "c1", "title": "Deploy the shop", "kind": "execute", "rawInput": {"command": "make deploy"}},
            "options": [{"optionId": "for-ever", "kind": "allow_always", "name": "Always"}, {"optionId": "this-once", "kind": "allow_once", "name": "Yes"}, {"optionId": "not-now", "kind": "reject_once", "name": "No"}]}})
    elif text.startswith("work "):
        if cancel.wait(float(text.split()[1])):
            cancel.clear()
            return out({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "cancelled"}})
        chunk(sid, "worked")
    out({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
for line in sys.stdin:
    m = json.loads(line)
    method, mid, p = m.get("method"), m.get("id"), m.get("params") or {}
    if method == "initialize":
        time.sleep(1)
        out({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}})
    elif method == "session/new":
        note("new"); out({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": "ses-stand-in", "models": {"currentModelId": "stand-in-1"}}})
    elif method == "session/load":
        note("load", p["sessionId"]); out({"jsonrpc": "2.0", "id": mid, "result": {}})
    elif method == "session/prompt":
        note("prompt", p["prompt"][0]["text"])
        threading.Thread(target=turn, args=(mid, p["sessionId"], p["prompt"][0]["text"]), daemon=True).start()
    elif method == "session/cancel": cancel.set()
    elif method is None and mid in asked:
        prompt, sid = asked.pop(mid)
        outcome = m["result"]["outcome"]
        note("answer", outcome.get("optionId") or outcome["outcome"])
        chunk(sid, "answered " + (outcome.get("optionId") or outcome["outcome"]))
        out({"jsonrpc": "2.0", "id": prompt, "result": {"stopReason": "end_turn"}})
note("ended")
''')
json.dump({
    "port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "held", "summarizer": {"kind": "none"}, "projects": {"shop": project},
    "launcher": "stand-in",
    "launchers": {"stand-in": {"argv": [sys.executable, AGENT, BIN, "{id}", "{cwd}", os.path.join(got, "said")],
                               "held": {"by": "lines", "fresh": [], "resume": [], "line": '{"text":{text}}', "stop": '{"stop":"{id}"}'}},
                  "stand-acp": {"harness": "stand-acp", "held": {"by": "acp", "program": [sys.executable, ACP, os.path.join(got, "acp")]}},
                  "broken": {"harness": "broken", "held": {"by": "acp", "program": [sys.executable, "-c", "import sys; sys.stderr.write('you are not signed in\\n'); sys.exit(3)"]}}},
}, open(os.path.join(home, ".airband", "config.json"), "w"))
# A stand-in for OpenCode's server: the few calls the tower's watcher makes, answered from what the test sets.
# It listens on a port of its own, and says where in the file OpenCode says it in.
import base64, http.server, socket, threading
class Fake:
    sessions, messages, pending, replies, prompts, unauthorised = [], {}, {}, [], [], 0
PASSWORD = os.urandom(8).hex()
class OpenCode(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def answer(self, code, body=None):
        self.send_response(code); self.send_header("content-type", "application/json"); self.end_headers()
        if body is not None: self.wfile.write(json.dumps(body).encode())
    def allowed(self):
        ok = self.headers.get("authorization") == "Basic " + base64.b64encode(f"opencode:{PASSWORD}".encode()).decode()
        if not ok: Fake.unauthorised += 1; self.answer(401)
        return ok
    def do_GET(self):
        if not self.allowed(): return
        parts = self.path.split("?")[0].strip("/").split("/")
        if parts == ["api", "session"]: return self.answer(200, {"data": Fake.sessions})
        if len(parts) == 4 and parts[3] == "message": return self.answer(200, {"data": Fake.messages.get(parts[2], [])})
        if len(parts) == 4 and parts[3] == "permission": return self.answer(200, {"data": Fake.pending.get(parts[2], [])})
        self.answer(404)
    def do_POST(self):
        if not self.allowed(): return
        parts, body = self.path.strip("/").split("/"), json.loads(self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}")
        if len(parts) == 4 and parts[3] == "prompt": Fake.prompts.append((parts[2], body["text"])); return self.answer(200, {})
        if len(parts) == 6 and parts[5] == "reply":
            Fake.replies.append((parts[4], body)); Fake.pending[parts[2]] = [r for r in Fake.pending.get(parts[2], []) if r["id"] != parts[4]]
            return self.answer(204)
        self.answer(404)
fake = http.server.ThreadingHTTPServer(("127.0.0.1", 0), OpenCode)
threading.Thread(target=fake.serve_forever, daemon=True).start()
os.makedirs(os.path.join(work, "state", "opencode"))
json.dump({"url": f"http://127.0.0.1:{fake.server_address[1]}", "password": PASSWORD}, open(os.path.join(work, "state", "opencode", "service.json"), "w"))
env = dict(os.environ, HOME=home, XDG_STATE_HOME=os.path.join(work, "state"))
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
    check("nothing of it is left running, a moment later", until(lambda: not subprocess.run(["pgrep", "-f", AGENT], capture_output=True, text=True).stdout.strip(), 5))

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

    print("An agent that speaks the Agent Client Protocol")
    noted = lambda: open(os.path.join(got, "acp")).read().split("\n")[:-1] if os.path.exists(os.path.join(got, "acp")) else []
    one = lambda: next((a for a in feed()["roster"] if a.get("harness") == "stand-acp"), None)
    from_it = lambda: [m for m in feed()["messages"] if one() and m.get("agent") == one()["id"]]
    began = time.time()
    check("he is told it is ready only once it has opened its session", say("/launch shop tool:stand-acp") == "A new agent is ready in shop." and time.time() - began >= 1 and noted() == ["new"], (time.time() - began, noted()))
    acp = one()
    # its session shows in its tool's own server too, as a real one's does: the driver reports for it, and nothing else
    Fake.sessions = [{"id": "ses-stand-in", "time": {"created": int(time.time() * 1000), "updated": int(time.time() * 1000)}, "location": {"directory": project}}]
    time.sleep(5)
    check("a session the tower runs is not put on the radio a second time by the watcher of its tool's server", not [a for a in feed()["roster"] if a.get("harness") == "opencode"], feed()["roster"])
    Fake.sessions = []
    check("it is on the radio, held, waiting, and says what it runs on", acp and acp.get("held") and acp["state"] not in ("working", "asking") and until(lambda: one().get("model") == "stand-in-1"), acp)
    call = "@" + acp["callsign"].replace(" ", "") + " "
    check("his words are its prompt, and what it writes is what it said", say(call + "say the shelves are stocked").startswith("Sent to") and until(lambda: [m for m in from_it() if m["text"] == "the shelves are stocked"]), [m["text"] for m in from_it()])
    say(call + "nothing")
    check("a turn that ends with nothing said leaves it waiting, not working for ever", until(lambda: "prompt nothing" in noted()) and until(lambda: one()["state"] != "working", 5), one())
    say(call + "fail")
    check("a turn that fails is told to him, in the agent's own words", until(lambda: [m for m in from_it() if "the model refused the request" in m["text"]], 8) and one()["state"] != "working", [m["text"] for m in from_it()])
    say(call + "work 30")
    until(lambda: one()["state"] == "working")
    say(call + "say AFTER THE STOP")
    check("it can be stopped, and what he said meanwhile is then its next prompt", http("POST", "/v1/radio/stop", {"agent": acp["id"]})[1] == {} and until(lambda: [m for m in from_it() if m["text"] == "AFTER THE STOP"], 8), (noted(), [m["text"] for m in from_it()]))
    say(call + "ask")
    card = until(lambda: [m for m in feed()["messages"] if m.get("permission")], 8)
    check("its request for leave is put to him as the command itself", card and card[0]["text"] == "Wants to run `make deploy`. Allow?" and card[0]["options"] == ["Allow", "Deny"], card)
    http("POST", "/v1/radio/utterance", {"text": "Allow", "id": os.urandom(6).hex(), "re": card[0]["id"]})
    check("his yes picks the agent's own choice for this once, never the one for always", until(lambda: "answer this-once" in noted(), 8), noted())
    until(lambda: one()["state"] != "working")
    say(call + "ask")
    card = until(lambda: [m for m in feed()["messages"] if m.get("permission")], 8)
    http("POST", "/v1/radio/utterance", {"text": "Deny", "id": os.urandom(6).hex(), "re": card[0]["id"]})
    check("and his no picks its refusal", until(lambda: "answer not-now" in noted(), 8), noted())
    until(lambda: one()["state"] != "working")
    # its holder goes (the computer was restarted): the next thing he says carries the same session on
    holder = subprocess.run(["pgrep", "-f", "hold --sock .* --id " + acp["id"]], capture_output=True, text=True).stdout.split()
    for pid in holder: os.kill(int(pid), signal.SIGKILL)
    time.sleep(0.5)
    check("with its holder gone, his next words carry on the session it had, by the agent's own name for it", say(call + "say back again").startswith("Sent to") and until(lambda: "load ses-stand-in" in noted() and [m for m in from_it() if m["text"] == "back again"], 10), noted())
    check("it can be ended, and its program ends with it", http("POST", "/v1/radio/end", {"agent": acp["id"]})[1] == {} and until(lambda: one() is None) and until(lambda: not subprocess.run(["pgrep", "-f", ACP], capture_output=True, text=True).stdout.strip(), 8), feed()["roster"])

    print("A tool that is a server the tower watches (as OpenCode is)")
    clock = lambda: int(time.time() * 1000)
    watched = lambda: next((a for a in feed()["roster"] if a.get("harness") == "opencode"), None)
    Fake.sessions = [{"id": "ses_watched", "time": {"created": clock(), "updated": clock()}, "location": {"directory": project}}]
    check("a session of its own that he opened is on the radio, with nothing installed", until(watched, 8), feed()["roster"])
    Fake.sessions[0]["time"].update(busy=clock(), updated=clock())
    check("it is working when the server says it is busy", until(lambda: watched()["state"] == "working", 8), watched())
    request = lambda n, command: {"id": f"per_{n}", "sessionID": "ses_watched", "action": "shell", "resources": [command], "save": [], "source": {"type": "tool", "messageID": "msg_1", "id": "call_1"}}
    leave = lambda: [m for m in feed()["messages"] if m.get("permission") and m.get("agent") == "ses_watched"]
    Fake.pending["ses_watched"] = [request(1, "make deploy")]
    card = until(leave, 8)
    check("its request for leave is put to him as the command itself", card and card[0]["text"] == "Wants to run `make deploy`. Allow?", card)
    http("POST", "/v1/radio/utterance", {"text": "Allow", "id": os.urandom(6).hex(), "re": card[0]["id"]})
    check("his yes is sent to the server as this once", until(lambda: ("per_1", {"decision": "once", "message": None}) in Fake.replies, 8), Fake.replies)
    Fake.pending["ses_watched"] = [request(2, "rm -rf build")]
    card = until(leave, 8)
    http("POST", "/v1/radio/utterance", {"text": "Deny", "id": os.urandom(6).hex(), "re": card[0]["id"]})
    check("his no is sent to the server as a refusal, and says it came from his phone", until(lambda: [b for i, b in Fake.replies if i == "per_2" and b["decision"] == "reject" and "phone" in (b["message"] or "")], 8), Fake.replies)
    Fake.pending["ses_watched"] = [request(3, "git push")]
    card = until(leave, 8)
    Fake.pending["ses_watched"] = []
    check("one answered on the tool's own screen is taken back from the phone, and nothing is sent for it", until(lambda: not leave(), 10) and not [i for i, _ in Fake.replies if i == "per_3"], (leave(), Fake.replies))
    Fake.messages["ses_watched"] = [{"type": "assistant", "content": [{"type": "reasoning", "text": "thinking"}, {"type": "text", "text": "The shop is deployed."}]}]
    Fake.sessions[0]["time"].update(idle=clock(), updated=clock())
    check("what it says when it goes idle is its report", until(lambda: [m for m in feed()["messages"] if m.get("agent") == "ses_watched" and m["text"] == "The shop is deployed."], 8), [m["text"] for m in feed()["messages"]][-3:])
    check("his words go into that session through the server, as his own turn", say("@" + watched()["callsign"].replace(" ", "") + " now the tests").startswith("Sent to") and until(lambda: ("ses_watched", "now the tests") in Fake.prompts, 8), Fake.prompts)
    Fake.sessions.append({"id": "ses_put_away", "time": {"created": clock(), "updated": clock(), "archived": clock()}, "location": {"directory": project}})
    Fake.sessions = [s for s in Fake.sessions if s["id"] != "ses_watched"]
    check("one that is gone from the server leaves the radio, and one put away never joins it", until(lambda: watched() is None, 8), feed()["roster"])
    check("every call to the server carried its password", Fake.unauthorised == 0, Fake.unauthorised)
    began = time.time()
    told = say("/launch shop tool:broken")
    check("one that cannot start is not announced: he is told why, in the program's own words", "Could not launch in shop" in told and "you are not signed in" in told and not [a for a in feed()["roster"] if a.get("harness") == "broken"], told)
    check("a tool this computer cannot start is refused in words, and nothing is started", "not set up to start nothing agents" in say("/launch shop tool:nothing"))
finally:
    tower.terminate()
    try: tower.wait(timeout=10)
    except subprocess.TimeoutExpired: tower.kill()
    # the stand-in's holder, if a failed run left one: found by this run's own file, and no other way
    for pid in subprocess.run(["pgrep", "-f", ACP], capture_output=True, text=True).stdout.split() + subprocess.run(["pgrep", "-f", AGENT], capture_output=True, text=True).stdout.split():
        try: os.kill(int(pid), signal.SIGTERM)
        except (ProcessLookupError, ValueError): pass
failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)} of {len(results)} passed." + ("" if not failed else "  Failed:\n  - " + "\n  - ".join(failed) + f"\nThe tower's log: {os.path.join(work, 'tower.log')}"))
if not failed and "--keep" not in sys.argv:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(1 if failed else 0)
