"""What every suite needs to speak to a tower as a radio of this build: the protocol version core declares.

Read from core's own constant so the suites never carry a copy of it.
"""
import os, re, socket

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
