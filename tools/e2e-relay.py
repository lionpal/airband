#!/usr/bin/env python3
"""End to end through a relay: what he says while his computer is away is kept by the relay and reaches the
tower in order, once each, when it is back; and the relay takes nothing on a channel from anyone but its radio.

    cargo build --release && tools/e2e-relay.py          about a minute and a half, no model calls
    AIRBAND_E2E_DYNAMO=1 tools/e2e-relay.py              the same, with the relay keeping everything in DynamoDB:
                                                         three throwaway tables, made with the `aws` command as
                                                         it is set up here (AWS_REGION, default us-east-2) and
                                                         deleted at the end

A relay, a tower and a radio, all on this machine, in a throwaway folder. The radio is the terminal client with
a pairing link, which uses the same core as the phone.
"""
import base64, json, os, shutil, subprocess, sys, tempfile, time, urllib.parse, urllib.request, urllib.error
import wire

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BUILT = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release")
BIN, RELAY = os.path.join(BUILT, "airband"), os.path.join(BUILT, "airband-relay")
RELAY_PORT, TOWER_PORT = wire.free_port(), wire.free_port()
# With AIRBAND_E2E_RELAY (and AIRBAND_E2E_ADMISSION if it asks for a key) the same checks run against a relay
# that is already up, such as the live one, on a channel of their own that is closed at the end. What the relay
# keeps cannot be looked at from outside, so those checks are skipped.
LIVE = os.environ.get("AIRBAND_E2E_RELAY")
DYNAMO = None if LIVE or not os.environ.get("AIRBAND_E2E_DYNAMO") else "airband-e2e-" + os.urandom(4).hex()
REGION = os.environ.get("AWS_REGION", "us-east-2")
BASE = LIVE or f"http://127.0.0.1:{RELAY_PORT}"
work = tempfile.mkdtemp(prefix="airband-relay-e2e-")
home, client, data, project, got = (os.path.join(work, d) for d in ("tower", "phone", "relay", "shop", "got.txt"))
for d in (os.path.join(home, ".airband"), os.path.join(client, ".airband"), data, project):
    os.makedirs(d)
config = {"port": TOWER_PORT, "token": "own", "radio_token": "radio", "name": "Charles", "station": "e2e", "relay": LIVE or f"http://127.0.0.1:{RELAY_PORT}", "summarizer": {"kind": "none"},
          # through the relay only: on a computer whose own network the phone reaches, it would otherwise go straight
          # (the last check, with the relay gone, turns it on for itself)
          "lan": False}
if os.environ.get("AIRBAND_E2E_ADMISSION"):
    config["relay_admission"] = os.environ["AIRBAND_E2E_ADMISSION"]
json.dump(config, open(os.path.join(home, ".airband", "config.json"), "w"))
# a machine on a private network (100.64.0.0/10) would give the phone that address too, and it would go straight;
# the stand-ins build is told to leave those out, so the relay path is what is proved here
tower_env, phone_env = dict(os.environ, HOME=home, AIRBAND_STAND_IN_NO_PRIVATE_NETWORK="1"), dict(os.environ, HOME=client)
results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def start_tower():
    """Start the tower and go on once this run of it says it serves the pairing: a radio's call that comes before
    then is told at once that the computer is away. Only what this run wrote is read, never the last run's lines."""
    log = os.path.join(work, "tower.log")
    before = os.path.getsize(log) if os.path.exists(log) else 0
    t = subprocess.Popen([BIN, "run"], env=tower_env, stdout=open(log, "a"), stderr=subprocess.STDOUT)
    for _ in range(100):
        with open(log) as f:
            f.seek(before)
            if "serving" in f.read(): break
        time.sleep(0.1)
    return t
def send(text, timeout=120):
    r = subprocess.run([BIN, "send", text, "--keep-trying", "--link", link], env=phone_env, capture_output=True, text=True, timeout=timeout)
    return r.stdout.strip()
def outbox():
    """What the phone still holds of what he typed: [(words, stage)] (crates/airband-radio/src/store.rs)."""
    path = os.path.join(client, ".airband", "radio", "radio.json")
    return [(o["text"], o["stage"]) for o in json.load(open(path))["outbox"]] if os.path.exists(path) else []
def status():
    req = urllib.request.Request(f"http://127.0.0.1:{TOWER_PORT}/v1/status", headers={"authorization": "Bearer own"})
    with urllib.request.urlopen(req, timeout=10) as res: return json.loads(res.read())
def received():
    return open(got).read() if os.path.exists(got) else ""
def aws(*args):
    r = subprocess.run(["aws", *args, "--region", REGION, "--output", "json"], capture_output=True, text=True)
    if r.returncode: raise SystemExit(f"aws {args[0]} {args[1]} failed: {r.stderr.strip()[-300:]}")
    return json.loads(r.stdout) if r.stdout.strip() else {}
def held_items(channel, kind):
    return aws("dynamodb", "query", "--table-name", DYNAMO + "-held", "--consistent-read", "--key-condition-expression", "channel = :c AND begins_with(k, :p)",
               "--expression-attribute-values", json.dumps({":c": {"S": channel}, ":p": {"S": kind}}))["Items"]
def kept():
    """The ids of what the relay is keeping for the channel, in the order they came."""
    if DYNAMO: return [i["id"]["S"] for i in held_items(keys["channel"], "m#")]
    return [i["id"] for i in sorted(held_list(), key=lambda i: i["seq"])]
def held_list():
    """What the relay's folder says it keeps for the channel: its written list."""
    path = os.path.join(data, "held", keys["channel"], "list.json")
    return json.load(open(path))["items"] if os.path.exists(path) else []
def kept_bytes():
    if DYNAMO: return b"".join(base64.b64decode(i["sealed"]["B"]) for i in held_items(keys["channel"], "i#"))
    return b"".join(open(os.path.join(data, "held", keys["channel"], str(i["seq"])), "rb").read() for i in held_list())
def post(path, token=None, ident=None, channel=None, body=b"not something the tower's key opens", timeout=10):
    """A request to the relay as a radio makes it, by hand: with a token or without."""
    headers = {"content-type": "application/octet-stream"}
    if ident is not None: headers["x-airband-id"] = ident
    if token: headers["authorization"] = f"Bearer {token}"
    req = urllib.request.Request(f"{BASE}/v1/channels/{channel or keys['channel']}/{path}", data=body, method="POST", headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as res: return res.status, res.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
def leave(ident, token=None, channel=None):
    return post("queue", token, ident, channel)[0]

relay_env = dict(os.environ, PORT=str(RELAY_PORT), HOST="127.0.0.1", AIRBAND_RELAY_DATA=data)
if DYNAMO:
    print(f"Making three DynamoDB tables in {REGION}: {DYNAMO}-channels, -held, -waitlist")
    one = lambda name: [{"AttributeName": name, "AttributeType": "S"}]
    aws("dynamodb", "create-table", "--table-name", DYNAMO + "-channels", "--billing-mode", "PAY_PER_REQUEST", "--attribute-definitions", json.dumps(one("channel")), "--key-schema", '[{"AttributeName":"channel","KeyType":"HASH"}]', "--tags", "Key=app,Value=airband")
    aws("dynamodb", "create-table", "--table-name", DYNAMO + "-waitlist", "--billing-mode", "PAY_PER_REQUEST", "--attribute-definitions", json.dumps(one("email")), "--key-schema", '[{"AttributeName":"email","KeyType":"HASH"}]', "--tags", "Key=app,Value=airband")
    aws("dynamodb", "create-table", "--table-name", DYNAMO + "-held", "--billing-mode", "PAY_PER_REQUEST", "--attribute-definitions", json.dumps(one("channel") + one("k")), "--key-schema", '[{"AttributeName":"channel","KeyType":"HASH"},{"AttributeName":"k","KeyType":"RANGE"}]', "--tags", "Key=app,Value=airband")
    for name in ("-channels", "-waitlist", "-held"):
        aws("dynamodb", "wait", "table-exists", "--table-name", DYNAMO + name)
    aws("dynamodb", "update-time-to-live", "--table-name", DYNAMO + "-held", "--time-to-live-specification", "Enabled=true,AttributeName=expires")
    # the relay signs its own requests; here it is lent this machine's identity for the length of the test
    lent = subprocess.run(["aws", "configure", "export-credentials", "--format", "env-no-export"], capture_output=True, text=True).stdout
    relay_env.update(dict(line.split("=", 1) for line in lent.splitlines() if "=" in line), AIRBAND_RELAY_DYNAMO=DYNAMO, AWS_REGION=REGION)
relay = None if LIVE else subprocess.Popen([RELAY], env=relay_env, stdout=open(os.path.join(work, "relay.log"), "w"), stderr=subprocess.STDOUT)
time.sleep(3 if DYNAMO else 0.6)
tower = start_tower()
try:
    link = [w for w in subprocess.run([BIN, "pair", "test"], env=tower_env, capture_output=True, text=True).stdout.split() if w.startswith("airband:")][-1]
    keys = wire.keys_of(link)
    serving = wire.serving(link)
    stranger = wire.derive(os.urandom(32).hex())  # another pairing altogether, whose channel was never opened here
    # the tower picks a new pairing up on its next look at the settings (every ten seconds): go on once it says
    # it is serving it through the relay, not after a guess at how long that takes
    until = time.time() + 40
    while serving not in open(os.path.join(work, "tower.log")).read() and time.time() < until:
        time.sleep(0.5)
    check("the tower serves the new pairing through the relay", serving in open(os.path.join(work, "tower.log")).read(), open(os.path.join(work, "tower.log")).read()[-400:])
    check("and its log names the radio by id, not by the name he gave it", '"test"' not in open(os.path.join(work, "tower.log")).read(), open(os.path.join(work, "tower.log")).read()[-400:])
    subprocess.run([BIN, "report", "--id", "a1", "--harness", "script", "--cwd", project, "--deliver-command", json.dumps(["sh", "-c", f"cat>>{got}"]), "checkin"], env=tower_env)

    print("With the computer there")
    check("what he says goes through the relay to the agent", "Delivered." in send("the first thing") and "the first thing" in received(), received()[-200:])
    logged = lambda name: open(os.path.join(work, name)).read()
    check("the tower writes one line for it: how it came, what became of it and how long it took", " by the relay: answered in " in logged("tower.log"), logged("tower.log")[-400:])
    check("and the phone lets go of it once the tower has said what came of it", outbox() == [], outbox())

    print("Whether he is there")
    # a radio says where it stands inside each sealed request; the tower reads that, and nothing else
    check("a radio that only types is not on a call", "test" not in status()["radios"], status()["radios"])
    # what it says goes to a file of its own, so a radio that would not start says why in the check
    on_call = subprocess.Popen([BIN, "radio", "--link", link], env=phone_env, stdin=subprocess.DEVNULL, stdout=open(os.path.join(work, "radio.log"), "w"), stderr=subprocess.STDOUT)
    for _ in range(40):
        if "test" in status()["radios"]: break
        time.sleep(0.25)
    check("one that turns reading aloud on says so through the relay, and is on a call", "test" in status()["radios"], (status()["radios"], on_call.poll(), logged("radio.log")[-300:]))
    on_call.terminate(); on_call.wait(10)

    print("Only the channel's own radio")
    refused = post("queue", None, os.urandom(8).hex())
    check("a message without a token is refused", refused[0] == 401, refused)
    check("and one with a wrong token", post("queue", "0" * 64, os.urandom(8).hex()) == refused)
    check("and one with another channel's token", post("queue", stranger["radio"], os.urandom(8).hex()) == refused)
    check("a call without the token or with another channel's is refused too", post("call") == refused and post("call", stranger["radio"]) == refused)
    check("a channel that does not exist answers the same, with or without a token", post("queue", None, os.urandom(8).hex(), stranger["channel"]) == refused and post("call", keys["radio"], channel=stranger["channel"]) == refused)

    print("With the computer away")
    tower.terminate(); tower.wait(10)
    # no wait: the relay calls a tower gone once no pull of its is held open and none is coming back (its held
    # pull's connection went with it; or, where nothing tells the relay that, once a call waited its whole 40 s)
    said = send("the second thing")
    check("he is told his computer is away and that it will get it", "Your computer is away" in said, said)
    send("the third thing")
    check("the phone keeps both as with the relay, until the computer says what came of them", outbox() == [("the second thing", "Left"), ("the third thing", "Left")], outbox())
    if not LIVE: check("the relay keeps both, sealed, in the order they came", len(kept()) == 2 and len(kept_bytes()) > 100 and b"second" not in kept_bytes(), kept())
    check("and the agent has not had them yet", "second" not in received())
    if not LIVE:
        # the relay's own record of the call it could not carry: six characters of the channel, and nothing sealed
        lines = [l for l in logged("relay.log").splitlines() if f"call on {keys['channel'][:6]} answered 503" in l]
        check("the relay writes one line for a call it could not carry, with six characters of the channel and nothing that was said",
              lines and "tower pulling: no" in lines[-1] and keys["channel"] not in logged("relay.log") and keys["radio"] not in logged("relay.log") and "second" not in logged("relay.log"), logged("relay.log")[-400:])
    if not LIVE:
        # the same message left twice is kept once: the relay goes by the id the radio gave it
        first = kept()[0]
        check("leaving one again under the same id keeps it once", leave(first, keys["radio"]) == 202 and len(kept()) == 2, kept())
        check("something without an id is not kept", leave("", keys["radio"]) == 400)
        check("and something the tower cannot open is kept like the rest (the relay cannot tell)", leave("ab" * 8, keys["radio"]) == 202 and len(kept()) == 3)

    print("When the computer is back")
    tower = start_tower()
    for _ in range(100):
        if "third" in received() and (LIVE or not kept()): break
        time.sleep(0.3)
    text = received()
    check("the agent gets what he said, in the order he said it", 0 < text.find("the first thing") < text.find("the second thing") < text.find("the third thing"), text[-400:])
    check("each of them once", text.count("the second thing") == 1 and text.count("the third thing") == 1)
    check("and the tower's log says they came late, from what the relay kept", " by the relay, kept while the tower was away: answered in " in logged("tower.log"), logged("tower.log")[-600:])
    if not LIVE: check("the relay keeps nothing once it is handed over, the unreadable one included", not kept(), kept())
    check("and what he says now goes straight through again", "Delivered." in send("the fourth thing") and received().count("the fourth thing") == 1)
    check("with the computer's word on each, the phone holds none of them any more", outbox() == [], outbox())
    if not LIVE:
        print("Without the relay")
        # a radio that has heard where the tower is talks to it straight: the relay can go and the chat goes on.
        # Only a tower that listens on its own network says where it is, so for this one check the same tower
        # (same keys, same pairing) comes back with the local network on; everything above went by the relay.
        tower.terminate(); tower.wait(10)
        live = os.path.join(home, ".airband", "config.json")  # as the tower keeps it now, the pairing included
        json.dump(dict(json.load(open(live)), lan=True), open(live + ".new", "w")); os.replace(live + ".new", live)
        tower = start_tower()
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
    if DYNAMO:
        for name in ("-channels", "-waitlist", "-held"):
            subprocess.run(["aws", "dynamodb", "delete-table", "--table-name", DYNAMO + name, "--region", REGION], capture_output=True)
        print(f"Deleted the three tables {DYNAMO}-*")
failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)} of {len(results)} passed." + ("" if not failed else "  Failed:\n  - " + "\n  - ".join(failed) + f"\nLogs: {work}"))
if not failed:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(1 if failed else 0)
