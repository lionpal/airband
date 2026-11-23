#!/usr/bin/env python3
"""End to end, for notifications: a real tower, a throwaway home, and a stand-in for the relay.

    cargo build --release -p airband-cli && tools/e2e-push.py        about a minute, no model

The stand-in relay takes what the tower asks it to hand to a push service and keeps it. Each notification is
then opened here, with the pairing's secret and nothing from the tower's code (HKDF and ChaCha20-Poly1305 from
Python's `cryptography`), which shows the sealing is the published one and that the relay's part of the
request holds nothing readable. Checked: what is sent and to whom, that it is sent once, that a phone showing
the chat is left alone and one only following it is not, a permission card, and an address the push service
says has gone. With no model, a message is a question if the agent said so and a report otherwise; that
chatter gets no notification is checked in the tower's own tests.
"""
import base64, json, os, shutil, subprocess, sys, tempfile, threading, time, urllib.request, urllib.error
import wire
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "airband")
PORT, RELAY, OWN, RADIO = wire.free_port(), wire.free_port(), "push-own", "push-radio"
work = tempfile.mkdtemp(prefix="airband-push-")
home, project = os.path.join(work, "home"), os.path.join(work, "code", "shop")
for d in (os.path.join(home, ".airband"), project):
    os.makedirs(d)
SECRET, ADDRESS = os.urandom(32).hex(), os.urandom(32).hex()
json.dump({"port": PORT, "token": OWN, "radio_token": RADIO, "name": "Charles", "station": "e2e", "permission_wait": 30, "summarizer": {"kind": "none"},
           "relay": f"http://127.0.0.1:{RELAY}", "pairings": [{"name": "phone", "secret": SECRET}]}, open(os.path.join(home, ".airband", "config.json"), "w"))
env = dict(os.environ, HOME=home)

def derived(label, size=32):
    return HKDF(algorithm=hashes.SHA256(), length=size, salt=b"airband.relay/1", info=label.encode()).derive(bytes.fromhex(SECRET))
CHANNEL, PULL = derived("channel")[:16].hex(), derived("pull").hex()
def opened(sealed):
    raw = base64.b64decode(sealed)
    return json.loads(ChaCha20Poly1305(derived("push")).decrypt(raw[:12], raw[12:], f"airband.relay/1 {CHANNEL} push".encode()))

pushes, answer_with = [], [200]
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
        if self.path != f"/v1/channels/{CHANNEL}/push":
            return self.reply(404)
        pushes.append({"auth": self.headers.get("authorization"), "raw": body, **json.loads(body)})
        code = answer_with[0]
        self.reply(code, json.dumps({"sent": True} if code == 200 else {"sent": False, "why": "gone"}).encode())
server = ThreadingHTTPServer(("127.0.0.1", RELAY), Relay)
threading.Thread(target=server.serve_forever, daemon=True).start()

results = []
def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"   [{detail}]" if detail and not ok else ""), flush=True)
def sealed(method, path, body=None, timeout=40, showing=True):
    """A request as the paired phone makes it straight to the tower: sealed with the pairing's key, so the tower
    knows it as that pairing (by its channel) and can wake it."""
    call = {"t": int(time.time() * 1000), "id": os.urandom(8).hex(), "method": method, "path": path, "body": body if body is not None else {},
            "schema": wire.SCHEMA, "presence": {"on_call": False, "showing_chat": showing}}
    nonce = os.urandom(12)
    plain = json.dumps(call).encode()
    sealed_call = nonce + ChaCha20Poly1305(derived("to tower")).encrypt(nonce, plain, f"airband.relay/1 {CHANNEL} to tower".encode())
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}/v1/sealed/{CHANNEL}", method="POST", data=sealed_call)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
    except (urllib.error.HTTPError, urllib.error.URLError):
        return 0, None
    answer = json.loads(ChaCha20Poly1305(derived("to radio")).decrypt(raw[:12], raw[12:], f"airband.relay/1 {CHANNEL} to radio".encode()))
    return answer["status"], answer.get("body")
def http(method, path, body=None, token=RADIO, radio="phone", timeout=40, showing=True):
    """`showing`: the radio says the chat is in front of him, as every request of a radio's says whether it is.
    The phone is the paired one, and speaks sealed; another radio speaks with the tower's radio token."""
    if radio == "phone" and token == RADIO and path.startswith("/v1/radio/"):
        return sealed(method, path, body, timeout, showing)
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json", wire.SCHEMA_HEADER: wire.SCHEMA, "x-airband-radio": radio,
                                          "x-airband-presence": json.dumps({"on_call": False, "showing_chat": showing})})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        # a refusal comes with a 4xx and says what it is in its body
        raw = e.read()
        try: return e.code, (json.loads(raw) if raw else None)
        except ValueError: return e.code, None
    except urllib.error.URLError:
        return 0, None
def report(*words, agent="a1"):
    return subprocess.run([BIN, "report", "--id", agent, "--harness", "script", "--cwd", project, *words], env=env, capture_output=True, text=True, timeout=60)
def pushed(count, within=20):
    """Wait until that many notifications have been asked for in all."""
    until = time.time() + within
    while len(pushes) < count and time.time() < until:
        time.sleep(0.2)
    return len(pushes) >= count
def quiet(count, for_seconds=8):
    """Nothing more is asked for in the time a notification takes to be decided and sent."""
    time.sleep(for_seconds)
    return len(pushes) == count

tower = subprocess.Popen([BIN, "run"], env=env, stdout=open(os.path.join(work, "tower.log"), "w"), stderr=subprocess.STDOUT)
try:
    for _ in range(100):
        if http("GET", "/v1/status", token=OWN)[0] == 200:
            break
        time.sleep(0.1)
    else:
        sys.exit("The tower did not start. Its log: " + os.path.join(work, "tower.log"))

    print("No phone can be woken yet")
    report("asked", "Ship the first fix?")
    check("with no address given, nothing is asked of the relay", quiet(0, 7))

    print("The phone says where it can be woken, and is not looking")
    check("the tower takes the address", http("POST", "/v1/radio/push", {"provider": "apns", "token": ADDRESS})[0] == 200)
    http("POST", "/v1/radio/push", {"provider": "apns", "token": ADDRESS})
    report("asked", "Rewrite the two easy questions, or leave them?")
    check("a question wakes it", pushed(1))
    if pushes:
        p = pushes[0]
        note = opened(p["sealed"])
        check("the relay is asked as this channel's tower, for that address, and told only that it is a question",
              p["auth"] == f"Bearer {PULL}" and p["to"] == ADDRESS and p["provider"] == "apns" and p["question"] is True and sorted(k for k in p if k not in ("auth", "raw")) == ["provider", "question", "sealed", "to"], {k: v for k, v in p.items() if k != "raw"})
        plain = json.dumps({k: v for k, v in p.items() if k not in ("auth", "raw", "sealed")}).encode()
        check("nothing the relay is given can be read: no name, no words", not [w for w in (b"shop", b"Rewrite", b"e2e") if w in plain or w in p["raw"].replace(p["sealed"].encode(), b"")],
              [w for w in (b"shop", b"Rewrite", b"e2e") if w in p["raw"]])
        check("opened with the pairing's secret, it says who asked and what", note["from"] == "shop 1" and note["badge"] == "S1" and note["agent"] and note["group"] == "shop" and note["station"] == "e2e" and note["kind"] == "needs-you" and "Rewrite the two easy questions" in note["text"], note)
        line = [m for m in http("GET", "/v1/radio/feed?since=0", showing=False)[1]["messages"] if "Rewrite" in m["text"]]
        check("and names the message an answer goes to", line and note["id"] == line[-1]["id"], note)
    check("once: the same message is not sent again, though the phone follows the chat without showing it", quiet(1))

    print("A report")
    report("said", "The checker passes on all twelve units.")
    check("something no model read wakes it too, as what it is, and is not called a question", pushed(2) and pushes[1]["question"] is False and opened(pushes[1]["sealed"])["kind"] == "unjudged")

    print("The phone has the chat open")
    showing, looking, saw_card = threading.Event(), threading.Event(), threading.Event()
    def show_chat():
        rev = 0
        while not showing.is_set():
            got = http("GET", f"/v1/radio/feed?since={rev}&wait=2")[1]
            rev = got["rev"] if got else rev
            # the tower has answered one: it knows the chat is in front of him
            looking.set()
            # the card came to the chat he is looking at, and asking for what follows it tells the tower he has it
            if any(m.get("permission") and m["open"] for m in (got or {"messages": []})["messages"]) and http("GET", f"/v1/radio/feed?since={rev}")[0] == 200:
                saw_card.set()
    reader = threading.Thread(target=show_chat); reader.start()
    looking.wait(10)
    report("asked", "Deploy to staging now?")
    check("what arrives while he is looking at the chat wakes nothing", quiet(2))
    out = []
    def ask_permission():
        r = report("permission", "run `npm run deploy`")
        out.append(r.returncode)
    asking = threading.Thread(target=ask_permission); asking.start()
    # the card is in the chat he is looking at, and the tower has been told he has it
    saw_card.wait(10)
    showing.set(); reader.join()

    print("He puts the phone down with a permission card waiting")
    # the card was put while the chat was open and he has not seen it change since: a second card, now
    check("the card he saw arrive wakes nothing", quiet(2, 6))
    card = [m for m in http("GET", "/v1/radio/feed?since=0", showing=False)[1]["messages"] if m.get("permission") and m["open"]]
    if card: http("POST", "/v1/radio/utterance", {"text": "Deny", "id": os.urandom(6).hex(), "re": card[-1]["id"], "act": {"kind": "verdict", "allow": False}})
    asking.join(40)
    asking = threading.Thread(target=ask_permission); asking.start()
    check("a permission card wakes the phone that was there a moment ago", pushed(3))
    if len(pushes) >= 3:
        note = opened(pushes[2]["sealed"])
        check("it says what will run, word for word, and that it is a permission card", note.get("permission") is True and note["text"] == "Wants to run `npm run deploy`. Allow?" and pushes[2]["question"] is True, note)
        allow = {"text": "Allow", "id": os.urandom(6).hex(), "re": note["id"], "act": {"kind": "verdict", "allow": True}}
        reply = None
        for _ in range(300):
            took = http("POST", "/v1/radio/utterance", allow)[1]
            if took and took["state"] == "answered":
                reply = took["reply"]
                break
            time.sleep(0.1)
        asking.join(40)
        check("Allow, sent as the answer to the message the notification named, lets it run", reply and reply["say"] == "Allowed." and out[-1:] == [0], (reply, out))

    print("Another radio looking does not stand in for the phone")
    http("GET", "/v1/radio/feed?since=999999", radio="desk")
    report("asked", "Merge it?", agent="a2")
    check("the phone is still woken", pushed(4))

    print("The push service says the address has gone")
    answer_with[0] = 410
    report("asked", "Tag the release?")
    check("the tower asks, and is told", pushed(5))
    # and has forgotten the address, as its log says
    for _ in range(100):
        if "the phone's address is no longer good; forgotten" in open(os.path.join(work, "tower.log")).read(): break
        time.sleep(0.1)
    answer_with[0] = 200
    report("asked", "Publish the notes?")
    check("it forgets the address: nothing more is asked for it", quiet(5))
    log = open(os.path.join(work, "tower.log")).read()
    # the log names the radio by the start of its channel, never by the name he gave the pairing
    asked = f"push: asked the relay to wake radio {wire.derive(SECRET)['channel'][:8]} for line "
    check("the log says what was asked and what came back", log.count(asked) == 5 and "sent" in log and "the phone's address is no longer good; forgotten" in log, [l for l in log.splitlines() if "push" in l])
    check("and names the radio by id only", '"phone"' not in log, [l for l in log.splitlines() if "phone" in l])
    check("and holds no address", ADDRESS not in log and SECRET not in log)
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
