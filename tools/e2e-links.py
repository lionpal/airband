#!/usr/bin/env python3
"""End to end, for the things an agent makes that live at a web address (a pull request, a published page).

    cargo build --release -p airband-cli && tools/e2e-links.py          a few seconds, no model, no network

A real tower in a throwaway home. A Claude Code session is played to it through `airband hook claude-code`,
with hook payloads and transcript lines in the shapes Claude Code really writes (taken from real sessions; see
the adapter's tests for the captured ones). Checked: a pull request the session opened and a page it published
are cards, from Claude Code's own records and nothing else; an address no tool said anything about is a plain
link by its site, however it reads; each is reported once; and it is all kept when the tower starts again.
"""
import json, os, shutil, subprocess, sys, tempfile, time, urllib.request, urllib.error
import wire

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "airband")
PORT = wire.free_port()
work = tempfile.mkdtemp(prefix="airband-links-")
home, shop, tools = os.path.join(work, "home"), os.path.join(work, "code", "shop"), os.path.join(work, "bin")
for d in (os.path.join(home, ".airband"), shop, tools):
    os.makedirs(d)
OWN, RADIO = "links-own-token", "links-radio-token"
json.dump({"port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "links"}, open(os.path.join(home, ".airband", "config.json"), "w"))
env = dict(os.environ, HOME=home)
env.pop("AIRBAND_INTERNAL", None)
results = []
def check(name, ok, detail=""):
    results.append(ok)
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def http(method, path, body=None):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {RADIO}", "content-type": "application/json", wire.SCHEMA_HEADER: wire.SCHEMA, "x-airband-radio": "links", "x-airband-presence": json.dumps({"on_call": False, "showing_chat": True})})
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
tower = subprocess.Popen([BIN, "run"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    check("the tower started", until(lambda: http("GET", "/v1/radio/feed?since=0") is not None, 20) is not None)

    pull, page, plain, unsaid = "https://github.com/acme/shop/pull/12", "https://claude.ai/artifact/linkscheck123", "https://docs.example.com/retries?x=1", "https://github.com/acme/shop/pull/99"
    transcript = os.path.join(work, "transcript.jsonl")
    base = {"session_id": "0a25b62f-4e7b-428f-a854-073b3dd9fe98", "cwd": shop, "transcript_path": transcript, "permission_mode": "default"}
    def wrote(entry):
        open(transcript, "a").write(json.dumps(entry) + "\n")
    def stop(words):
        hook({**base, "hook_event_name": "Stop", "stop_hook_active": False, "last_assistant_message": words})

    print("A Claude Code session opens a pull request and publishes a page")
    wrote({"type": "pr-link", "prNumber": 3, "prUrl": "https://github.com/acme/shop/pull/3", "prRepository": "acme/shop"})  # from before anyone was watching
    hook({**base, "hook_event_name": "SessionStart", "source": "startup"})
    hook({**base, "hook_event_name": "UserPromptSubmit", "prompt": "fix the duplicate charges"})
    # as Claude Code writes them: the pull request into its transcript (twice, as it does), the page as its tool's answer
    for _ in range(2):
        wrote({"type": "pr-link", "prNumber": 12, "prUrl": pull, "prRepository": "acme/shop", "sessionId": base["session_id"], "timestamp": "2026-11-22T00:09:45.224Z"})
    hook({**base, "hook_event_name": "PostToolUse", "tool_name": "Artifact", "tool_input": {"file_path": os.path.join(shop, "plan.html"), "icon": "test"},
          "tool_response": {"url": page, "path": os.path.join(shop, "plan.html"), "artifact_id": "1687a527-01d7-4833-9328-96aa99574dec", "title": "Refund plan", "updated": False, "owned": True, "audience": "owner", "seq": 1}})
    stop(f"The fix is up for review. Background: {plain}. See also {unsaid}. The plan is published.")
    cards = until(lambda: (lambda l: l if len(l) == 4 else None)(links_of("The fix is up")), 20) or links_of("The fix is up")
    check("the pull request it opened is a card, though it did not write the address out", (cards.get(pull, {}).get("name"), cards.get(pull, {}).get("title")) == ("Pull request", "acme/shop #12"), str(cards))
    check("the page it published is a card with the title its tool gave", (cards.get(page, {}).get("name"), cards.get(page, {}).get("title")) == ("Artifact", "Refund plan"), str(cards.get(page)))
    check("an address no tool said anything about is a plain link by its site", (cards.get(plain, {}).get("kind"), cards.get(plain, {}).get("host"), cards.get(plain, {}).get("title")) == (None, "docs.example.com", None), str(cards.get(plain)))
    check("so is one that reads like a pull request", (cards.get(unsaid, {}).get("kind"), cards.get(unsaid, {}).get("name"), cards.get(unsaid, {}).get("host")) == (None, None, "github.com"), str(cards.get(unsaid)))
    check("a pull request from before the tower was watching is not shown", "https://github.com/acme/shop/pull/3" not in cards)

    print("Each is reported once")
    hook({**base, "hook_event_name": "UserPromptSubmit", "prompt": "thanks"})
    stop("Glad to help with that.")
    until(lambda: any("Glad to help" in m.get("text", "") for m in (http("GET", "/v1/radio/feed?since=0") or {"messages": []})["messages"]), 20)
    check("the next thing it says has no cards", links_of("Glad to help") == {}, str(links_of("Glad to help")))
    hook({**base, "hook_event_name": "UserPromptSubmit", "prompt": "where is it"})
    stop(f"Again, it is at {pull} for you.")
    again = until(lambda: links_of("Again, it is at"), 10) or {}
    check("when it writes the address out later, that is the same card", again.get(pull, {}).get("title") == "acme/shop #12", str(again))

    print("Any tool can say what it made")
    report("b1", "made", "https://preview.example.com/build/7", "Checkout preview")
    report("b1", "said", "The preview is ready.")
    said = until(lambda: links_of("The preview is ready"), 10) or {}
    check("a thing reported with `airband report made` is shown by the title given", (said.get("https://preview.example.com/build/7", {}).get("title"), said.get("https://preview.example.com/build/7", {}).get("kind")) == ("Checkout preview", None), str(said))

    print("It is kept when the tower starts again")
    tower.terminate(); tower.wait(timeout=20)
    tower = subprocess.Popen([BIN, "run"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    kept = until(lambda: links_of("The fix is up"), 20) or {}
    check("the cards are as they were", (kept.get(page, {}).get("title"), kept.get(pull, {}).get("name")) == ("Refund plan", "Pull request"), str(kept))
finally:
    tower.terminate()
    if "--keep" not in sys.argv:
        shutil.rmtree(work, ignore_errors=True)
print(f"\n{sum(results)} of {len(results)} passed")
sys.exit(0 if all(results) else 1)
