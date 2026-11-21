#!/usr/bin/env python3
"""End to end: a real tower, real model calls, pretend agents, and (with --claude) one real Claude Code session.

It starts a tower in a throwaway home on its own port, puts six agents on it the way the homepage's demo has
them (two teams with two agents each, two teams with one), and goes through everything a radio can do, checking
each result. Nothing here touches the real tower or its files.

    cargo build --release && tools/e2e.py            about two minutes, some dozens of small model calls
    tools/e2e.py --local                             the same, with the model the tower runs itself doing the reading
    tools/e2e.py --claude                            also asks a real `claude -p` session for permission
    tools/e2e.py --gemini                            the same, with Gemini as the model that reads everything

Checks that depend on a model's judgement (what kind a message is, who an untagged reply is for) can fail now
and then for that reason alone; they are marked (model).
"""
import json, os, shutil, subprocess, sys, tempfile, threading, time, urllib.request, urllib.error

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "airband")
PORT = 7391
REAL_HOME = os.path.expanduser("~")
work = tempfile.mkdtemp(prefix="airband-e2e-")
home, projects, got = os.path.join(work, "home"), os.path.join(work, "code"), os.path.join(work, "got")
for d in (os.path.join(home, ".airband"), got, *(os.path.join(projects, p) for p in ("catalog", "logistics", "orders", "payouts"))):
    os.makedirs(d)
OWN, RADIO = "e2e-own-token", "e2e-radio-token"
# the model that reads everything: Claude Code as a command, or with --gemini a Gemini key given in the
# environment (GEMINI_API_KEY)
MODEL = {"kind": "command", "offer": "own", "argv": ["env", f"HOME={REAL_HOME}", "claude", "-p", "--model", "haiku", "--no-session-persistence", "--strict-mcp-config",
                                     "--system-prompt", "You rewrite messages. Reply with the JSON asked for and nothing else.", "--tools", ""]}
if "--gemini" in sys.argv:
    MODEL = {"kind": "gemini", "offer": "flash-lite", "key": os.environ["GEMINI_API_KEY"]}
    if os.environ.get("GEMINI_MODEL"):
        MODEL["model"] = os.environ["GEMINI_MODEL"]
# --local runs everything through the model the tower fetches and runs itself (catalog/providers/qwen.json).
# AIRBAND_LOCAL_CACHE names a folder that already has the model, so it is not fetched again (2.7 GB).
if "--local" in sys.argv:
    MODEL = {"kind": "qwen", "offer": "qwen-4b"}
    if os.environ.get("AIRBAND_LOCAL_CACHE"):
        os.makedirs(os.path.join(home, ".airband", "local"))
        os.symlink(os.environ["AIRBAND_LOCAL_CACHE"], os.path.join(home, ".airband", "local", "models"))
json.dump({
    "port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "e2e", "permission_wait": 25,
    "summarizer": MODEL,
    "launchers": {"test": {"argv": ["sh", "-c", 'printf "%s\\n" "$@" > "{cwd}/launched.txt"', "sh", "{options}", "{task}"], "models": {"fable": "fable-id", "opus": "opus-id"},
                           "efforts": ["low", "medium", "high"], "with_model": ["--model", "{model}"], "with_effort": ["--effort", "{effort}"]}}, "launcher": "test",
    "quick": [{"label": "Make no mistakes", "text": "Make no mistakes."}],
}, open(os.path.join(home, ".airband", "config.json"), "w"))
# Accounts are tried against made-up logins: two keychain items under names of this run's own (deleted at the
# end) and a stand-in for Anthropic's servers. No real login is read or changed.
RUN = os.urandom(4).hex()
LIVE_ITEM, VAULT_ITEMS, STAND_IN = f"airband-e2e-live-{RUN}", f"airband-e2e-vault-{RUN}", 7392
env = dict(os.environ, HOME=home, AIRBAND_TRACE="1", AIRBAND_CLAUDE_API=f"http://127.0.0.1:{STAND_IN}", AIRBAND_CLAUDE_TOKEN_URL=f"http://127.0.0.1:{STAND_IN}/token",
           AIRBAND_CLAUDE_LIVE_SERVICE=LIVE_ITEM, AIRBAND_CLAUDE_VAULT_SERVICE=VAULT_ITEMS)
env.pop("CLAUDE_CONFIG_DIR", None)

results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)

# Where each radio here says it stands, which every request of a radio's says: the one that types has the chat
# in front of him; the ones that read aloud are on a call.
STANDS = {"reader": {"on_call": True, "showing_chat": False}, "reader2": {"on_call": True, "showing_chat": False}}
def http(method, path, body=None, token=RADIO, radio="e2e", timeout=90):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json", "x-airband-radio": radio,
                                          "x-airband-presence": json.dumps(STANDS.get(radio, {"on_call": False, "showing_chat": True}))})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        return e.code, None
    except urllib.error.URLError:
        return 0, None

def report(agent, project, title, *words):
    """An agent's harness reports something, through the generic adapter."""
    out = os.path.join(got, agent)
    return subprocess.run([BIN, "report", "--id", agent, "--harness", "claude-code", "--cwd", os.path.join(projects, project), "--title", title,
                           "--deliver-command", f"sh -c cat>>{out}", *words], env=env, capture_output=True, text=True)
def received(agent):
    path = os.path.join(got, agent)
    return open(path).read() if os.path.exists(path) else ""
def said(text, **more):
    """Say something and wait for what came of it. The tower takes it at once; asking again under the same id
    says where it stands, until it is answered."""
    body = {"text": text, "id": os.urandom(6).hex(), **more}
    for _ in range(1200):
        took = http("POST", "/v1/radio/utterance", body)[1]
        if took["state"] == "answered": return took["reply"]
        time.sleep(0.1)
def say(text, **more):
    return said(text, **more)["say"]
def feed(since=0):
    return http("GET", f"/v1/radio/feed?since={since}")[1]
def message(f, who, has=""):
    hits = [m for m in f["messages"] if m["from"] == who and has.lower() in m["text"].lower()]
    return hits[-1] if hits else None
def roster(f):
    return {a["callsign"]: a for a in f["roster"]}

def keychain(*args, **more):
    return subprocess.run(["security", *args], capture_output=True, text=True, timeout=15, stdin=subprocess.DEVNULL, **more)
def accounts_check():
    """Two made-up accounts: a@ and b@. Listing, allowances, a login that has to be refreshed, and switching."""
    import getpass
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    user, hour = getpass.getuser(), 3_600_000
    who = {"t-a-2": "a@example.com", "t-b-1": "b@example.com"}                       # token -> account
    used = {"a@example.com": {"seven_day": {"utilization": 90.0, "resets_at": "2026-11-25T01:00:00+00:00"}, "five_hour": {"utilization": 30, "resets_at": "2026-11-19T10:00:00Z"}},
            "b@example.com": {"seven_day": {"utilization": 20.0, "resets_at": "2026-11-22T01:00:00+00:00"}, "five_hour": None}}
    refreshed = []
    class StandIn(BaseHTTPRequestHandler):
        def log_message(self, *a): pass
        def answer(self, code, body):
            raw = json.dumps(body).encode()
            self.send_response(code); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(raw))); self.end_headers(); self.wfile.write(raw)
        def do_GET(self):
            email = who.get(self.headers.get("authorization", "").removeprefix("Bearer "))
            if not email: return self.answer(401, {})
            self.answer(200, {"account": {"email_address": email}} if self.path.endswith("/profile") else used[email])
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["content-length"])))
            refreshed.append(body.get("refresh_token"))
            if body.get("refresh_token") != "r-a-1": return self.answer(400, {"error": "invalid_grant"})
            self.answer(200, {"access_token": "t-a-2", "refresh_token": "r-a-2", "expires_in": 3600})
    server = ThreadingHTTPServer(("127.0.0.1", STAND_IN), StandIn)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    now = int(time.time() * 1000)
    def sign_in(email, token, refresh, expires):
        blob = {"claudeAiOauth": {"accessToken": token, "refreshToken": refresh, "expiresAt": expires, "scopes": ["user:inference", "user:profile"]}, "mcpOAuth": {"some-server": "kept"}}
        keychain("add-generic-password", "-U", "-a", user, "-s", LIVE_ITEM, "-w", json.dumps(blob))
        json.dump({"oauthAccount": {"emailAddress": email, "accountUuid": "id-" + email[0]}, "somethingElse": "kept"}, open(os.path.join(home, ".claude.json"), "w"))
    def cli(*args):
        r = subprocess.run([BIN, "accounts", *args], env=env, capture_output=True, text=True, timeout=60)
        return (r.stdout + r.stderr).strip()
    def live_now():
        return json.loads(keychain("find-generic-password", "-a", user, "-s", LIVE_ITEM, "-w").stdout)
    try:
        # a@ is signed in with a login that has already run out (its saved copy will need refreshing), then b@
        who["t-a-1"] = "a@example.com"
        sign_in("a@example.com", "t-a-1", "r-a-1", now - hour)
        check("the login in use is saved, by its account", cli("save") == "Saved a@example.com.")
        del who["t-a-1"]
        sign_in("b@example.com", "t-b-1", "r-b-1", now + hour)
        check("a second account is saved beside it", cli("save") == "Saved b@example.com.")
        code, got_ = http("GET", "/v1/radio/accounts?fresh=1")
        by = {a["label"]: a for a in (got_ or {}).get("accounts", [])}
        check("a radio is told both accounts and which is in use", code == 200 and set(by) == {"a@example.com", "b@example.com"} and by["b@example.com"]["live"] and not by["a@example.com"]["live"], got_)
        check("each with how much is used and when it starts again", by and [(l["name"], l["used"]) for l in by["a@example.com"]["limits"]] == [("week", 0.9), ("5 hours", 0.3)]
              and by["b@example.com"]["limits"] == [{"name": "week", "used": 0.2, "resets_at": 1795309200000}], by)
        check("a saved login that had run out was refreshed, once", refreshed == ["r-a-1"], refreshed)
        check("nothing that could sign in is sent to a radio", not any(secret in json.dumps(got_) for secret in ("t-a", "t-b", "r-a", "r-b")))
        code, after = http("POST", "/v1/radio/accounts/use", {"harness": "claude-code", "id": "a@example"})
        live = live_now()
        check("switching puts the other account's newest login in use", code == 200 and live["claudeAiOauth"]["accessToken"] == "t-a-2" and live["claudeAiOauth"]["refreshToken"] == "r-a-2", after)
        check("and leaves the other logins kept beside it alone", live.get("mcpOAuth") == {"some-server": "kept"})
        told = json.load(open(os.path.join(home, ".claude.json")))
        check("Claude Code is told which account it now is, and the rest of its file is kept", told["oauthAccount"]["emailAddress"] == "a@example.com" and told["somethingElse"] == "kept", told)
        check("the answer shows the new state", {a["label"]: a["live"] for a in (after or {}).get("accounts", [])} == {"a@example.com": True, "b@example.com": False}, after)
        check("the account switched away from can be switched back to", "▶ b@example.com" in cli("use", "b@") and live_now()["claudeAiOauth"]["accessToken"] == "t-b-1")
        code, refused = http("POST", "/v1/radio/accounts/use", {"harness": "claude-code", "id": "nobody@"})
        check("an account that is not saved is refused in words", code == 200 and "No saved account" in (refused or {}).get("error", ""), refused)
        check("the tower's own token is not needed, and an agent's harness that has no accounts says so",
              "no accounts" in (http("POST", "/v1/radio/accounts/use", {"harness": "codex", "id": "x"})[1] or {}).get("error", ""))
    finally:
        server.shutdown()
        keychain("delete-generic-password", "-a", user, "-s", LIVE_ITEM)
        for email in ("a@example.com", "b@example.com"):
            keychain("delete-generic-password", "-a", email, "-s", VAULT_ITEMS)

tower = subprocess.Popen([BIN, "run"], env=env, stdout=open(os.path.join(work, "tower.log"), "w"), stderr=subprocess.STDOUT)
try:
    for _ in range(100):
        if http("GET", "/v1/status", token=OWN)[0] == 200:
            break
        time.sleep(0.1)
    else:
        sys.exit("The tower did not start. Its log: " + os.path.join(work, "tower.log"))
    if "--local" in sys.argv:
        # the tower fetches the program that runs the model, then loads the model: minutes the first time.
        # Until it is ready a message is given as written, which is right, and not what is being checked here
        print("Waiting for the tower's own model to be ready")
        began = time.time()
        while "local model: ready" not in open(os.path.join(work, "tower.log")).read():
            if time.time() - began > 1200:
                sys.exit("The tower's own model was not ready in twenty minutes. Its log: " + os.path.join(work, "tower.log"))
            time.sleep(2)
        print(f"  ready after {time.time() - began:.0f} s")

    print("Nobody is there yet")
    r = subprocess.run([BIN, "report", "--id", "early", "--harness", "script", "--cwd", os.path.join(projects, "orders"), "permission", "run something"], env=env, capture_output=True, text=True, timeout=20)
    check("with no radio there, a permission request comes straight back unanswered (exit 2)", r.returncode == 2, r.returncode)
    subprocess.run([BIN, "report", "--id", "early", "--harness", "script", "signoff"], env=env)

    print("Six agents: two teams of two, two teams of one")
    shot = os.path.join(ROOT, "docs", "assets", "app.png")
    report("page", "catalog", "catalog part page", "show", shot)
    report("page", "catalog", "catalog part page", "said", "Part page B is built and the tests pass. One screenshot attached.")
    report("import", "catalog", "catalog import", "said", "Imported 1,240 parts from the new supplier. All tests pass.")
    report("rates", "logistics", "logistics rates", "said", "Two carriers are updated. I'll do the third one next.")
    report("refunds", "orders", "orders refunds", "said", "The duplicate charge bug is fixed. Refunds and an apology email for 14 double-charged customers are ready, and I would send them now. Send them, or show you first?")
    report("payouts", "payouts", "payouts schedule", "said", "The weekly payout summary email for sellers is built and checked. Preview: https://claude.ai/artifact/abc123 Should it go out on Mondays or Fridays?")
    report("letters", "letters", "letters seller notice", "said", "I'm still holding the notice to the seller. Approval for an email to a seller has to come from you in this session, and a yes relayed to me by another session or through Airband doesn't count. Reply here in this terminal with \"send the notice\" and I'll send it.")
    report("import", "catalog", "catalog import", "working")
    for _ in range(3):
        http("POST", "/v1/agent/events", {"schema": "airband.agent/1", "agent": {"id": "import", "harness": "claude-code"}, "event": {"type": "subagent", "what": "map columns"}}, token=OWN)
    f = feed()
    who = roster(f)
    check("sessions are called what he named them", set(who) == {"catalog part page", "catalog import", "logistics rates", "orders refunds", "payouts schedule", "letters seller notice"}, sorted(who))
    own = message(f, "letters seller notice")
    check("a question the agent will take only in its own session is marked so, with no buttons (model)", own and own.get("kind") == "needs-you" and own.get("own_session") is True and not own.get("options"), own)
    check("two agents share the catalog team", who["catalog part page"]["group"] == who["catalog import"]["group"] == "catalog")
    q = message(f, "orders refunds")
    check("a question is found, with its answers as buttons (model)", q and q.get("kind") == "needs-you" and len(q.get("options", [])) >= 2 and q["open"], q)
    check("the answer the agent itself recommended is marked (model)", q and q.get("recommended") == 0, q and q.get("recommended"))
    check("a long message that reports and then asks is cut to its news and, last, the question (model)", q and len(q["text"]) < 330 and q["long"] and "fix" in q["text"].lower() and q["text"].rstrip().endswith("?"), q and (len(q["text"]), q["long"], q["text"]))
    s = message(f, "logistics rates")
    check("an agent that stopped part way is seen as stalled (model)", s and s.get("kind") == "stalled" and who["logistics rates"]["state"] == "stalled", s and s.get("kind"))
    check("states: working, asking, idle", (who["catalog import"]["state"], who["orders refunds"]["state"], who["catalog part page"]["state"]) == ("working", "asking", "idle"),
          [(k, v["state"]) for k, v in who.items()])
    check("subagents are counted", who["catalog import"]["subagents"] == 3)
    p = message(f, "catalog part page")
    pic = p and p.get("pictures")
    check("a picture the agent sent comes with its message", bool(pic), p)
    if pic:
        code, part = http("GET", f"/v1/radio/picture/{pic[0]['id']}?from=0")
        import base64
        check("the picture can be fetched, as a JPEG", code == 200 and base64.b64decode(part["data"])[:2] == b"\xff\xd8")
    a = message(f, "payouts schedule")
    check("a link the agent wrote is kept beside the short text", a and a.get("links") == ["https://claude.ai/artifact/abc123"], a and a.get("links"))
    check("the tower offers the quick messages, his own last", [x["label"] for x in f["quick"]][-1] == "Make no mistakes" and len(f["quick"]) == 7)

    print("What a working agent is doing")
    report("import", "catalog", "catalog import", "doing", "Mapping the Northway columns")
    seen = feed()
    check("a harness can say what its agent is doing, and the roster carries it", roster(seen)["catalog import"].get("doing") == "Mapping the Northway columns" and "doing" not in roster(seen)["catalog part page"], roster(seen)["catalog import"])
    started = time.time()
    threading.Timer(1.0, lambda: report("import", "catalog", "catalog import", "doing", "Running the import tests")).start()
    later = http("GET", f"/v1/radio/feed?since={seen['rev']}&roster={seen['roster_rev']}&wait=20")[1]
    check("a radio waiting on the feed hears of it at once, with no message changed", time.time() - started < 6 and roster(later)["catalog import"].get("doing") == "Running the import tests" and not later["messages"], (time.time() - started, later.get("messages")))
    # a Claude Code session is asked through its transcript, with no hook on each step
    transcript = os.path.join(work, "session.jsonl")
    stamp = lambda: time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())
    with open(transcript, "w") as f:
        f.write(json.dumps({"type": "user", "uuid": "11111111-aaaa-4bbb-8ccc-000000000001", "timestamp": stamp(), "origin": {"kind": "human"}, "message": {"role": "user", "content": "run the payments tests, and tell me which ones are slow"}}) + "\n")
        f.write(json.dumps({"type": "user", "uuid": "11111111-aaaa-4bbb-8ccc-000000000002", "timestamp": stamp(), "origin": {"kind": "peer", "from": "uds:/tmp/x.sock"}, "message": {"role": "user", "content": "Message from Charles, sent through Airband: passed on by the tower"}}) + "\n")
        f.write(json.dumps({"type": "assistant", "effort": "medium", "message": {"model": "claude-opus-4-7", "content": [{"type": "tool_use", "name": "Read", "input": {"file_path": "/x/orders/webhook.ts"}}]}}) + "\n")
        f.write(json.dumps({"type": "assistant", "effort": "high", "message": {"model": "claude-opus-4-7", "content": [{"type": "tool_use", "name": "Bash", "input": {"command": "npm test -- payments", "description": "Run the payments tests"}}]}}) + "\n")
    def session(ident, project, event, title=None, record=None):
        agent = {"id": ident, "harness": "claude-code", "cwd": os.path.join(projects, project), "meta": {"transcript": record or transcript}}
        if title: agent["title"] = title
        return http("POST", "/v1/agent/events", {"schema": "airband.agent/1", "agent": agent, "event": event}, token=OWN)
    session("tr", "orders", {"type": "working"}, "orders webhook")
    for _ in range(12):
        asked = roster(feed()).get("orders webhook", {})
        if asked.get("doing") and asked.get("model"): break
        time.sleep(0.5)
    check("a Claude Code session's latest step is read from the end of its transcript", asked.get("doing") == "Run the payments tests", asked)
    check("and the model and thinking level it runs on", asked.get("model") == "Opus 4.7" and asked.get("effort") == "high", asked)

    print("What he types at an agent's own keyboard")
    his = lambda has: [m for m in feed()["messages"] if m["from"] == "you" and has in m["text"]]
    typed = his("payments tests")
    check("it is in the chat, in his words, as said to that agent", len(typed) == 1 and typed[0]["text"] == "run the payments tests, and tell me which ones are slow" and typed[0]["to"] == ["orders webhook"], typed)
    check("what the tower itself passed on is not taken for something he typed", not [m for m in feed()["messages"] if "passed on by the tower" in m["text"]])
    with open(transcript, "a") as f:
        f.write(json.dumps({"type": "attachment", "uuid": "11111111-aaaa-4bbb-8ccc-000000000003", "attachment": {"type": "queued_command", "prompt": [{"type": "text", "text": "also check the refund webhook"}], "source_uuid": "11111111-aaaa-4bbb-8ccc-0000000000q3", "origin": {"kind": "human"}, "timestamp": stamp()}}) + "\n")
    session("tr", "orders", {"type": "working"}, "orders webhook")
    session("tr", "orders", {"type": "working"}, "orders webhook")
    time.sleep(2.5)
    check("something he sends while it is working is added too, and nothing is added twice", len(his("refund webhook")) == 1 and len(his("payments tests")) == 1, (his("refund webhook"), his("payments tests")))

    print("Teams")
    moved = http("POST", "/v1/radio/team", {"agent": "tr", "team": "Payouts"})[1]
    check("he can move an agent to another team, and a name he gave it goes with it", moved == {"callsign": "payouts webhook", "team": "payouts"} and roster(feed())["payouts webhook"]["group"] == "payouts", moved)
    check("what he said to it is now shown as said to it under its new name", his("payments tests")[0]["to"] == ["payouts webhook"], his("payments tests"))
    moved = http("POST", "/v1/radio/team", {"agent": "tr", "team": "returns"})[1]
    f = feed()
    check("a team that was not there is made by moving an agent into it, with a colour of its own", moved.get("callsign") == "returns webhook" and "returns" in f["colours"], (moved, f["colours"]))
    session("tr", "orders", {"type": "working"}, "orders hooks")
    check("renamed afterwards, it stays in the team he put it in", "returns hooks" in roster(feed()), list(roster(feed())))
    session("nb", "orders", {"type": "working"})
    was = [c for c, a in roster(feed()).items() if a["id"] == "nb"]
    moved = http("POST", "/v1/radio/team", {"agent": "nb", "team": "catalog"})[1]
    check("one known only by a number takes the next free number in its new team", was == ["orders one"] and moved.get("callsign") == "catalog one", (was, moved))
    check("a team with no name, or an agent that has gone, is refused in words", "error" in http("POST", "/v1/radio/team", {"agent": "nb", "team": "  "})[1] and "error" in http("POST", "/v1/radio/team", {"agent": "nobody", "team": "catalog"})[1])
    session("nb", "orders", {"type": "signoff"})
    session("tr", "orders", {"type": "signoff"})
    # opened in the catalog folder, but every step it takes is in the logistics project
    stock, there = os.path.join(work, "stock.jsonl"), os.path.join(projects, "logistics")
    with open(stock, "w") as f:
        for tool, given in [("Read", {"file_path": f"{there}/src/rates.ts"}), ("Edit", {"file_path": f"{there}/src/rates.ts"}), ("Edit", {"file_path": f"{there}/src/zones.ts"}),
                            ("Bash", {"command": f"cd {there} && npm test -- rates", "description": "Run the rate tests"}), ("Edit", {"file_path": f"{there}/test/rates.test.ts"})]:
            f.write(json.dumps({"type": "assistant", "effort": "high", "message": {"model": "claude-opus-4-7", "content": [{"type": "tool_use", "name": tool, "input": given}]}}) + "\n")
    session("st", "catalog", {"type": "working"}, record=stock)
    session("st", "catalog", {"type": "said", "text": "The carrier rate table now reads the new zone prices, and the rate tests pass."}, record=stock)
    placed = {}
    for _ in range(60):
        placed = next((a for a in roster(feed()).values() if a["id"] == "st"), {})
        if placed.get("group") == "logistics": break
        time.sleep(1)
    check("an agent opened in one project and working in another is moved to that team by the tower (model)", placed.get("group") == "logistics", placed)
    check("and the chat says what it is called now", [m for m in feed()["messages"] if "is now logistics" in m["text"]], [m["text"] for m in feed()["messages"]][-4:])
    http("POST", "/v1/radio/team", {"agent": "st", "team": "catalog"})
    for _ in range(2):
        session("st", "catalog", {"type": "working"}, record=stock)
        session("st", "catalog", {"type": "said", "text": "The zone prices for the two new carriers are in as well."}, record=stock)
    time.sleep(12)
    check("one he has placed himself stays where he put it", next((a for a in roster(feed()).values() if a["id"] == "st"), {}).get("group") == "catalog")
    session("st", "catalog", {"type": "signoff"})
    report("import", "catalog", "catalog import", "said", "Imported 1,240 parts from the new supplier. All tests pass.")
    report("rates", "logistics", "logistics rates", "said", "Two carriers are updated. I'll do the third one next.")
    report("refunds", "orders", "orders refunds", "said", "The duplicate charge bug is fixed. Refunds and an apology email for 14 double-charged customers are ready, and I would send them now. Send them, or show you first?")
    report("payouts", "payouts", "payouts schedule", "said", "The weekly payout summary email for sellers is built and checked. Preview: https://claude.ai/artifact/abc123 Should it go out on Mondays or Fridays?")
    report("import", "catalog", "catalog import", "working")
    for _ in range(3):
        http("POST", "/v1/agent/events", {"schema": "airband.agent/1", "agent": {"id": "import", "harness": "claude-code"}, "event": {"type": "subagent", "what": "map columns"}}, token=OWN)
    f = feed()
    who = roster(f)
    check("sessions are called what he named them", set(who) == {"catalog part page", "catalog import", "logistics rates", "orders refunds", "payouts schedule", "letters seller notice"}, sorted(who))
    check("two agents share the catalog team", who["catalog part page"]["group"] == who["catalog import"]["group"] == "catalog")
    q = message(f, "orders refunds")
    check("a question is found, with its answers as buttons (model)", q and q.get("kind") == "needs-you" and len(q.get("options", [])) >= 2 and q["open"], q)
    check("the answer the agent itself recommended is marked (model)", q and q.get("recommended") == 0, q and q.get("recommended"))
    check("a long message that reports and then asks is cut to its news and, last, the question (model)", q and len(q["text"]) < 330 and q["long"] and "fix" in q["text"].lower() and q["text"].rstrip().endswith("?"), q and (len(q["text"]), q["long"], q["text"]))
    s = message(f, "logistics rates")
    check("an agent that stopped part way is seen as stalled (model)", s and s.get("kind") == "stalled" and who["logistics rates"]["state"] == "stalled", s and s.get("kind"))
    check("states: working, asking, idle", (who["catalog import"]["state"], who["orders refunds"]["state"], who["catalog part page"]["state"]) == ("working", "asking", "idle"),
          [(k, v["state"]) for k, v in who.items()])
    check("subagents are counted", who["catalog import"]["subagents"] == 3)
    p = message(f, "catalog part page")
    pic = p and p.get("pictures")
    check("a picture the agent sent comes with its message", bool(pic), p)
    if pic:
        code, part = http("GET", f"/v1/radio/picture/{pic[0]['id']}?from=0")
        import base64
        check("the picture can be fetched, as a JPEG", code == 200 and base64.b64decode(part["data"])[:2] == b"\xff\xd8")
    a = message(f, "payouts schedule")
    check("a link the agent wrote is kept beside the short text", a and a.get("links") == ["https://claude.ai/artifact/abc123"], a and a.get("links"))
    check("the tower offers the quick messages, his own last", [x["label"] for x in f["quick"]][-1] == "Make no mistakes" and len(f["quick"]) == 7)

    print("What a working agent is doing")
    report("import", "catalog", "catalog import", "doing", "Mapping the Northway columns")
    seen = feed()
    check("a harness can say what its agent is doing, and the roster carries it", roster(seen)["catalog import"].get("doing") == "Mapping the Northway columns" and "doing" not in roster(seen)["catalog part page"], roster(seen)["catalog import"])
    started = time.time()
    threading.Timer(1.0, lambda: report("import", "catalog", "catalog import", "doing", "Running the import tests")).start()
    later = http("GET", f"/v1/radio/feed?since={seen['rev']}&roster={seen['roster_rev']}&wait=20")[1]
    check("a radio waiting on the feed hears of it at once, with no message changed", time.time() - started < 6 and roster(later)["catalog import"].get("doing") == "Running the import tests" and not later["messages"], (time.time() - started, later.get("messages")))
    # a Claude Code session is asked through its transcript, with no hook on each step
    transcript = os.path.join(work, "session.jsonl")
    with open(transcript, "w") as f:
        f.write(json.dumps({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "input": {"file_path": "/x/orders/webhook.ts"}}]}}) + "\n")
        f.write(json.dumps({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Bash", "input": {"command": "npm test -- payments", "description": "Run the payments tests"}}]}}) + "\n")
    http("POST", "/v1/agent/events", {"schema": "airband.agent/1", "agent": {"id": "tr-again", "harness": "claude-code", "cwd": os.path.join(projects, "orders"), "title": "orders webhook", "meta": {"transcript": transcript}}, "event": {"type": "working"}}, token=OWN)
    for _ in range(12):
        asked = roster(feed()).get("orders webhook", {})
        if asked.get("doing"): break
        time.sleep(0.5)
    check("a Claude Code session's latest step is read from the end of its transcript", asked.get("doing") == "Run the payments tests", asked)
    http("POST", "/v1/agent/events", {"schema": "airband.agent/1", "agent": {"id": "tr-again", "harness": "claude-code"}, "event": {"type": "signoff"}}, token=OWN)
    report("import", "catalog", "catalog import", "said", "Imported 1,240 parts from the new supplier. All tests pass.")
    check("once it has spoken it is no longer doing anything", "doing" not in roster(feed())["catalog import"])
    report("import", "catalog", "catalog import", "working")

    print("Answering")
    check("a tapped answer goes to the agent that asked", say("Send", re=q["id"]) == "Sent to orders refunds." and "Send" in received("refunds"))
    check("and the question is no longer open", q["id"] not in feed()["open"])
    check("words about one agent's work reach it, past its teammate (model)", say("make the photo on the part page bigger") == "Sent to catalog part page.", received("page")[-200:])
    check("a follow-up with nobody named goes to the same agent (model)", say("and keep the price next to it") == "Sent to catalog part page.")
    check("words about another team's work reach that team (model)", say("send the payout email on Fridays") == "Sent to payouts schedule.")
    check("an agent named by the two letters his phone shows for it gets it (model)", say("C I, run the import again with the new supplier file") == "Sent to catalog import.")
    check("a team named alone narrows it to that team, and the work decides which of its agents (model)", say("to catalog. how is the supplier import going, did all the parts come in") == "Sent to catalog import.")
    check("what an agent gets is his words with one line in front and nothing after", received("page").rstrip().endswith("and keep the price next to it") and "From Charles, on his phone through Airband." in received("page"), received("page")[-300:])
    # what his phone adds when he is paired with a second computer: the agents he can see that this tower does not have
    there = [{"callsign": "warehouse stock", "group": "warehouse", "said": "Counted the shelves in bay 4: 212 units, 9 short of what the system says.", "ago": 40}]
    told = said("recount bay 4 and tell me which units are missing", others=there)
    check("words about the work of an agent on another computer are handed back for that computer (model)", told.get("elsewhere") == "warehouse stock" and not told.get("say") and "bay 4" not in "".join(received(a) for a in ("page", "import", "rates", "refunds", "schedule")), told)
    told = said("make the photo on the part page a bit smaller again", others=there)
    check("and words about this computer's work still go to its own agent (model)", told.get("say") == "Sent to catalog part page." and not told.get("elsewhere"), told)
    unsure = said("hm, what about the other thing", others=there)
    check("when it cannot tell, it says who it might be for, to tap, and delivers nothing (model)", len(unsure.get("ask", [])) >= 2 and unsure["say"].startswith("Who is that for?"), unsure)
    say("@catalogpartpage never mind")  # so the words it was holding are not waiting for a name
    check("the computer has no voice and no ears: it makes no speech, hands out no listening pass and takes no recording", http("GET", "/v1/radio/speech/x")[0] == 404 and http("GET", "/v1/radio/ears")[0] == 404 and http("POST", "/v1/radio/hear", {"clip": "c", "data": "", "samples": 0, "last": True})[0] in (404, 405) and "listens" not in feed())
    asked = http("GET", f"/v1/radio/message/{q['id']}?form=spoken&part=asked")[1] or {}
    check("a question can be read out as it was asked: who asks, its short form, and its answers", asked.get("text", "").startswith("orders refunds. ") and all(o in asked["text"] for o in q["options"]) and "in your own words" in asked["text"], asked)
    check("a tag for a team reaches every agent in it", say("@catalog commit what you have") in ("Sent to catalog part page and catalog import.", "Sent to catalog import and catalog part page."))
    check("a quick message sends the whole instruction and shows its label", say("Send me a picture of the result.", label="Screenshot", re=p["id"]) == "Sent to catalog part page."
          and "picture of the result" in received("page") and message(feed(), "you", "Screenshot") is not None)

    print("Attaching a photo and a video")
    import base64 as b64
    def attach(path, name):
        raw, ident, at = open(path, "rb").read(), os.urandom(8).hex(), 0
        while True:
            piece = raw[at:at + 90_000]
            last = at + len(piece) >= len(raw)
            code, r = http("POST", "/v1/radio/attachment", {"id": ident, "name": name, "from": at, "data": b64.b64encode(piece).decode(), "done": last})
            assert code == 200 and "next" in (r or {}), r
            at = r["next"]
            if last: return ident
    photo = attach(shot, "../../Photo 1.png")
    check("a photo sent in parts is kept whole, under a safe name, in the tower's own folder", open(os.path.join(home, ".airband", "attachments", photo, "Photo-1.png"), "rb").read() == open(shot, "rb").read())
    check("with it, his words reach the agent along with where the file is", say("make the header look like this", re=p["id"], attachments=[photo]) == "Sent to catalog part page."
          and os.path.join(".airband", "attachments", photo, "Photo-1.png") in received("page") and "(an image" in received("page"), received("page")[-300:])
    mine = message(feed(), "you", "make the header look like this")
    check("and his message in the chat shows the photo, without the file path", mine and mine.get("pictures") and "attachments" not in mine["text"], mine)
    check("a part that skips ahead is refused", "error" in (http("POST", "/v1/radio/attachment", {"id": os.urandom(8).hex(), "name": "x.png", "from": 500, "data": "", "done": False})[1] or {}))
    # told how long the file is, the tower takes its parts in any order: here the last first
    raw, ident = os.urandom(200_000), os.urandom(8).hex()
    pieces = [(at, raw[at:at + 90_000]) for at in range(0, len(raw), 90_000)]
    answers = [http("POST", "/v1/radio/attachment", {"id": ident, "name": "blob.bin", "from": at, "data": b64.b64encode(piece).decode(), "done": at + len(piece) >= len(raw), "total": len(raw)})[1] for at, piece in reversed(pieces)]
    kept = os.path.join(home, ".airband", "attachments", ident, "blob.bin")
    check("a file's parts can arrive in any order and are put together whole", answers[0].get("any") and answers[-1].get("next") == len(raw) and open(kept, "rb").read() == raw, [a.get("next") for a in answers])
    if shutil.which("ffmpeg"):
        clip = os.path.join(work, "clip.mp4")
        subprocess.run(["ffmpeg", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=3:size=320x240:rate=10", "-pix_fmt", "yuv420p", clip], check=True)
        video = attach(clip, "clip.mp4")
        check("a video goes too, and with nothing said the agent is asked to look at it", say("", re=p["id"], attachments=[video]) == "Sent to catalog part page." and "(a video" in received("page") and "Look at what I attached." in received("page"), received("page")[-400:])
        check("with a frame from each second pulled out for the agent to look at", len(os.listdir(os.path.join(home, ".airband", "attachments", video, "frames"))) == 3 and "frames" in received("page"))

    print("Poke")
    check("a poke sets the stalled agent going, and only it", say("/poke") == "Sent to logistics rates." and "Keep going" in received("rates") and "Keep going" not in received("import"))
    check("with nobody stalled, a poke says so", say("/poke") == "Nobody has stopped.")

    print("A new idea, with no project named")
    check("it starts in the team it belongs to (model)", say("new idea: check carrier prices every night and switch to whichever is cheapest") == "Launching in logistics."
          and os.path.exists(os.path.join(projects, "logistics", "launched.txt")))
    check("an ordinary reply that starts with \"new\" is not taken for one", not say("new numbers look fine").startswith("Launching"))

    print("A new agent on a model he names, for something to look into")
    task = "investigate why refunds for double-charged customers sometimes fail, and what it would take to fix"
    check("\"with fable high, …\" starts one on that model at that effort, where it belongs (model)", say("Hey, with Fable high, " + task) == "Launching in orders with fable, high."
          and (lambda got: got[:4] == ["--model", "fable-id", "--effort", "high"] and "refunds" in got[4] and "fable" not in got[4].lower())(open(os.path.join(projects, "orders", "launched.txt")).read().split("\n")))
    check("however he puts it: \"can you get opus to look into …\" (model)", say("can you get opus to look into why the supplier import got so slow") == "Launching in catalog with opus."
          and open(os.path.join(projects, "catalog", "launched.txt")).read().split("\n")[:2] == ["--model", "opus-id"])
    check("and \"have fable on medium compare …\" (model)", say("have fable on medium compare our two carriers on cost per parcel") == "Launching in logistics with fable, medium.")
    check("a model the launcher does not offer is refused in words, and nothing is started on another (model)", (lambda r: not r.startswith("Launching") and "gemini" in r.lower())(say("spin up a new agent on gemini to tidy the payout schedule docs")))
    check("a remark that opens with a model is not taken for one", not say("with opus it is too slow").startswith("Launching"))
    report("looker", "orders", "orders refund failures", "said", """I looked into why refunds for double-charged customers sometimes fail.

## What I found
- 212 refunds were attempted in the last 30 days and 19 failed (9%).
- 17 of the 19 failures are the same cause: the refund is sent to the payment provider with the id of the duplicate charge's *authorisation*, not its *capture*. That works when the two ids happen to match (single-capture orders) and fails with `charge_not_refundable` when the order was captured in two parts (split shipments).
- The other 2 failures were cards that had expired; the provider rejects those and there is nothing to fix on our side, but we never told the customer.
- Nothing retries a failed refund, and the order page still shows "refunded", so support only hears about it when the customer complains. Median time to notice: 6 days.

## What it would take
1. Use the capture id when refunding (`orders/refunds.ts`, about 15 lines, plus a test with a split shipment). Half a day.
2. Re-run the 17 stuck refunds once that ships: a one-off script, 1 hour. Total owed to customers: $1,284.
3. Show "refund failed" on the order page and email the customer when a card has expired: 1 day.

I have changed nothing. I would do 1 and 2 now and leave 3 for you to decide on, since it changes what customers see.""")
    brief = message(feed(), "orders refund failures")
    n = len(brief["text"].split()) if brief else 0
    check("what it finds comes back as an executive summary, longer than a headline and far shorter than the report (model)", brief and brief.get("kind") in ("report", "needs-you") and 28 <= n <= 100, brief and brief["text"])
    print("        it said: " + (brief["text"] if brief else "nothing"))
    report("refunds", "orders", "orders refunds", "said", """Here is the email I would send to the 14 customers. Nothing has gone out yet.

Subject: We charged you twice, and we've refunded it

Hi {first_name},

On Tuesday a fault on our side charged your card twice for order {order_number}. That was our mistake. We refunded the second charge of {amount} today; it should reach your account in 3 to 5 business days.

You don't need to do anything. If the refund hasn't arrived by next Friday, reply to this email and we'll chase it with your bank.

Sorry for the trouble,
The Parts Team

Send it as written, or do you want changes?""")
    letter = message(feed(), "orders refunds", "")
    check("a message that holds something to read whole is offered by name (model)", letter and "email" in (letter.get("read") or "").lower() and letter.get("long"), letter and (letter.get("read"), letter["text"]))
    check("and a short report offers nothing to read (model)", not (message(feed(), "catalog import") or {}).get("read"))
    check("with the numbers that matter in it (model)", brief and any(x in brief["text"] for x in ("19", "17", "9%", "nine percent", "1,284")), brief and brief["text"])

    print("Permission")
    def ask_permission(agent, project, what, out):
        r = subprocess.run([BIN, "report", "--id", agent, "--harness", "script", "--cwd", os.path.join(projects, project), "--title", f"{project} deploy", "permission", what],
                           env=env, capture_output=True, text=True, timeout=60)
        out.append((r.returncode, r.stdout.strip()))
    def pending_card(who_):
        for _ in range(40):
            m = message(feed(), who_, "Allow?")
            if m and m["open"]:
                return m
            time.sleep(0.25)
    out = []
    t = threading.Thread(target=ask_permission, args=("deploy", "orders", "run `npm run deploy`", out)); t.start()
    card = pending_card("orders deploy")
    check("the request is put to him word for word, as a yes or no", card and card["text"] == "Wants to run `npm run deploy`. Allow?" and card["options"] == ["Allow", "Deny"] and card.get("permission"), card)
    check("handing the decision back is refused", say("Do whatever you think is best.", re=card["id"]) == "That one needs a yes or a no from you.")
    check("Allow goes back to the harness as yes", say("Allow", re=card["id"]) == "Allowed." and (t.join(30) or out == [(0, "")]), out)
    before = len(feed()["messages"])
    check("a second tap on a request that is over allows nothing and reaches nobody", say("Allow", re=card["id"]) == "That request is no longer waiting. Nothing was allowed." and len(feed()["messages"]) == before)
    out = []
    t = threading.Thread(target=ask_permission, args=("deploy", "orders", "run `./migrate --env production`", out)); t.start()
    card = pending_card("orders deploy")
    check("his own words refuse it and are passed on as the reason", card and say("run it on staging first", re=card["id"]) == "Refused." and (t.join(30) or out == [(1, "run it on staging first")]), out)
    # a harness's own tool for asking him something comes through the same door, and is put to him as the question it is
    asked = http("POST", "/v1/agent/permission", {"schema": "airband.agent/1", "agent": {"id": "deploy", "harness": "claude-code", "cwd": os.path.join(projects, "orders"), "title": "orders deploy"},
                 "what": "use AskUserQuestion", "detail": "AskUserQuestion\n{…}", "question": {"text": "Which disk should I clear to make room?", "choices": ["The NAS", "The internal disk"]}}, token=OWN)[1]
    card = next((m for _ in range(40) for m in [message(feed(), "orders deploy", "Which disk")] if m and m["open"] or time.sleep(0.25)), None)
    check("a question an agent asks through its harness is shown as that question, with its own answers", card and card["text"] == "Which disk should I clear to make room?" and card["options"] == ["The NAS", "The internal disk"] and not card.get("permission"), card)
    check("and his answer goes back to it as the answer", say("The NAS", re=card["id"]) == "Sent to it." and http("GET", f"/v1/agent/permission/{asked['id']}?wait=5", token=OWN)[1].get("message") == "The NAS")
    out = []
    t = threading.Thread(target=ask_permission, args=("deploy", "orders", "run `rm -rf build`", out)); t.start()
    card = pending_card("orders deploy")
    plain = say("yes")
    still = pending_card("orders deploy")
    check("a \"yes\" that is not said to the request does not answer it", card and plain != "Allowed." and still is not None and out == [], (plain, out))
    check("said in reply to it, a plain yes in his own words allows it (model)", card and say("yeah, that's fine", re=card["id"]) == "Allowed." and (t.join(30) or out == [(0, "")]), out)
    out = []
    t = threading.Thread(target=ask_permission, args=("deploy", "orders", "run `sleep 1`", out)); t.start()
    card = pending_card("orders deploy")
    t.join(60)
    check("no answer is not a yes: the harness is told to ask some other way (exit 2)", out == [(2, "")], out)
    left = message(feed(), "orders deploy", "sleep 1")
    check("and the card stops offering an answer", left and not left.get("options") and not left.get("permission"), left)

    print("Names and identity")
    before = feed()["rev"]
    report("payouts", "payouts", "payouts friday email", "checkin")
    changed = feed(before)
    check("a renamed session's earlier messages come again under the new name", any(m["from"] == "payouts friday email" for m in changed["messages"]), [m["from"] for m in changed["messages"]])
    report("ask2", "orders", "orders two", "asked", "Ship the fix?")
    opened = [m for m in feed()["messages"] if m["from"] == "orders two"][-1]
    report("ask2", "orders", "orders two", "signoff")
    report("ask3", "orders", "orders two", "checkin")
    check("a question from an agent that has gone does not pass to whoever takes its name", opened["id"] not in feed()["open"])

    print("Access")
    check("a radio's token cannot report as an agent", http("POST", "/v1/agent/events", {"schema": "airband.agent/1"}, token=RADIO)[0] == 401)
    check("and cannot read the status page", http("GET", "/v1/status", token=RADIO)[0] == 401)

    if sys.platform == "darwin":
        print("Accounts")
        accounts_check()

    if "--claude" in sys.argv:
        print("A real Claude Code session asks permission")
        proj = os.path.join(projects, "realcheck")
        os.makedirs(os.path.join(proj, ".claude"))
        json.dump({"hooks": {"PermissionRequest": [{"hooks": [{"type": "command", "timeout": 120, "command": f"env HOME={home} AIRBAND_INCLUDE_HEADLESS=1 {BIN} hook claude-code"}]}]}},
                  open(os.path.join(proj, ".claude", "settings.json"), "w"))
        done = []
        def run_claude():
            r = subprocess.run(["claude", "-p", "--model", "haiku", "--permission-mode", "default",
                                "Use the Bash tool to run exactly this command and nothing else: touch made-by-claude.txt   Then reply with the single word done."],
                               cwd=proj, capture_output=True, text=True, timeout=180)
            done.append(r.stdout.strip()[-200:])
        t = threading.Thread(target=run_claude); t.start()
        card = None
        for _ in range(240):
            hits = [m for m in feed()["messages"] if m.get("permission") and m["open"] and "made-by-claude" in m["text"]]
            if hits:
                card = hits[-1]
                break
            time.sleep(0.5)
        check("Claude Code's permission prompt arrives as a question", card is not None, done)
        if card:
            check("it says exactly what will run", "touch made-by-claude.txt" in card["text"], card["text"])
            say("Allow", re=card["id"])
        t.join(180)
        check("after Allow from the radio, Claude ran it", os.path.exists(os.path.join(proj, "made-by-claude.txt")), done)
    print("An answer that comes late")
    os.makedirs(os.path.join(projects, "late"), exist_ok=True)
    report("late1", "late", "late", "checkin")
    report("late1", "late", "late", "said", "The export is ready. Should I also email it to the accountant?")
    def last_of(who, kind=None):
        for _ in range(120):
            hits = [m for m in feed()["messages"] if m["from"].startswith("late") and (kind is None or m.get("kind") == kind)]
            if hits and hits[-1].get("kind"): return hits[-1]
            time.sleep(0.5)
    q = last_of("late")
    say("Yes", re=q["id"])
    check("an answer to the last thing an agent said goes as he said it", "Yes" in received("late1") and "asked earlier" not in received("late1"), received("late1")[-300:])
    report("late1", "late", "late", "said", "Emailed. One more thing: the archive is 40 GB. Should I delete exports older than a year?")
    q2 = last_of("late")
    report("late1", "late", "late", "said", "Separately, I finished renaming the columns in the export and all the tests pass.")
    for _ in range(120):
        if [m for m in feed()["messages"] if m["from"].startswith("late") and "renam" in (m.get("text") or "").lower() and m.get("kind")]: break
        time.sleep(0.5)
    before = len(received("late1"))
    say("Yes", re=q2["id"])
    late = received("late1")[before:]
    check("an answer to a question the agent has since spoken past takes the question with it", "Yes" in late and "asked earlier" in late and ("delete" in late.lower() or "archive" in late.lower() or "exports" in late.lower()), late[-400:])

    print("Reading aloud")
    # a second radio turns reading aloud on; everything it is handed here it says it heard, until told otherwise
    def read_next(joined=False):
        return http("GET", "/v1/radio/next" + ("?joined=1" if joined else ""), radio="reader", timeout=40)
    def heard(t):
        http("POST", "/v1/radio/heard", {"id": t["id"]}, radio="reader")
        http("POST", "/v1/radio/ready", {}, radio="reader")
    def read_until(callsign, joined=False):
        for n in range(12):
            status, t = read_next(joined and n == 0)
            if status != 200: return None
            if t["callsign"] == callsign: return t
            heard(t)
    os.makedirs(os.path.join(projects, "solo"), exist_ok=True)
    report("solo", "solo", "solo run", "said", "The nightly export finished. 412 rows are in the warehouse table and the old file is archived.")
    report("solo", "solo", "solo run", "signoff")
    gone = read_until("solo run", joined=True)
    check("a report is read out even though its agent has since gone (model)", gone and "412" in gone["say"], gone)
    if gone: heard(gone)
    report("solo2", "solo", "solo second", "said", "The price list is rebuilt: 88 parts changed price and none failed.")
    silent = read_until("solo second")
    def trail():
        return http("GET", "/v1/status", token=OWN)[1]["aloud"]
    check("the next report is handed over in its turn (model)", silent and "88" in silent["say"], silent)
    if silent:
        http("POST", "/v1/radio/unheard", {"id": silent["id"], "why": "the test made no sound"}, radio="reader")
        waiting = http("GET", "/v1/status", token=OWN)[1]["pending"]
        check("a report that made no sound stays waiting to be read", any(p["callsign"] == "solo second" for p in waiting), waiting)
        check("and the tower writes down why", "could not read solo second aloud (the test made no sound)" in open(os.path.join(work, "tower.log")).read())
        check("its place in the queue is on record: handed over, then back to waiting", any("solo second" in l and "handed to reader → waiting" in l for l in trail()), trail())
        # another radio takes it, and the answer that carried it is lost on the way: asking again, saying what it
        # last got, gets the same one at once and not after two minutes
        def other(query):
            return http("GET", "/v1/radio/next?" + query, radio="reader2", timeout=40)
        lost = other("joined=1&have=")
        began = time.time()
        again = other("have=")
        check("a transmission whose answer was lost is handed over again when the radio says what it last got",
              lost[0] == 200 and again[0] == 200 and again[1]["id"] == lost[1]["id"] == silent["id"] and time.time() - began < 5, (lost, again))
        http("POST", "/v1/radio/heard", {"id": silent["id"]}, radio="reader2")
        http("POST", "/v1/radio/ready", {}, radio="reader2")
        status = http("GET", "/v1/status", token=OWN)[1]
        check("once heard it is out of the queue and on record as heard", not any(p["callsign"] == "solo second" for p in status["pending"]) and any("solo second" in l and "→ heard on reader2" in l for l in status["aloud"]), status["aloud"])
    # something newer from the same agent takes the place of what it said before, and that is written down too
    report("solo3", "solo", "solo third", "said", "The first batch of 40 invoices is filed.")
    report("solo3", "solo", "solo third", "said", "All 120 invoices are filed and the totals match the ledger.")
    check("an unheard report replaced by a newer one leaves the queue with its reason on record", any("solo third" in l and "→ dropped (superseded)" in l for l in trail()), trail())
    check("and the change is a line in the tower's log", "→ dropped (superseded)" in open(os.path.join(work, "tower.log")).read())
    first = [m for m in feed()["messages"] if "first batch" in (m.get("text") or "") + m["id"] or m.get("not_read")]
    check("the line he missed while he was listening says so in the chat, with the reason", any(m.get("not_read") == "superseded" for m in first), first)

    print("Saying something")
    # The tower answers at once that it has what he said. What came of it is told in the feed of the radio that
    # said it, under the id it was said with, and to anyone who asks again.
    before = received("solo3")
    mine = os.urandom(6).hex()
    rev = feed()["rev"]
    began = time.time()
    code, took = http("POST", "/v1/radio/utterance", {"text": "the invoices: file the March ones too", "id": mine})
    check("what he says is taken at once, before a model has read it", code == 200 and took == {"state": "taken"} and time.time() - began < 2, (code, took, time.time() - began))
    code, again = http("POST", "/v1/radio/utterance", {"text": "the invoices: file the March ones too", "id": mine})
    check("asked after while it is being answered, it is still only taken", again == {"state": "taken"}, again)
    told = None
    seen = rev
    for _ in range(100):
        f = http("GET", f"/v1/radio/feed?since={seen}&wait=5")[1]
        told = next((a for a in f.get("answers", []) if a["id"] == mine), None)
        if told: break
        seen = f.get("rev", seen)
    check("what came of it arrives in his radio's feed, under its id", told and told["state"] == "answered" and "say" in told["reply"], told)
    other = http("GET", f"/v1/radio/feed?since={rev}", radio="desk")[1]
    check("and in no other radio's", not other.get("answers"), other.get("answers"))
    code, later = http("POST", "/v1/radio/utterance", {"text": "the invoices: file the March ones too", "id": mine})
    check("asking again says the same, and nothing is delivered twice", later.get("state") == "answered" and told and later["reply"]["say"] == told["reply"]["say"] and received("solo3")[len(before):].count("March") <= 1, (later, received("solo3")[len(before):]))
    check("everything he says is one line in the tower's log, with its id and what became of it", f"utterance {mine} from e2e by this computer: answered in" in open(os.path.join(work, "tower.log")).read())
    check("something said with no id is refused", http("POST", "/v1/radio/utterance", {"text": "@solo3 thanks"})[0] in (400, 422))
    # a tower that stopped part way through answering: the id is in its database with no answer, and nothing in
    # this tower is answering it. It is taken up again, with no wait.
    import sqlite3
    stopped = os.urandom(6).hex()
    db = sqlite3.connect(os.path.join(home, ".airband", "airband.db"), timeout=10)
    db.execute("insert into receipts (id, at) values (?, ?)", (stopped, int(time.time() * 1000)))
    db.commit(); db.close()
    before = received("solo3")
    body = {"text": "@solo3 and again", "id": stopped}
    for _ in range(100):
        took = http("POST", "/v1/radio/utterance", body)[1]
        if took["state"] == "answered": break
        time.sleep(0.1)
    check("something an earlier tower took and never answered is answered by this one, once", took["state"] == "answered" and received("solo3")[len(before):].count("and again") == 1, (took, received("solo3")[len(before):]))

    print("Whether he is there")
    # a radio says where it stands; the tower does not work it out from what the radio asks for
    radios = lambda: http("GET", "/v1/status", token=OWN)[1]["radios"]
    check("a radio that says it is on a call is on one", "reader2" in radios(), radios())
    http("POST", "/v1/radio/presence", {"on_call": False, "showing_chat": False}, radio="reader2")
    STANDS["reader2"] = {"on_call": False, "showing_chat": False}
    check("and is off it the moment it says so, not a minute later", "reader2" not in radios(), radios())
    STANDS["quiet"] = {"on_call": False, "showing_chat": False}
    quiet = http("GET", "/v1/radio/next", radio="quiet", timeout=40)
    check("asking for something to read does not put a radio on a call that says it is not on one", quiet[0] == 204 and "quiet" not in radios(), (quiet, radios()))
finally:
    tower.terminate()
    tower.wait(10)

failed = [name for name, ok in results if not ok]
print(f"\n{len(results) - len(failed)} of {len(results)} passed." + ("" if not failed else "  Failed:\n  - " + "\n  - ".join(failed)))
if failed:
    print(f"The tower's log, with what the routing model was asked and said: {os.path.join(work, 'tower.log')}")
if not failed:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(1 if failed else 0)
