#!/usr/bin/env python3
"""Does the tower's memory stay flat? A throwaway tower with no model is put through its everyday paths many
times over (agents reporting, saying what they are doing, a radio reading the feed and answering, pictures,
his typed messages read from a transcript), and its footprint is read as it goes.

    tools/soak.py            6,000 rounds
    tools/soak.py 20000      more

It keeps the last 1,000 messages in memory, so the footprint climbs until that many have been said and should
then stay where it is. The run fails if it is still climbing in the second half.
"""
import json, os, re, shutil, subprocess, sys, tempfile, time, urllib.request, urllib.error

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "airband")
PORT, OWN, RADIO = 7395, "soak-own", "soak-radio"
ROUNDS = int(sys.argv[1]) if len(sys.argv) > 1 else 6000
work = tempfile.mkdtemp(prefix="airband-soak-")
home, projects = os.path.join(work, "home"), os.path.join(work, "code")
for d in (os.path.join(home, ".airband"), *(os.path.join(projects, p) for p in ("catalog", "orders"))):
    os.makedirs(d)
json.dump({"port": PORT, "token": OWN, "radio_token": RADIO, "relay": None, "name": "Charles", "station": "soak", "summarizer": {"kind": "none"}}, open(os.path.join(home, ".airband", "config.json"), "w"))
env = dict(os.environ, HOME=home)

def http(method, path, body=None, token=RADIO):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json", "x-airband-radio": "soak", "x-airband-presence": '{"on_call":false,"showing_chat":true}'})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
    except (urllib.error.HTTPError, urllib.error.URLError):
        return 0, None

def footprint(pid):
    out = subprocess.run(["footprint", str(pid)], capture_output=True, text=True).stdout
    m = re.search(r"phys_footprint: (\d+(?:\.\d+)?) (KB|MB)", out)
    return float(m.group(1)) / (1024 if m.group(2) == "KB" else 1) if m else 0.0

transcripts = {a: os.path.join(work, f"{a}.jsonl") for a in ("a1", "a2", "a3")}
def event(agent, project, kind, **more):
    return http("POST", "/v1/agent/events", {"schema": "airband.agent/1", "agent": {"id": agent, "harness": "claude-code", "cwd": os.path.join(projects, project), "meta": {"transcript": transcripts[agent]},
                "deliver": {"kind": "command", "argv": ["true"]}}, "event": {"type": kind, **more}}, token=OWN)

tower = subprocess.Popen([BIN, "run"], env=env, stdout=open(os.path.join(work, "tower.log"), "w"), stderr=subprocess.STDOUT)
try:
    for _ in range(100):
        if http("GET", "/v1/status", token=OWN)[0] == 200: break
        time.sleep(0.1)
    else:
        sys.exit("The tower did not start. Its log: " + os.path.join(work, "tower.log"))
    filler = "The supplier import ran and every row was checked against the catalog. " * 8
    readings, since, started = [], 0, time.time()
    for n in range(1, ROUNDS + 1):
        agent, project = (("a1", "catalog"), ("a2", "catalog"), ("a3", "orders"))[n % 3]
        stamp = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())
        with open(transcripts[agent], "a") as f:
            f.write(json.dumps({"type": "user", "uuid": f"{n:032x}", "timestamp": stamp, "origin": {"kind": "human"}, "message": {"role": "user", "content": f"round {n}: check the import again"}}) + "\n")
            f.write(json.dumps({"type": "assistant", "effort": "high", "message": {"model": "claude-opus-4-7", "content": [{"type": "tool_use", "name": "Bash", "input": {"command": "npm test", "description": f"Run the tests, round {n}"}}]}}) + "\n")
        event(agent, project, "working")
        event(agent, project, "activity", text=f"Running the tests, round {n}")
        event(agent, project, "said", text=f"Round {n}. {filler}")
        code, f = http("GET", f"/v1/radio/feed?since={since}&limit=50")
        if f:
            since = f["rev"]
        if n % 5 == 0:
            http("POST", "/v1/radio/utterance", {"text": f"@catalog keep going, round {n}", "id": os.urandom(6).hex()})
        if n % 500 == 0 or n == 100:
            mb = footprint(tower.pid)
            readings.append((n, mb))
            print(f"  after {n:>6} rounds: {mb:.1f} MB", flush=True)
    half = [mb for n, mb in readings if n >= ROUNDS // 2]
    grew = half[-1] - half[0]
    print(f"{ROUNDS} rounds in {time.time() - started:.0f} s. Second half: {half[0]:.1f} MB to {half[-1]:.1f} MB.")
    ok = grew < 1.0
    print("Flat." if ok else f"Still climbing: {grew:.1f} MB over the second half. The tower's log: {os.path.join(work, 'tower.log')}")
    sys.exit(0 if ok else 1)
finally:
    tower.terminate()
    tower.wait(timeout=10)
    if "--keep" not in sys.argv:
        shutil.rmtree(work, ignore_errors=True)
