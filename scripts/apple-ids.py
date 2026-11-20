#!/usr/bin/env python3
"""Make sure Apple knows the app's three identifiers and what each may do.

    scripts/apple-ids.py

  dev.airband.app          the app: push notifications
  dev.airband.app.notify   its notification extension, which opens a sealed notification on the phone

One thing Apple's API cannot turn on (it refuses the name): Communication Notifications, which lets a
notification carry the agent's badge as a sender's picture. That is a tick box at developer.apple.com →
Certificates, Identifiers & Profiles → Identifiers → dev.airband.app. scripts/testflight.sh sees whether it is on.

Safe to run again: what is already there is left alone. Uses the App Store Connect API key named in the
environment (ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH), as scripts/testflight.sh does. Needs PyJWT with
cryptography (pip3 install 'pyjwt[crypto]').
"""
import json
import os
import sys
import time
import urllib.error
import urllib.request

import jwt

API = "https://api.appstoreconnect.apple.com/v1"
IDS = {
    "dev.airband.app": ("airband", ["PUSH_NOTIFICATIONS"]),
    "dev.airband.app.notify": ("airband notifications", []),
    "dev.airband.app.widgets": ("airband widgets", []),
}


def token():
    key = open(os.environ["ASC_KEY_PATH"]).read()
    claims = {"iss": os.environ["ASC_ISSUER_ID"], "iat": int(time.time()), "exp": int(time.time()) + 600, "aud": "appstoreconnect-v1"}
    return jwt.encode(claims, key, algorithm="ES256", headers={"kid": os.environ["ASC_KEY_ID"]})


def call(method, path, body=None):
    req = urllib.request.Request(API + path, method=method, data=json.dumps(body).encode() if body else None)
    req.add_header("Authorization", "Bearer " + token())
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=30) as res:
            text = res.read()
            return json.loads(text) if text else {}
    except urllib.error.HTTPError as e:
        detail = "; ".join(err.get("detail", "") for err in json.loads(e.read() or b"{}").get("errors", []))
        sys.exit(f"✗ Apple refused {method} {path.split('?')[0]} ({e.code}): {detail}")


def certificate(serial):
    """The id Apple knows a signing certificate by, from its serial number: for asking for a profile made with
    that certificate and no other."""
    found = call("GET", "/certificates?limit=200")
    for c in (found or {}).get("data", []):
        if c["attributes"].get("serialNumber", "").upper().lstrip("0") == serial.upper().lstrip("0"):
            print(c["id"])
            return
    sys.exit(f"Apple has no certificate with serial {serial} for this team")


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "certificate":
        return certificate(sys.argv[2])
    for identifier, (name, wanted) in IDS.items():
        found = [b for b in call("GET", f"/bundleIds?filter[identifier]={identifier}&limit=50")["data"] if b["attributes"]["identifier"] == identifier]
        if found:
            bundle = found[0]["id"]
        else:
            made = call("POST", "/bundleIds", {"data": {"type": "bundleIds", "attributes": {"identifier": identifier, "name": name, "platform": "IOS"}}})
            bundle = made["data"]["id"]
            print(f"✓ registered {identifier}")
        have = {c["attributes"]["capabilityType"] for c in call("GET", f"/bundleIds/{bundle}/bundleIdCapabilities")["data"]}
        for capability in wanted:
            if capability in have:
                continue
            call("POST", "/bundleIdCapabilities", {"data": {"type": "bundleIdCapabilities", "attributes": {"capabilityType": capability, "settings": []},
                                                            "relationships": {"bundleId": {"data": {"type": "bundleIds", "id": bundle}}}}})
            print(f"✓ {identifier}: {capability} turned on")
        print(f"✓ {identifier}: {', '.join(sorted(have | set(wanted))) or 'no capabilities needed'}")


if __name__ == "__main__":
    main()
