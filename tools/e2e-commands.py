#!/usr/bin/env python3
"""End to end, for the commands and skills he picks from the phone: a real tower in a throwaway home, and a real
Claude Code agent it holds, signed in with his own login and nothing else of his.

    CARGO_TARGET_DIR=target/stand-ins tools/e2e-commands.py       a minute or two, a few small model calls

The agent works in a throwaway project with one skill of its own (`.claude/skills/greet`). Checks: the tower
lists what Claude Code itself says it takes there, the project's skill with its kind, where it comes from and its
hint, and Claude Code's own commands; picking the skill from the list (what the phone puts in the box, `/greet `
and what he adds) runs it in the agent; a skill added to the project is listed on the next ask, read again because
its folder changed; and the same skill asked for aloud ("slash greet …") is run too, read by the model.

Claude Code gets a settings folder of this run's own (CLAUDE_CONFIG_DIR), so none of his hooks, settings, skills or
history are read or written; his login is copied there as the file Claude Code reads when it has no keychain item
of its own, and the folder is deleted at the end, whatever happened. Nothing here touches his tower or his phone.
"""
import json, os, shutil, subprocess, sys, tempfile, time, urllib.request, urllib.error
import wire

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "airband")
PORT, OWN, RADIO = wire.free_port(), "commands-own", "commands-radio"
REAL_HOME = os.path.expanduser("~")
work = tempfile.mkdtemp(prefix="airband-commands-")
home, config_dir = os.path.join(work, "home"), os.path.join(work, "claude-config")
# as Claude Code names the folder: the temporary folder is a link on macOS
project = os.path.join(os.path.realpath(work), "code", "shop")
for d in (os.path.join(home, ".airband"), os.path.join(project, ".claude", "skills", "greet")):
    os.makedirs(d)
os.makedirs(config_dir, mode=0o700)
# a project with a skill of its own; git, so the project's top is where Claude Code and airband stop looking up
subprocess.run(["git", "init", "-q", project], check=True)
open(os.path.join(project, ".claude", "skills", "greet", "SKILL.md"), "w").write(
    "---\nname: greet\ndescription: Greet someone in a fixed way\nargument-hint: <name>\n---\n"
    "Reply with exactly this line and nothing else, with the name you were given in place of NAME: HELLO-SKILL-RAN NAME\n")
# the model that reads what he says aloud: Claude Code as a command, with his own login, as tools/e2e.py has it
MODEL = {"kind": "command", "offer": "own", "argv": ["env", f"HOME={REAL_HOME}", "claude", "-p", "--model", "haiku", "--no-session-persistence", "--strict-mcp-config", "--setting-sources", "project",
                                     "--system-prompt", "You read what someone said. Reply with the JSON asked for and nothing else.", "--tools", ""]}
json.dump({"port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "commands", "summarizer": MODEL,
           "projects": {"shop": project}, "launcher": "claude-held"}, open(os.path.join(home, ".airband", "config.json"), "w"))
# its hooks, as airband puts them in his settings, report to this run's tower and nowhere else: a held agent
# says what it said through them
hook = f"env HOME={home} {BIN} hook claude-code"
json.dump({"hooks": {event: [{"hooks": [{"type": "command", "command": hook, "timeout": 10}]}] for event in ("SessionStart", "Stop", "Notification", "UserPromptSubmit", "SessionEnd")}},
          open(os.path.join(config_dir, "settings.json"), "w"))
json.dump({"hasCompletedOnboarding": True, "projects": {project: {"hasTrustDialogAccepted": True}}}, open(os.path.join(config_dir, ".claude.json"), "w"))
# the variables a Claude Code that runs this test sets for its own children, which would make the agent think it
# is one of them
env = {k: v for k, v in os.environ.items() if not (k.startswith("CLAUDE") or k == "CLAUDECODE")}
env.update(HOME=home, CLAUDE_CONFIG_DIR=config_dir)

results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def http(method, path, body=None, token=RADIO, timeout=90):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json", wire.SCHEMA_HEADER: wire.SCHEMA, "x-airband-radio": "commands", "x-airband-presence": '{"on_call":false,"showing_chat":true}'})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
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
    for _ in range(1200):
        took = http("POST", "/v1/radio/utterance", body)[1]
        if took and took["state"] == "answered": return took["reply"]["say"]
        time.sleep(0.1)
def until(what, seconds=60):
    end = time.time() + seconds
    while time.time() < end:
        v = what()
        if v: return v
        time.sleep(0.5)
    return what()
feed = lambda: http("GET", "/v1/radio/feed?since=0")[1]
def held():
    return next((a for a in feed()["roster"] if a.get("held") and a.get("harness") == "claude-code"), None)
def said_by(agent, mark):
    """The whole of what the agent wrote, for its messages since the start, when one holds `mark`."""
    for m in feed()["messages"]:
        if m.get("agent") == agent and not m.get("his"):
            full = http("GET", f"/v1/radio/message/{m['id']}?form=full")[1] or {}
            if mark in full.get("text", ""): return full["text"]
def commands(agent):
    return http("GET", f"/v1/radio/commands?agent={agent}", timeout=120)

login = os.path.join(config_dir, ".credentials.json")
tower = None
try:
    fd = os.open(login, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        # never printed: it goes from the keychain to this file and nowhere else
        if subprocess.run(["security", "find-generic-password", "-s", "Claude Code-credentials", "-w"], stdout=f, stderr=subprocess.DEVNULL).returncode != 0:
            sys.exit("No Claude Code login in this Mac's keychain: sign in to Claude Code first.")
    tower = subprocess.Popen([BIN, "run"], env=env, stdout=open(os.path.join(work, "tower.log"), "w"), stderr=subprocess.STDOUT)
    if not until(lambda: http("GET", "/v1/status", token=OWN)[0] == 200, 10):
        sys.exit("The tower did not start. Its log: " + os.path.join(work, "tower.log"))

    print("A Claude Code agent the tower holds, in a project with a skill of its own")
    started = say("New agent in shop", act={"kind": "launch", "team": project, "tool": "claude-code", "model": "haiku"})
    agent = until(held, 90)
    check("it is started and reports in, held", agent, (started, feed()["roster"]))
    if not agent:
        sys.exit(1)
    me = agent["id"]

    code, listed = commands(me)
    items = {c["name"]: c for c in (listed or {}).get("items", [])}
    greet = items.get("greet", {})
    check("its list is what Claude Code says it takes there: the project's skill, with its kind, its source and its hint",
          code == 200 and greet.get("kind") == "skill" and greet.get("source") == "project" and greet.get("hint") == "<name>" and greet.get("runs") is True, (code, greet))
    compact = items.get("compact", {})
    check("and Claude Code's own commands, said to be its own, with no kind made up for them", compact.get("source") == "tool" and "kind" not in compact and compact.get("runs") is True, compact)
    check("none of his own skills is read: Claude Code has a settings folder of this run's own", not any(c.get("source") == "his" for c in items.values()), [n for n, c in items.items() if c.get("source") == "his"])

    print("Picked from the list, with what he adds")
    # what the phone puts in the box when he picks it, and the name he adds
    reply = say("/greet Ada", to=[me])
    ran = until(lambda: said_by(me, "HELLO-SKILL-RAN"), 120)
    check("the skill runs in the agent, given what he added", ran and "HELLO-SKILL-RAN Ada" in ran, (reply, ran, feed()["messages"][-3:]))

    print("A skill added to the project")
    os.makedirs(os.path.join(project, ".claude", "skills", "wave"))
    open(os.path.join(project, ".claude", "skills", "wave", "SKILL.md"), "w").write("---\nname: wave\ndescription: Wave at someone\n---\nWave.\n")
    again = {c["name"] for c in (commands(me)[1] or {}).get("items", [])}
    check("is listed on the next ask: its folder changed, so Claude Code is asked again", "wave" in again and "greet" in again, sorted(again))

    print("Asked for aloud")
    reply = say("slash greet Bea", to=[me], spoken=True)
    ran = until(lambda: said_by(me, "HELLO-SKILL-RAN Bea"), 150)
    check("the model reads which of its skills he asked for and the skill runs, given what he said (model)", ran, (reply, feed()["messages"][-3:]))
finally:
    if tower:
        try:
            agent = held()
            if agent: http("POST", "/v1/radio/end", {"agent": agent["id"]})
        except Exception:
            pass
        tower.terminate()
        tower.wait()
    # the held agent's own process, if the tower did not end it
    subprocess.run(["pkill", "-f", f"--cwd {project}"], stderr=subprocess.DEVNULL)
    if os.path.exists(login): os.remove(login)
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)} of {len(results)} passed")
    if failed:
        print("Kept for a look: " + work)
        # his login is never kept, even then
        shutil.rmtree(config_dir, ignore_errors=True)
    else:
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if failed else 0)
