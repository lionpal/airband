"""Time and memory of a local model doing airband's own summary job, on real messages from a tower.

Each message goes through `airband triage`, which is the tower's real prompt and code path, pointed at a
llama.cpp server on this computer. See docs/experiments/2026-11-20-local-summary-models.md.

  BENCH_DIR=<folder with msgs/ and meta.json> python3 tools/bench-local-models.py name=<hf repo>:<quant> ...
"""
import json, os, subprocess, sys, tempfile, time, urllib.request, concurrent.futures as cf
S = os.environ.get("BENCH_DIR", ".")   # holds msgs/NN.txt and meta.json (see sample.py in the write-up); results are written here
SERVER = os.environ.get("LLAMA_SERVER", "llama-server")           # from a llama.cpp release, or `brew install llama.cpp`
AIRBAND = os.environ.get("AIRBAND", "target/release/airband")
CACHE = os.environ.get("LLAMA_CACHE", os.path.expanduser("~/Library/Caches/llama.cpp"))  # where models are downloaded to
PORT = 8791
MODELS = [a.split("=", 1) for a in sys.argv[1:]]  # name=hf-repo:quant
meta = json.load(open(f"{S}/meta.json"))
env = dict(os.environ, LLAMA_CACHE=CACHE)

def footprint(pid):
    # what Activity Monitor calls Memory: includes what the graphics chip holds for the process
    out = subprocess.run(["top", "-l", "1", "-pid", str(pid), "-stats", "mem"], capture_output=True, text=True).stdout.strip().splitlines()[-1].strip()
    n = float(out.rstrip("+-MGKB ").strip() or 0)
    return n * (1024 if "G" in out else 1 if "M" in out else 1 / 1024)

def healthy():
    try:
        return urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2).status == 200
    except Exception:
        return False

results = {}
for name, repo in MODELS:
    home = tempfile.mkdtemp(prefix="airband-bench-")
    henv = dict(os.environ, HOME=home)
    subprocess.run([AIRBAND, "model", "local", "--url", f"http://127.0.0.1:{PORT}/v1", "--model", name], env=henv, capture_output=True)
    args = [SERVER, "-hf", repo, "--no-mmproj", "--reasoning", "off", "-c", "24576", "-np", "3", "--host", "127.0.0.1", "--port", str(PORT)]
    began = time.time()
    server = subprocess.Popen(args, env=env, stdout=open(f"{S}/{name}.server.log", "w"), stderr=subprocess.STDOUT)
    while not healthy():
        if server.poll() is not None:
            print(name, "server exited; see its log"); break
        time.sleep(2)
    if server.poll() is not None:
        continue
    ready = time.time() - began
    def one(i):
        t = time.time()
        r = subprocess.run([AIRBAND, "triage"], stdin=open(f"{S}/msgs/{i:02d}.txt"), env=henv, capture_output=True, text=True, timeout=180)
        return time.time() - t, r.stdout.strip() or ("ERR " + r.stderr.strip()[:200])
    first, _ = one(0)                       # the first call, model just loaded
    times, outs, peak = [], [], footprint(server.pid)
    for m in meta:
        s, out = one(m["i"]); times.append(s); outs.append(out); peak = max(peak, footprint(server.pid))
    t = time.time()                         # three at once, as when several agents report together
    with cf.ThreadPoolExecutor(3) as ex:
        three = [s for s, _ in ex.map(one, [3, 9, 15])]
    three_wall = time.time() - t
    peak = max(peak, footprint(server.pid))
    server.terminate(); server.wait()
    times_sorted = sorted(times)
    results[name] = {"ready_s": round(ready, 1), "first_s": round(first, 1), "median_s": round(times_sorted[len(times)//2], 1), "slowest_s": round(times_sorted[-1], 1), "fastest_s": round(times_sorted[0], 1),
                     "three_at_once_wall_s": round(three_wall, 1), "three_each_s": [round(x, 1) for x in three], "peak_mb": round(peak), "outs": outs}
    json.dump(results, open(f"{S}/results.json", "w"), indent=1)
    print(name, {k: v for k, v in results[name].items() if k != "outs"}, flush=True)
