#!/usr/bin/env python3
"""What the relay is built from, as cargo says, and whether a change touched any of it.

    scripts/relay-sources.py                  the files and folders in this repository the relay is built from
    scripts/relay-sources.py --changed A B    "true" if a file changed between commits A and B is one of them
    scripts/relay-sources.py --since-deployed D H
                                              "true" if what changed from the deployed commit D to H is one of
                                              them, the infrastructure (infra/) or the deploy workflow
    scripts/relay-sources.py --self-test      checks the list holds what the relay is known to be built from

Nothing is listed by hand. The relay is built once (`cargo build -p airband-relay`), and the list is what that
build read: every source file the compiler read for the relay and the crates in this repository it depends on
(cargo's dep-info for the binary), every file or folder their build scripts said they read
(`cargo:rerun-if-changed`), and the manifest of each such crate and of the workspace. So a file split out of a
module, a new crate the relay comes to depend on, or the catalogue the core compiles in is counted without anyone
remembering to list it (deploy-relay.yml).
"""
import json
import os
import subprocess
import sys

ROOT = os.path.realpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))


def inside(path):
    """The path relative to the repository, or None when it is outside it (a registry crate, the target folder)."""
    real = os.path.realpath(path)
    target = os.path.realpath(os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target"))
    if real == ROOT or not real.startswith(ROOT + os.sep) or real.startswith(target + os.sep):
        return None
    return os.path.relpath(real, ROOT)


def dep_info(path):
    """The files a cargo dep-info file names (`out: a b c`, with `\\ ` for a space in a name)."""
    with open(path, encoding="utf-8") as f:
        first = f.read().split("\n\n")[0].replace("\\\n", " ")
    _, _, deps = first.partition(": ")
    names, word, i = [], "", 0
    while i < len(deps):
        c = deps[i]
        if c == "\\" and i + 1 < len(deps) and deps[i + 1] == " ":
            word, i = word + " ", i + 2
            continue
        if c.isspace():
            if word:
                names.append(word)
            word = ""
        else:
            word += c
        i += 1
    return names + ([word] if word else [])


def sources():
    out = subprocess.run(["cargo", "build", "-q", "-p", "airband-relay", "--message-format=json"], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    found = {"Cargo.toml", "Cargo.lock"}
    for line in out.splitlines():
        msg = json.loads(line)
        reason = msg.get("reason")
        manifest = msg.get("manifest_path")
        local = manifest and inside(manifest)
        if not local:
            continue
        found.add(local)
        if reason == "compiler-artifact":
            for name in msg.get("filenames", []):
                stem, ext = os.path.splitext(name)
                for d in (stem + ".d", os.path.join(os.path.dirname(name), "deps", os.path.basename(stem) + ".d")):
                    if ext in ("", ".exe", ".rlib", ".rmeta") and os.path.exists(d):
                        found.update(p for p in map(inside, dep_info(d)) if p)
        elif reason == "build-script-executed":
            output = os.path.join(os.path.dirname(msg["out_dir"]), "output")
            with open(output, encoding="utf-8") as f:
                for said in f.read().splitlines():
                    for prefix in ("cargo:rerun-if-changed=", "cargo::rerun-if-changed="):
                        if said.startswith(prefix):
                            p = inside(said[len(prefix):])
                            if p:
                                found.add(p)
    return sorted(found)


def touches(changed, made_from):
    return any(c == m or c.startswith(m.rstrip("/") + "/") for c in changed for m in made_from)


# besides what the relay is built from, a deploy also ships these
DEPLOYED_WITH = ["infra", ".github/workflows/deploy-relay.yml"]


def changed_between(a, b, cwd=ROOT):
    # -z: names exactly as git has them, one per NUL, whatever characters they hold
    out = subprocess.run(["git", "diff", "-z", "--name-only", a, b], cwd=cwd, check=True, capture_output=True).stdout
    return [n.decode() for n in out.split(b"\0") if n]


def needs_deploy(deployed, head, made_from, cwd=ROOT):
    """Whether head ships something the deployed commit does not: the difference from the last deploy, not this
    push's, so a change whose own run was dropped or failed is still in it."""
    return touches(changed_between(deployed, head, cwd), made_from + DEPLOYED_WITH)


def deploy_self_test():
    """needs_deploy against a throwaway repository: a relay change then a docs change, with the deploy recorded
    before both, still deploys (the run for the relay change having been dropped)."""
    import tempfile
    with tempfile.TemporaryDirectory() as repo:
        def git(*a):
            return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *a], cwd=repo, check=True, capture_output=True, text=True).stdout.strip()
        def commit(path, text):
            os.makedirs(os.path.join(repo, os.path.dirname(path)), exist_ok=True)
            with open(os.path.join(repo, path), "w", encoding="utf-8") as f:
                f.write(text)
            git("add", "-A")
            git("commit", "-q", "-m", path)
            return git("rev-parse", "HEAD")
        git("init", "-q")
        deployed = commit("crates/airband-relay/src/main.rs", "a")
        commit("crates/airband-relay/src/main.rs", "b")
        docs = commit("docs/state.md", "c")
        made_from = ["crates/airband-relay/src"]
        assert needs_deploy(deployed, docs, made_from, repo), "a relay change behind a docs push is skipped"
        assert not needs_deploy(docs, docs, made_from, repo), "nothing changed, yet it deploys"
        later = commit("docs/protocols.md", "d")
        assert not needs_deploy(docs, later, made_from, repo), "a docs change alone deploys"
        infra = commit("infra/cdk.mjs", "e")
        assert needs_deploy(later, infra, made_from, repo), "an infrastructure change is skipped"


def main(args):
    made_from = sources()
    if args[:1] == ["--changed"]:
        print("true" if touches(changed_between(args[1], args[2]), made_from) else "false")
    elif args[:1] == ["--since-deployed"]:
        print("true" if needs_deploy(args[1], args[2], made_from) else "false")
    elif args[:1] == ["--self-test"]:
        # files the relay is known to be built from, each a way a hand list went wrong before
        for known in ["crates/airband-relay/src/handlers.rs", "crates/airband-core/src/relay/push.rs", "crates/airband-core/src/relay/routes.rs", "crates/airband-io/src/http.rs", "catalog/rev", "Cargo.lock"]:
            assert touches([known], made_from), f"{known} is not counted"
        for unrelated in ["docs/state.md", "apps/apple/Sources/RadioModel.swift", "crates/airband-radio/src/desk.rs", "crates/airband-cli/src/main.rs"]:
            assert not touches([unrelated], made_from), f"{unrelated} is counted"
        assert not touches([], made_from)
        deploy_self_test()
        print(f"ok: {len(made_from)} paths")
    else:
        print("\n".join(made_from))


if __name__ == "__main__":
    main(sys.argv[1:])
