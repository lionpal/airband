#!/usr/bin/env python3
"""End to end, for the things an agent makes that live at a web address (a pull request, an issue, an artifact).

    cargo build --release -p airband-cli && tools/e2e-links.py          about two minutes, no model, no network
    tools/e2e-links.py --real <owner>/<repo>                             the same against GitHub, signed in as you

A real tower in a throwaway home. By default `gh` is a stand-in on the tower's path that answers from a file,
so what the tower asks, and that a card follows the thing as it changes, are checked with nothing outside this
computer. With --real the tower asks the real `gh` about a pull request and an issue this script opens in the
repository named (one you can write to; use a private throwaway), then merges and closes them and watches the
cards follow. A published page is reported the way Claude Code's own hook reports one.
"""
import json, os, shutil, subprocess, sys, tempfile, time, urllib.request, urllib.error

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "airband")
PORT = 7396
REAL = sys.argv[sys.argv.index("--real") + 1] if "--real" in sys.argv else None
work = tempfile.mkdtemp(prefix="airband-links-")
home, shop, tools = os.path.join(work, "home"), os.path.join(work, "code", "shop"), os.path.join(work, "bin")
for d in (os.path.join(home, ".airband"), shop, tools):
    os.makedirs(d)
OWN, RADIO = "links-own-token", "links-radio-token"
json.dump({"port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "links"}, open(os.path.join(home, ".airband", "config.json"), "w"))
env = dict(os.environ, HOME=home)
env.pop("AIRBAND_INTERNAL", None)
ANSWERS, ASKED = os.path.join(work, "answers.json"), os.path.join(work, "asked.log")
if not REAL:
    # a stand-in for `gh`: it writes down what it was asked and answers from a file this script changes
    open(os.path.join(tools, "gh"), "w").write(f"""#!/usr/bin/env python3
import json, sys
open({ASKED!r}, "a").write(json.dumps(sys.argv[1:]) + "\\n")
answers = json.load(open({ANSWERS!r}))
said = answers.get(" ".join(sys.argv[1:4]))
if said is None: sys.exit(1)
print(json.dumps(said))
""")
    os.chmod(os.path.join(tools, "gh"), 0o755)
    env["PATH"] = tools + ":" + env["PATH"]
    json.dump({}, open(ANSWERS, "w"))

results = []
def check(name, ok, detail=""):
    results.append(ok)
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def http(method, path, body=None):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {RADIO}", "content-type": "application/json", "x-airband-radio": "links", "x-airband-presence": json.dumps({"on_call": False, "showing_chat": True})})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.loads(r.read() or b"null")
    except (urllib.error.HTTPError, urllib.error.URLError):
        return None
def report(agent, *words):
    return subprocess.run([BIN, "report", "--id", agent, "--harness", "claude-code", "--cwd", shop, *words], env=env, capture_output=True, text=True)
def hook(notification):
    return subprocess.run([BIN, "hook", "claude-code"], input=json.dumps(notification), env=dict(env, AIRBAND_INCLUDE_HEADLESS="1"), capture_output=True, text=True)
def links_of(has):
    f = http("GET", "/v1/radio/feed?since=0") or {"messages": []}
    hits = [m for m in f["messages"] if has in m.get("text", "")]
    return {l["address"]: l for l in hits[-1].get("links", [])} if hits else {}
def until(what, seconds=100):
    for _ in range(seconds * 2):
        got = what()
        if got: return got
        time.sleep(0.5)
    return None
def gh(*args):
    return subprocess.run(["gh", *args], capture_output=True, text=True, cwd=shop)

tower = subprocess.Popen([BIN, "run"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    check("the tower started", until(lambda: http("GET", "/v1/radio/feed?since=0") is not None, 20) is not None)

    if REAL:
        print(f"Against GitHub: {REAL}")
        subprocess.run(["gh", "repo", "clone", REAL, shop, "--", "-q"], capture_output=True)
        branch = f"links-{os.urandom(3).hex()}"
        for c in (["git", "checkout", "-q", "-b", branch], ["sh", "-c", f"echo {branch} >> README.md"], ["git", "-c", "user.name=airband", "-c", "user.email=links@airband.dev", "commit", "-qam", "A line for the links check"],
                  ["git", "-c", "credential.helper=!gh auth git-credential", "push", "-q", "-u", "origin", branch]):
            subprocess.run(c, cwd=shop, capture_output=True)
        pull = gh("pr", "create", "--title", "Stop the retry charging twice", "--body", "Made by tools/e2e-links.py.").stdout.strip().splitlines()[-1]
        issue = gh("issue", "create", "--title", "Refund emails go out twice", "--body", "Made by tools/e2e-links.py.").stdout.strip().splitlines()[-1]
        check("a pull request and an issue were opened", pull.startswith("https://github.com/") and "/issues/" in issue, f"{pull} {issue}")
    else:
        pull, issue = "https://github.com/acme/shop/pull/12", "https://github.com/acme/shop/issues/7"
        json.dump({"pr view 12": {"title": "Stop the retry charging twice", "state": "OPEN"}, "issue view 7": {"title": "Refund emails go out twice", "state": "OPEN"}}, open(ANSWERS, "w"))
    page = "https://claude.ai/artifact/linkscheck123"

    print("A message with a pull request, an issue, a plain link, and a page its tool published")
    # the page is reported as Claude Code's own hook reports the Artifact tool: its answer says where and what
    hook({"session_id": "a1", "cwd": shop, "hook_event_name": "PostToolUse", "tool_name": "Artifact", "tool_input": {"file_path": os.path.join(shop, "plan.html")},
          "tool_response": {"url": page, "title": "Refund plan", "seq": 1, "audience": "owner"}})
    report("a1", "said", f"The fix is up at {pull} and I filed {issue}. Background: https://docs.example.com/retries. The plan is published.")
    cards = until(lambda: (lambda l: l if len(l) == 4 else None)(links_of("The fix is up")), 20) or {}
    check("each address in the message is a card, and so is the page the agent did not write out", set(cards) == {pull, issue, "https://docs.example.com/retries", page}, str(list(cards)))
    check("the pull request and the issue are known by the shape of their addresses", (cards.get(pull, {}).get("name"), cards.get(issue, {}).get("name")) == ("Pull request", "Issue"), str(cards.get(pull)))
    check("the page is what its tool said it is, with the title the tool gave", (cards.get(page, {}).get("name"), cards.get(page, {}).get("title")) == ("Artifact", "Refund plan"), str(cards.get(page)))
    plain = cards.get("https://docs.example.com/retries", {})
    check("an address no row fits is a plain link called by its site", (plain.get("kind"), plain.get("label"), plain.get("title")) == (None, "docs.example.com", None), str(plain))

    print("What each is called and how it stands is asked on this computer")
    known = until(lambda: (lambda l: l if l.get(pull, {}).get("state") and l.get(issue, {}).get("state") else None)(links_of("The fix is up")), 60) or {}
    check("the pull request has its title and is open", (known.get(pull, {}).get("title"), known.get(pull, {}).get("state", {}).get("name")) == ("Stop the retry charging twice", "Open"), str(known.get(pull)))
    check("the issue has its title and is open", (known.get(issue, {}).get("title"), known.get(issue, {}).get("state", {}).get("name")) == ("Refund emails go out twice", "Open"), str(known.get(issue)))
    if not REAL:
        asked = [json.loads(l) for l in open(ASKED)]
        check("the program was given the parts of the address and nothing else of the message", ["pr", "view", "12", "--repo", "acme/shop"] in [a[:5] for a in asked] and not any("The fix" in " ".join(a) for a in asked), str(asked[:2]))
    check("nothing was asked about the page or the plain link, which no program knows", "state" not in known.get(page, {}) and "state" not in plain)

    print("A card follows the thing as it changes")
    if REAL:
        merged = gh("pr", "merge", pull, "--squash", "--delete-branch")
        gh("issue", "close", issue, "--reason", "not planned")
        check("the pull request was merged and the issue closed", merged.returncode == 0, merged.stderr[-200:])
    else:
        json.dump({"pr view 12": {"title": "Stop the retry charging twice", "state": "MERGED"}, "issue view 7": {"title": "Refund emails go out twice", "state": "NOT_PLANNED"}}, open(ANSWERS, "w"))
    after = until(lambda: (lambda l: l if l.get(pull, {}).get("state", {}).get("name") == "Merged" and l.get(issue, {}).get("state", {}).get("name") == "Closed" else None)(links_of("The fix is up")), 100) or {}
    check("the pull request's card says merged", after.get(pull, {}).get("state") == {"name": "Merged", "tone": "done"}, str(links_of("The fix is up").get(pull)))
    check("the issue's card says closed", after.get(issue, {}).get("state") == {"name": "Closed", "tone": "stopped"}, str(links_of("The fix is up").get(issue)))
    if not REAL:
        # merged will not change again: it is not asked about any more, while the issue (which can be reopened) is
        time.sleep(70)
        late = [json.loads(l) for l in open(ASKED)][-2:]
        check("a merged pull request is not asked about again", all(a[0] == "issue" for a in late), str(late))
        json.dump({}, open(ANSWERS, "w"))
        report("a2", "said", "Have a look at https://github.com/acme/shop/pull/99 when you can.")
        time.sleep(20)
        unknown = links_of("Have a look").get("https://github.com/acme/shop/pull/99", {})
        check("one the program cannot say anything about shows only what its address says", (unknown.get("name"), unknown.get("label"), unknown.get("title"), unknown.get("state")) == ("Pull request", "shop #99", None, None), str(unknown))

    print("It is kept when the tower starts again")
    tower.terminate(); tower.wait(timeout=20)
    tower = subprocess.Popen([BIN, "run"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    again = until(lambda: links_of("The fix is up"), 20) or {}
    check("the cards are as they were, the page with its title too", again.get(page, {}).get("title") == "Refund plan" and again.get(pull, {}).get("state", {}).get("name") == "Merged", str(again.get(page)))
finally:
    tower.terminate()
    if "--keep" not in sys.argv:
        shutil.rmtree(work, ignore_errors=True)
print(f"\n{sum(results)} of {len(results)} passed")
sys.exit(0 if all(results) else 1)
