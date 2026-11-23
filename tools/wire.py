"""What every suite needs to speak to a tower as a radio of this build: the protocol version core declares.

Read from core's own constant so the suites never carry a copy of it.
"""
import hashlib, hmac, os, re, socket, urllib.parse

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
with open(os.path.join(ROOT, "crates", "airband-core", "src", "radio.rs")) as f:
    SCHEMA = re.search(r'^pub const SCHEMA: &str = "([^"]+)";$', f.read(), re.M).group(1)
SCHEMA_HEADER = "x-airband-schema"


def free_port():
    """A port nothing on this machine listens on now: each suite's own tower, relay and stand-ins take one, so
    the suites run side by side without reaching each other's."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def refusal(body):
    """The refusal a tower answered with (its `refusal` tag), or None when the answer is not one."""
    return body.get("refusal") if isinstance(body, dict) else None


def reason(body):
    """A refusal's words for him, or "" when it carries none or is not a refusal."""
    return (body.get("reason") or "") if refusal(body) else ""


def derive(secret):
    """What both ends make from a pairing secret (crates/airband-core/src/relay.rs): HKDF-SHA256, RFC 5869."""
    prk = hmac.new(b"airband.relay/1", bytes.fromhex(secret), hashlib.sha256).digest()
    take = lambda label: hmac.new(prk, label.encode() + b"\x01", hashlib.sha256).digest()
    return {"channel": take("channel")[:16].hex(), "radio": take("radio").hex()}


def keys_of(link):
    """What a pairing link's secret makes (`derive`)."""
    return derive(urllib.parse.parse_qs(urllib.parse.urlparse(link).query)["secret"][0])


def serving(link):
    """The line a tower logs once it serves a pairing through the relay. The log names a radio by the start of
    its channel (`radio_tag` in crates/airband-cli/src/tower/mod.rs), never by the name he gave it."""
    return f"serving radio {keys_of(link)['channel'][:8]} through"
