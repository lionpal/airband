#!/usr/bin/env python3
"""End to end through a relay: what he says while his computer is away is kept by the relay and reaches the
tower in order, once each, when it is back.

    cargo build --release && tools/e2e-relay.py          about a minute and a half, no model calls

A relay, a tower and a radio, all on this machine, in a throwaway folder. The radio is the terminal client with
a pairing link, which uses the same core as the phone.
"""
import json, os, shutil, subprocess, sys, tempfile, time, urllib.request, urllib.error

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN, RELAY = os.path.join(ROOT, "target", "release", "airband"), os.path.join(ROOT, "target", "release", "airband-relay")
RELAY_PORT, TOWER_PORT = 7394, 7395
# With AIRBAND_E2E_RELAY (and AIRBAND_E2E_ADMISSION if it asks for a key) the same checks run against a relay
# that is already up, such as the live one, on a channel of their own that is closed at the end. What the relay
# keeps cannot be looked at from outside, so those checks are skipped.
LIVE = os.environ.get("AIRBAND_E2E_RELAY")
work = tempfile.mkdtemp(prefix="airband-relay-e2e-")
home, client, data, project, got = (os.path.join(work, d) for d in ("tower", "phone", "relay", "shop", "got.txt"))
for d in (os.path.join(home, ".airband"), os.path.join(client, ".airband"), data, project):
    os.makedirs(d)
config = {"port": TOWER_PORT, "token": "own", "radio_token": "radio", "name": "Charles", "station": "e2e", "relay": LIVE or f"http://127.0.0.1:{RELAY_PORT}", "summarizer": {"kind": "none"}}
if os.environ.get("AIRBAND_E2E_ADMISSION"):
    config["relay_admission"] = os.environ["AIRBAND_E2E_ADMISSION"]
json.dump(config, open(os.path.join(home, ".airband", "config.json"), "w"))
tower_env, phone_env = dict(os.environ, HOME=home), dict(os.environ, HOME=client)
results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def start_tower():
    t = subprocess.Popen([BIN, "run"], env=tower_env, stdout=open(os.path.join(work, "tower.log"), "a"), stderr=subprocess.STDOUT)
    for _ in range(100):
        if "serving" in open(os.path.join(work, "tower.log")).read().split("listening")[-1]: break
        time.sleep(0.1)
    return t
def send(text, timeout=120):
    r = subprocess.run([BIN, "send", text, "--keep-trying", "--link", link], env=phone_env, capture_output=True, text=True, timeout=timeout)
    return r.stdout.strip()
def received():
    return open(got).read() if os.path.exists(got) else ""
def kept():
    return sorted(f for _, _, fs in os.walk(os.path.join(data, "held")) for f in fs if not f.startswith("."))

relay = None if LIVE else subprocess.Popen([RELAY], env=dict(os.environ, PORT=str(RELAY_PORT), HOST="127.0.0.1", AIRBAND_RELAY_DATA=data), stdout=open(os.path.join(work, "relay.log"), "w"), stderr=subprocess.STDOUT)
time.sleep(0.6)
tower = start_tower()
try:
    link = [w for w in subprocess.run([BIN, "pair", "test"], env=tower_env, capture_output=True, text=True).stdout.split() if w.startswith("airband:")][-1]
    time.sleep(11)  # the tower picks a new pairing up within ten seconds
    subprocess.run([BIN, "report", "--id", "a1", "--harness", "script", "--cwd", project, "--deliver-command", f"sh -c cat>>{got}", "checkin"], env=tower_env)

    print("With the computer there")
    check("what he says goes through the relay to the agent", "Delivered." in send("the first thing") and "the first thing" in received(), received()[-200:])

    print("With the computer away")
    tower.terminate(); tower.wait(10)
    time.sleep(47)  # the relay calls a tower gone when it has not been heard from for 45 seconds
    said = send("the second thing")
    check("he is told his computer is away and that it will get it", "Your computer is away" in said, said)
    send("the third thing")
    if not LIVE: check("the relay keeps both, sealed, in the order they came", len(kept()) == 2 and b"second" not in b"".join(open(os.path.join(dp, f), "rb").read() for dp, _, fs in os.walk(os.path.join(data, "held")) for f in fs), kept())
    check("and the agent has not had them yet", "second" not in received())
    if not LIVE:
        # the same message left twice is kept once: the relay goes by the id the radio gave it
        channel = os.listdir(os.path.join(data, "held"))[0]
        def leave(ident, body=b"not something the tower's key opens"):
            req = urllib.request.Request(f"http://127.0.0.1:{RELAY_PORT}/v1/channels/{channel}/queue", data=body, method="POST", headers={"x-airband-id": ident, "content-type": "application/octet-stream"})
            try: return urllib.request.urlopen(req, timeout=10).status
            except urllib.error.HTTPError as e: return e.code
        first = kept()[0].split("-", 1)[1]
        check("leaving one again under the same id keeps it once", leave(first) == 202 and len(kept()) == 2, kept())
        check("something without an id is not kept", leave("") == 400)
        check("and something the tower cannot open is kept like the rest (the relay cannot tell)", leave("ab" * 8) == 202 and len(kept()) == 3)

    print("When the computer is back")
    tower = start_tower()
    for _ in range(100):
        if "third" in received() and (LIVE or not kept()): break
        time.sleep(0.3)
    text = received()
    check("the agent gets what he said, in the order he said it", 0 < text.find("the first thing") < text.find("the second thing") < text.find("the third thing"), text[-400:])
    check("each of them once", text.count("the second thing") == 1 and text.count("the third thing") == 1)
    if not LIVE: check("the relay keeps nothing once it is handed over, the unreadable one included", not kept(), kept())
    check("and what he says now goes straight through again", "Delivered." in send("the fourth thing") and received().count("the fourth thing") == 1)
    if not LIVE:
        print("Without the relay")
        # a radio that has heard where the tower is talks to it straight: the relay can go and the chat goes on
        say = lambda words: subprocess.run([BIN, "report", "--id", "a1", "--harness", "script", "--cwd", project, "said", words], env=tower_env)
        watch = subprocess.Popen([BIN, "feed", "--link", link], env=phone_env, stdout=open(os.path.join(work, "feed.out"), "w"), stderr=subprocess.STDOUT)
        seen = lambda words: words in open(os.path.join(work, "feed.out")).read()
        time.sleep(2)
        say("said with the relay still there")
        for _ in range(60):
            if seen("relay still there"): break
            time.sleep(0.25)
        time.sleep(1.5)
        relay.terminate(); relay.wait(10)
        say("said after the relay went")
        for _ in range(60):
            if seen("after the relay went"): break
            time.sleep(0.25)
        check("a radio on the tower's own network goes on hearing it with the relay gone", seen("relay still there") and seen("after the relay went") and watch.poll() is None, open(os.path.join(work, "feed.out")).read()[-300:])
        watch.terminate()
finally:
    if LIVE:
        subprocess.run([BIN, "unpair", "test"], env=tower_env, capture_output=True)  # closes the channel on the relay
    for p in (tower, relay):
        if p is None: continue
        p.terminate()
        try: p.wait(10)
        except Exception: p.kill()
failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)} of {len(results)} passed." + ("" if not failed else "  Failed:\n  - " + "\n  - ".join(failed) + f"\nLogs: {work}"))
if not failed:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(1 if failed else 0)
