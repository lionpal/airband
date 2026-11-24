#!/usr/bin/env python3
"""End to end, for the commands and skills he picks from the phone: a real tower in a throwaway home, and a real
agent of each coding tool it holds, each in a throwaway project with one skill or command of its own that says to
answer with a mark the agent is told nowhere else.

    CARGO_TARGET_DIR=target/stand-ins tools/e2e-commands.py       a few minutes, a few small model calls

For each tool: the tower lists what the tool itself says it takes in that folder, the project's own one among
them, and picking it from the list (what the phone puts in the box, `/name ` and what he adds) runs it in the
agent, given what he added. For Claude Code also: its own commands are listed as its own, a skill added to the
project is listed on the next ask (its folder changed), and the skill asked for aloud ("slash greet …") is read by
the model as that skill and runs.

Nothing of his is read or written but what each tool needs to sign in, and nothing lands in his tools' histories:
every tool runs with this run's throwaway home as its home.
- Claude Code: a settings folder of this run's own (CLAUDE_CONFIG_DIR); his login is copied there as the file Claude
  Code reads when it has no keychain item of its own.
- Codex (through codex-acp): its home in the throwaway home (CODEX_HOME), his login (`auth.json`) copied there.
- OpenCode: its folders in the throwaway home (XDG_*), where the server `opencode acp` runs for itself keeps its
  sessions. It runs on a free model, signed in to nothing. OpenCode 2.3.2 runs a command sent this way but never
  says what came of it, so its project's command is checked to be listed as giving no answer, and is not sent.
- Antigravity CLI: on a Gemini API key in GEMINI_API_KEY (in the run's environment, never on a command line);
  without one its part is skipped, and said to be.
Copied logins are deleted at the end, whatever happened. Nothing here touches his tower or his phone. A tool that
is not installed is skipped, and said to be.
"""
import json, os, shutil, subprocess, sys, tempfile, time, urllib.request, urllib.error
import wire

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "airband")
PORT, OWN, RADIO = wire.free_port(), "commands-own", "commands-radio"
REAL_HOME = os.path.expanduser("~")
work = tempfile.mkdtemp(prefix="airband-commands-")
# as the tools name their folders: the temporary folder is a link on macOS
home, config_dir = os.path.join(os.path.realpath(work), "home"), os.path.join(work, "claude-config")
code = os.path.join(os.path.realpath(work), "code")
for d in (os.path.join(home, ".airband"), code):
    os.makedirs(d)
os.makedirs(config_dir, mode=0o700)
MARK = "HELLO-SKILL-RAN"
SAYS = f"Reply with exactly this line and nothing else, with the name you were given in place of NAME: {MARK} NAME\n"

def project(name, files):
    """A project of its own, under git so its top is where the tools stop looking up, with these files in it."""
    at = os.path.join(code, name)
    os.makedirs(at)
    subprocess.run(["git", "init", "-q", at], check=True)
    for path, text in files.items():
        os.makedirs(os.path.dirname(os.path.join(at, path)), exist_ok=True)
        open(os.path.join(at, path), "w").write(text)
    return at
SKILL = "---\nname: greet\ndescription: Greet someone in a fixed way\nargument-hint: <name>\n---\n" + SAYS
shop = project("shop", {".claude/skills/greet/SKILL.md": SKILL})
loft = project("loft", {".agents/skills/greet/SKILL.md": SKILL})
barn = project("barn", {".opencode/commands/greet.md": "---\ndescription: Greet someone in a fixed way\n---\n" + SAYS.replace("the name you were given", "this name: $ARGUMENTS. Put the name")})
mill = project("mill", {".agents/skills/greet/SKILL.md": SKILL})

# the model that reads what he says aloud: Claude Code as a command, with his own login, as tools/e2e.py has it
MODEL = {"kind": "command", "offer": "own", "argv": ["env", f"HOME={REAL_HOME}", "claude", "-p", "--model", "haiku", "--no-session-persistence", "--strict-mcp-config", "--setting-sources", "project",
                                     "--system-prompt", "You read what someone said. Reply with the JSON asked for and nothing else.", "--tools", ""]}
CONFIG = os.path.join(home, ".airband", "config.json")
with open(os.open(CONFIG, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
    json.dump({"port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "commands", "summarizer": MODEL,
               "projects": {"shop": shop, "loft": loft, "barn": barn, "mill": mill}, "launcher": "claude-held"}, f)

# Claude Code: its hooks, as airband puts them in his settings, report to this run's tower and nowhere else (a held
# agent says what it said through them)
hook = f"env HOME={home} {BIN} hook claude-code"
json.dump({"hooks": {event: [{"hooks": [{"type": "command", "command": hook, "timeout": 10}]}] for event in ("SessionStart", "Stop", "Notification", "UserPromptSubmit", "SessionEnd")}},
          open(os.path.join(config_dir, "settings.json"), "w"))
json.dump({"hasCompletedOnboarding": True, "projects": {shop: {"hasTrustDialogAccepted": True}}}, open(os.path.join(config_dir, ".claude.json"), "w"))
# Codex: its home in the throwaway home, the project trusted, thinking little
codex_home = os.path.join(home, ".codex")
os.makedirs(codex_home)
open(os.path.join(codex_home, "config.toml"), "w").write(f'model_reasoning_effort = "low"\n\n[projects."{loft}"]\ntrust_level = "trusted"\n')
# Antigravity CLI: on the key in the environment, the project trusted, its first-run questions answered
agy_state = os.path.join(home, ".gemini", "antigravity-cli")
os.makedirs(os.path.join(agy_state, "cache"))
json.dump({"modelProvider": "gemini", "trustedWorkspaces": [mill]}, open(os.path.join(agy_state, "settings.json"), "w"))
json.dump({"consumerOnboardingComplete": True, "enterpriseOnboardingComplete": True, "onboardingComplete": True}, open(os.path.join(agy_state, "cache", "onboarding.json"), "w"))

# the variables a Claude Code that runs this test sets for its own children, which would make an agent think it is
# one of them; and any of his own folders for the tools, so each reads only the throwaway home
env = {k: v for k, v in os.environ.items() if not (k.startswith("CLAUDE") or k == "CLAUDECODE" or k.startswith("XDG_") or k in ("CODEX_HOME", "AIRBAND_INTERNAL"))}
env.update(HOME=home, CLAUDE_CONFIG_DIR=config_dir, CODEX_HOME=codex_home,
           XDG_CONFIG_HOME=os.path.join(home, ".config"), XDG_DATA_HOME=os.path.join(home, ".local", "share"),
           XDG_STATE_HOME=os.path.join(home, ".local", "state"), XDG_CACHE_HOME=os.path.join(home, ".cache"),
           # codex-acp is fetched with npx: from his npm cache, which holds packages and nothing of his
           npm_config_cache=os.path.join(REAL_HOME, ".npm"))
if not os.environ.get("GEMINI_API_KEY"):
    env.pop("GEMINI_API_KEY", None)

results, skipped = [], []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def skip(name, why):
    skipped.append(name)
    print(f"  skip  {name}: {why}", flush=True)
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
    for _ in range(1800):
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
def held(harness):
    return next((a for a in feed()["roster"] if a.get("held") and a.get("harness") == harness), None)
def said_by(agent, mark):
    """The whole of what the agent wrote, for one of its messages since the start that holds `mark`."""
    for m in feed()["messages"]:
        if m.get("agent") == agent and not m.get("his"):
            full = http("GET", f"/v1/radio/message/{m['id']}?form=full")[1] or {}
            if mark in full.get("text", ""): return full["text"]
def listed(agent):
    code, got = http("GET", f"/v1/radio/commands?agent={agent}", timeout=120)
    return {c["name"]: c for c in (got or {}).get("items", [])} if code == 200 else {}

def start(tool, folder):
    """An agent of this tool the tower starts and holds, in this folder."""
    started = say("New agent in " + os.path.basename(folder), act={"kind": "launch", "team": folder, "tool": tool, **({"model": "haiku"} if tool == "claude-code" else {})})
    agent = until(lambda: held(tool), 180)
    check(f"{tool}: it is started and reports in, held", agent, (started, feed()["roster"]))
    return agent and agent["id"]

def picked(tool, folder, kind, runs="here"):
    """Its project's own one, listed by the tool and, when the tool says what came of it, picked from the list
    with a name added, runs."""
    me = start(tool, folder)
    if not me: return
    # an agent held through the Agent Client Protocol lists them once its session is open
    items = until(lambda: (lambda all: all if any(n.endswith("greet") for n in all) else None)(listed(me)), 90) or listed(me)
    greet = next((c for n, c in items.items() if n.endswith("greet")), {})
    check(f"{tool}: the project's own {kind} is listed as the tool lists it, with its kind and source, and " + ("runs from here" if runs == "here" else f"said to be {runs}"),
          greet.get("kind") == kind and greet.get("source") == "project" and greet.get("runs") == runs, (sorted(items), greet))
    if not greet or runs != "here": return
    # what the phone puts in the box when he picks it, and the name he adds
    reply = say(f"/{greet['name']} Ada", to=[me])
    ran = until(lambda: said_by(me, MARK), 240)
    check(f"{tool}: picked from the list, it runs in the agent, given what he added", ran and f"{MARK} Ada" in ran, (reply, ran, feed()["messages"][-3:]))

logins = []
tower = None
try:
    tower = subprocess.Popen([BIN, "run"], env=env, stdout=open(os.path.join(work, "tower.log"), "w"), stderr=subprocess.STDOUT)
    if not until(lambda: http("GET", "/v1/status", token=OWN)[0] == 200, 10):
        sys.exit("The tower did not start. Its log: " + os.path.join(work, "tower.log"))

    print("Claude Code, held, in a project with a skill of its own")
    login = os.path.join(config_dir, ".credentials.json")
    logins.append(login)
    fd = os.open(login, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        # never printed: it goes from the keychain to this file and nowhere else
        signed_in = subprocess.run(["security", "find-generic-password", "-s", "Claude Code-credentials", "-w"], stdout=f, stderr=subprocess.DEVNULL).returncode == 0
    me = start("claude-code", shop) if signed_in else skip("claude-code", "no Claude Code login in this Mac's keychain")
    if me:
        items = listed(me)
        greet = items.get("greet", {})
        check("claude-code: its list is what Claude Code says it takes there: the project's skill, with its kind, its source and its hint",
              greet.get("kind") == "skill" and greet.get("source") == "project" and greet.get("hint") == "<name>" and greet.get("runs") == "here", greet)
        compact = items.get("compact", {})
        check("claude-code: and its own commands, said to be its own, with no kind made up for them", compact.get("source") == "tool" and "kind" not in compact and compact.get("runs") == "here", compact)
        check("claude-code: none of his own skills is read: it has a settings folder of this run's own", not any(c.get("source") == "his" for c in items.values()), [n for n, c in items.items() if c.get("source") == "his"])
        reply = say("/greet Ada", to=[me])
        ran = until(lambda: said_by(me, MARK), 120)
        check("claude-code: picked from the list, the skill runs in the agent, given what he added", ran and f"{MARK} Ada" in ran, (reply, ran, feed()["messages"][-3:]))
        os.makedirs(os.path.join(shop, ".claude", "skills", "wave"))
        open(os.path.join(shop, ".claude", "skills", "wave", "SKILL.md"), "w").write("---\nname: wave\ndescription: Wave at someone\n---\nWave.\n")
        again = set(listed(me))
        check("claude-code: a skill added to the project is listed on the next ask: its folder changed", "wave" in again and "greet" in again, sorted(again))
        reply = say("slash greet Bea", to=[me], spoken=True)
        ran = until(lambda: said_by(me, f"{MARK} Bea"), 150)
        check("claude-code: asked for aloud, the model reads which skill he asked for and it runs, given what he said (model)", ran, (reply, feed()["messages"][-3:]))

    print("Codex, held through codex-acp, in a project with a skill of its own")
    auth = os.path.join(REAL_HOME, ".codex", "auth.json")
    if not shutil.which("codex") or not shutil.which("npx"):
        skip("codex", "Codex or npx is not installed")
    elif not os.path.exists(auth):
        skip("codex", "Codex is not signed in on this Mac")
    else:
        copy = os.path.join(codex_home, "auth.json")
        logins.append(copy)
        fd = os.open(copy, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as f, open(auth, "rb") as src:
            f.write(src.read())
        picked("codex", loft, "skill")

    print("OpenCode, held, in a project with a command of its own")
    if not shutil.which("opencode"):
        skip("opencode", "OpenCode is not installed")
    else:
        # OpenCode 2.3.2 runs it, but its protocol server ends the turn before it has and never says what
        # came of it: listed, and said to give no answer (its row, `held_commands`)
        picked("opencode", barn, "command", runs="no-answer")

    print("Antigravity CLI, held, in a project with a skill of its own")
    if not shutil.which("agy"):
        skip("antigravity-cli", "agy is not installed")
    elif not env.get("GEMINI_API_KEY"):
        skip("antigravity-cli", "no Gemini API key in GEMINI_API_KEY")
    else:
        picked("antigravity-cli", mill, "skill")
finally:
    if tower:
        try:
            for a in feed()["roster"]:
                if a.get("held"): http("POST", "/v1/radio/end", {"agent": a["id"]})
        except Exception:
            pass
        tower.terminate()
        tower.wait()
    for login in logins:
        if os.path.exists(login): os.remove(login)
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)} of {len(results)} passed" + (f", {len(skipped)} skipped" if skipped else ""))
    if failed:
        print("Kept for a look: " + work)
        # his logins are never kept, even then
        shutil.rmtree(config_dir, ignore_errors=True)
    else:
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if failed else 0)
