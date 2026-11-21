#!/usr/bin/env python3
"""End to end, for the banner on the lock screen: a real tower, a throwaway home, and a stand-in for the relay.

    cargo build --release -p airband-cli && tools/e2e-banner.py        about two minutes, no model

A push service delivers a banner unsealed, so what matters here is what the tower hands the relay: three
numbers, the banner's address, and nothing else. Checked: that a banner is put up when the first agent starts
waiting and he is not there, that it is not put up twice, that its numbers follow the agents once the phone
has said where to change it, that it comes down when nobody is waiting, that a phone showing the chat is sent
nothing, that an address the push service says has gone is forgotten, and that none of the agents' names or words is in anything the relay is given.
"""
import json, os, shutil, subprocess, sys, tempfile, threading, time, urllib.request, urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "airband")
PORT, RELAY, OWN, RADIO = 7396, 7397, "banner-own", "banner-radio"
work = tempfile.mkdtemp(prefix="airband-banner-")
home, project = os.path.join(work, "home"), os.path.join(work, "code", "shop")
for d in (os.path.join(home, ".airband"), project):
    os.makedirs(d)
SECRET, START, UPDATE = os.urandom(32).hex(), os.urandom(32).hex(), os.urandom(32).hex()
json.dump({"port": PORT, "token": OWN, "radio_token": RADIO, "name": "Charles", "station": "e2e", "permission_wait": 30, "summarizer": {"kind": "none"},
           "relay": f"http://127.0.0.1:{RELAY}", "pairings": [{"name": "phone", "secret": SECRET}]}, open(os.path.join(home, ".airband", "config.json"), "w"))
env = dict(os.environ, HOME=home)

def derived(label, size=32):
    return HKDF(algorithm=hashes.SHA256(), length=size, salt=b"airband.relay/1", info=label.encode()).derive(bytes.fromhex(SECRET))
CHANNEL, PULL = derived("channel")[:16].hex(), derived("pull").hex()

asked, answer_with = [], [200]
class Relay(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *_): pass
    def reply(self, code, body=b""):
        self.send_response(code)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_PUT(self): self.reply(204)
    def do_GET(self):
        time.sleep(3)  # a pull is held open; nothing ever comes
        self.reply(204)
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        if self.path != f"/v1/channels/{CHANNEL}/banner":
            return self.reply(200 if self.path.endswith("/push") else 404, b'{"sent":true}')
        asked.append({"auth": self.headers.get("authorization"), "raw": body, **json.loads(body)})
        code = answer_with[0]
        self.reply(code, json.dumps({"sent": code == 200, "gone": code == 410, "reason": "Unregistered" if code == 410 else ""}).encode())
server = ThreadingHTTPServer(("127.0.0.1", RELAY), Relay)
threading.Thread(target=server.serve_forever, daemon=True).start()

results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def http(method, path, body=None, token=RADIO, radio="phone", timeout=40, showing=False):
    """`showing`: the radio says the chat is in front of him, as every request of a radio's says whether it is."""
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json", "x-airband-radio": radio,
                                          "x-airband-presence": json.dumps({"on_call": False, "showing_chat": showing})})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        return e.code, None
    except urllib.error.URLError:
        return 0, None
def report(*words, agent="a1"):
    return subprocess.run([BIN, "report", "--id", agent, "--harness", "script", "--cwd", project, *words], env=env, capture_output=True, text=True, timeout=60)
def changed(count, within=45):
    """Wait until the relay has been asked that many times in all."""
    until = time.time() + within
    while len(asked) < count and time.time() < until:
        time.sleep(0.2)
    return len(asked) >= count
def quiet(count, for_seconds=27):
    """Nothing more is asked in longer than the tower waits between two changes."""
    time.sleep(for_seconds)
    return len(asked) == count
def shown(i):
    return {k: v for k, v in asked[i].items() if k != "raw"} if len(asked) > i else None

tower = subprocess.Popen([BIN, "run"], env=env, stdout=open(os.path.join(work, "tower.log"), "w"), stderr=subprocess.STDOUT)
try:
    for _ in range(100):
        if http("GET", "/v1/status", token=OWN)[0] == 200:
            break
        time.sleep(0.1)
    else:
        sys.exit("The tower did not start. Its log: " + os.path.join(work, "tower.log"))

    print("The phone has given no address for a banner")
    report("asked", "Ship the first fix?")
    check("nothing is asked of the relay", quiet(0, 12))

    print("A second agent waits, and the phone says where a banner can be put up")
    report("asked", "Rewrite the two easy questions, or leave them?", agent="a2")
    check("the tower takes the address", http("POST", "/v1/radio/banner", {"provider": "apns", "which": "start", "token": START})[0] == 200)
    check("a banner is put up", changed(1))
    if asked:
        a = asked[0]
        check("as this channel's tower, at the address that puts one up, lighting the screen", a["auth"] == f"Bearer {PULL}" and a["to"] == START and a["event"] == "start" and a["urgent"] is True, shown(0))
        check("what it carries is the three numbers and nothing more", sorted(k for k in a if k not in ("auth", "raw")) == ["counts", "event", "provider", "to", "urgent"] and a["counts"] == {"waiting": 2, "working": 0, "unread": 0}, shown(0))
    check("and not a second one while the phone has not said where to change the first", quiet(1))

    print("The phone says where to change the banner that is up")
    http("POST", "/v1/radio/banner", {"provider": "apns", "which": "update", "token": UPDATE})
    report("said", "The checker passes on all twelve units.", agent="a3")
    check("a report he has not seen changes the numbers", changed(2))
    if len(asked) > 1:
        a = asked[1]
        check("at the address of the banner that is up, quietly", a["to"] == UPDATE and a["event"] == "update" and a["urgent"] is False and a["counts"]["waiting"] == 2 and a["counts"]["unread"] == 1, shown(1))

    print("He lets both questions go, with the app not in front")
    for m in [m for m in http("GET", "/v1/radio/feed?since=0")[1]["messages"] if m.get("open")]:
        http("POST", "/v1/radio/dismiss", {"message": m["id"]})
    check("the banner comes down", changed(3))
    if len(asked) > 2:
        check("at the address of the banner that was up, with nobody waiting", asked[2]["to"] == UPDATE and asked[2]["event"] == "end" and asked[2]["counts"]["waiting"] == 0, shown(2))
    report("said", "Deployed.", agent="a3")
    check("with none up and nobody waiting, a report puts nothing up", quiet(3))

    print("He has the chat open")
    showing = threading.Event()
    def show_chat():
        rev = 0
        while not showing.is_set():
            got = http("GET", f"/v1/radio/feed?since={rev}&wait=2", showing=True)[1]
            rev = got["rev"] if got else rev
    reader = threading.Thread(target=show_chat); reader.start()
    time.sleep(1)
    report("asked", "Deploy to staging now?", agent="a4")
    check("an agent that starts waiting puts nothing up on a phone he is looking at", quiet(3))
    showing.set(); reader.join()
    # the tower takes him to be there for a minute after the app last asked for the chat
    check("nor when he puts it down having seen it", quiet(3, 75))

    print("Another agent waits; the push service says the address has gone")
    answer_with[0] = 410
    report("asked", "Tag the release?")
    check("the tower asks for a banner", changed(4) and asked[3]["event"] == "start" and asked[3]["counts"]["waiting"] == 2, shown(3))
    time.sleep(2)
    answer_with[0] = 200
    report("asked", "Publish the notes?", agent="a2")
    check("it forgets the address: nothing more is asked", quiet(4))

    raw = b"".join(a["raw"] for a in asked)
    check("nothing the relay was given holds a name or a word of theirs", not any(w in raw for w in (b"shop", b"Rewrite", b"e2e", b"Ship", b"checker", b"Deploy", b"Tag", b"one", b"two")), raw)
    log = open(os.path.join(work, "tower.log")).read()
    check("the log says what was asked and what came back", "banner: asked the relay to put a banner up on \"phone\": done" in log and "change the banner on \"phone\": done" in log and "take the banner down on \"phone\": done" in log and "no longer good (Unregistered); forgotten" in log, [l for l in log.splitlines() if "banner" in l])
    check("and holds no address", START not in log and UPDATE not in log and SECRET not in log)
finally:
    tower.terminate()
    tower.wait(10)
    server.shutdown()

failed = [name for name, ok in results if not ok]
print(f"\n{len(results) - len(failed)} of {len(results)} passed." + ("" if not failed else "  Failed:\n  - " + "\n  - ".join(failed)))
if failed:
    print(f"The tower's log: {os.path.join(work, 'tower.log')}")
else:
    shutil.rmtree(work, ignore_errors=True)
sys.exit(1 if failed else 0)
