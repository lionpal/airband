"""Stand-ins for every listening service in the catalogue, on one port of this machine, each speaking that
service's own protocol as the phone's core speaks it (crates/airband-radio/src/ears/*.rs, hears.rs):

    Microsoft Azure Speech   Universal Speech Protocol over a WebSocket: speech.hypothesis, speech.phrase, turn.end
    ElevenLabs Scribe        realtime WebSocket: input_audio_chunk in, partial_transcript / committed_transcript out
    Google Gemini            Live (BidiGenerateContentConstrained): setup / setupComplete, realtimeInput, serverContent
    OpenAI                   Realtime transcription: client secret, input_audio_buffer.append / commit,
                             conversation.item.input_audio_transcription.delta / completed (24 kHz sound)
    Groq, his own server     POST {url}/audio/transcriptions, a WAV in a multipart form

They hear nothing: each answers with the words it was made with, revealed as sound arrives (a few words a
second of it), and the whole of them once the phone says the sound is over. Each checks the sound is what the
service is sent (its rate and its framing) and says what it was given, by service, in `seen`, which the
end-to-end test reads to know that what reached the agent came the way it should have.

A build of the phone's core made with the `stand-ins` feature sends what it would send to these services' hosts
here instead (AIRBAND_STAND_IN_HOSTS, from `hosts`). A build he installs has no such thing.
"""
import base64, hashlib, http.server, json, socket, struct, threading, time

# the hosts the catalogue's rows reach (Azure's at the usual region)
HOSTS = ["eastus.api.cognitive.microsoft.com", "eastus.stt.speech.microsoft.com", "api.elevenlabs.io", "generativelanguage.googleapis.com", "api.openai.com", "api.groq.com", "localhost"]


class Listening:
    def __init__(self, words):
        self.words = words.split()
        self.lock = threading.Lock()
        # by service: seconds of sound it was given, how many partial answers it gave, whether it gave its final
        # words, and anything wrong with what it was sent
        self.seen = {}
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.handler())
        self.port = server.server_address[1]
        self.server = server
        threading.Thread(target=server.serve_forever, daemon=True).start()

    def hosts(self):
        return ",".join(f"{h}=127.0.0.1:{self.port}" for h in HOSTS)

    def shutdown(self):
        self.server.shutdown()

    def note(self, service, **kv):
        with self.lock:
            s = self.seen.setdefault(service, {"seconds": 0.0, "partials": 0, "final": False, "wrong": []})
            for k, v in kv.items():
                if k == "seconds":
                    s["seconds"] += v
                elif k == "partials":
                    s["partials"] += v
                elif k == "wrong":
                    s["wrong"].append(v)
                else:
                    s[k] = v

    def so_far(self, seconds):
        """The words heard after this much sound: a few a second, never all of them before the end."""
        return " ".join(self.words[: max(1, min(len(self.words) - 1, int(seconds * 2.5)))])

    def handler(self):
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args):
                pass

            def answer(self, status, body, kind="application/json"):
                data = body if isinstance(body, bytes) else (body if isinstance(body, str) else json.dumps(body)).encode()
                self.send_response(status)
                self.send_header("content-type", kind)
                self.send_header("content-length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def body(self):
                n = int(self.headers.get("content-length") or 0)
                return self.rfile.read(n) if n else b""

            def do_GET(self):
                if self.headers.get("upgrade", "").lower() == "websocket":
                    return outer.socket(self)
                # a key being tested: it is a good one
                self.answer(200, {"data": [], "models": []})

            def do_POST(self):
                body, path = self.body(), self.path
                if path.endswith("/sts/v1.0/issueToken"):
                    claims = base64.urlsafe_b64encode(json.dumps({"exp": int(time.time()) + 600}).encode()).decode().rstrip("=")
                    return self.answer(200, f"h.{claims}.s", "text/plain")
                if path.endswith("/single-use-token/realtime_scribe"):
                    return self.answer(200, {"token": "stand-in"})
                if path.endswith("/auth_tokens"):
                    return self.answer(200, {"name": "auth_tokens/stand-in"})
                if path.endswith("/realtime/client_secrets"):
                    asked = json.loads(body or b"{}")
                    fmt = asked.get("session", {}).get("audio", {}).get("input", {}).get("format", {})
                    if asked.get("session", {}).get("type") != "transcription" or fmt != {"type": "audio/pcm", "rate": 24000}:
                        outer.note("openai", wrong=f"session asked for: {asked}")
                    return self.answer(200, {"value": "ek_stand-in", "expires_at": int(time.time()) + 600})
                if path.endswith("/audio/transcriptions"):
                    service = "groq" if path.startswith("/openai/") else "local"
                    if b"RIFF" not in body or b'name="model"' not in body:
                        outer.note(service, wrong="not a WAV and a model in a form")
                    else:
                        riff = body.index(b"RIFF")
                        rate = struct.unpack("<I", body[riff + 24 : riff + 28])[0]
                        if rate != 16000:
                            outer.note(service, wrong=f"rate {rate}")
                        outer.note(service, seconds=(len(body) - riff - 44) / 32000, final=True)
                    return self.answer(200, {"text": " ".join(outer.words)})
                self.answer(404, {"error": f"no stand-in for {path}"})

        return Handler

    # -- WebSockets, by hand: the server's half of RFC 6455

    def socket(self, h):
        key = h.headers["sec-websocket-key"]
        accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        h.send_response(101)
        h.send_header("upgrade", "websocket")
        h.send_header("connection", "Upgrade")
        h.send_header("sec-websocket-accept", accept)
        h.end_headers()
        h.wfile.flush()
        h.close_connection = True
        conn = h.connection
        path = h.path
        if path.startswith("/speech/recognition/"):
            self.usp(conn)
        elif path.startswith("/v1/speech-to-text/realtime"):
            self.chunks(conn, path)
        elif "BidiGenerateContentConstrained" in path:
            self.bidi(conn)
        elif path.startswith("/v1/realtime"):
            self.realtime(conn, h.headers.get("authorization", ""))

    @staticmethod
    def recv(conn):
        """One message: (opcode, payload), or None when the line closed."""
        def exactly(n):
            out = b""
            while len(out) < n:
                got = conn.recv(n - len(out))
                if not got:
                    raise ConnectionError
                out += got
            return out
        try:
            data, first = b"", None
            while True:
                b1, b2 = exactly(2)
                op, n = b1 & 0x0F, b2 & 0x7F
                if n == 126:
                    n = struct.unpack(">H", exactly(2))[0]
                elif n == 127:
                    n = struct.unpack(">Q", exactly(8))[0]
                mask = exactly(4) if b2 & 0x80 else b"\0\0\0\0"
                payload = bytes(c ^ mask[i % 4] for i, c in enumerate(exactly(n)))
                if op == 8:
                    return None
                if op == 9:
                    Listening.send(conn, payload, 10)
                    continue
                if op in (1, 2):
                    first = op
                data += payload
                if b1 & 0x80:
                    return first, data
        except (ConnectionError, OSError):
            return None

    @staticmethod
    def send(conn, payload, op=1):
        data = payload.encode() if isinstance(payload, str) else payload
        n = len(data)
        head = bytes([0x80 | op]) + (bytes([n]) if n < 126 else bytes([126]) + struct.pack(">H", n) if n < 65536 else bytes([127]) + struct.pack(">Q", n))
        conn.sendall(head + data)

    def usp(self, conn):
        service, heard, told = "azure", 0.0, ""
        say = lambda path, body: self.send(conn, f"Path: {path}\r\nX-RequestId: r\r\nContent-Type: application/json\r\n\r\n{json.dumps(body)}")
        while (m := self.recv(conn)) is not None:
            op, data = m
            if op == 1:
                continue  # speech.config, speech.context
            head_len = struct.unpack(">H", data[:2])[0]
            audio = data[2 + head_len :]
            if not audio:
                say("speech.phrase", {"RecognitionStatus": "Success", "DisplayText": " ".join(self.words) + ".", "Offset": 0, "Duration": 1})
                say("turn.end", {})
                self.note(service, final=True)
                continue
            if audio[:4] == b"RIFF":
                if struct.unpack("<I", audio[24:28])[0] != 16000:
                    self.note(service, wrong="the header's rate is not 16 kHz")
                audio = audio[44:]
            heard += len(audio) / 32000
            self.note(service, seconds=len(audio) / 32000)
            if (now := self.so_far(heard)) != told:
                told = now
                say("speech.hypothesis", {"Text": now, "Offset": 0, "Duration": 1})
                self.note(service, partials=1)

    def chunks(self, conn, path):
        service, heard, told = "elevenlabs", 0.0, ""
        if "audio_format=pcm_16000" not in path:
            self.note(service, wrong=f"asked for {path}")
        self.send(conn, json.dumps({"message_type": "session_started", "session_id": "s"}))
        while (m := self.recv(conn)) is not None:
            v = json.loads(m[1])
            if v.get("message_type") != "input_audio_chunk" or v.get("sample_rate") != 16000:
                self.note(service, wrong=f"a message: {list(v)} at {v.get('sample_rate')}")
            pcm = base64.b64decode(v.get("audio_base_64", ""))
            if v.get("commit"):
                self.send(conn, json.dumps({"message_type": "committed_transcript", "text": " ".join(self.words)}))
                self.note(service, final=True)
                continue
            heard += len(pcm) / 32000
            self.note(service, seconds=len(pcm) / 32000)
            if (now := self.so_far(heard)) != told:
                told = now
                self.send(conn, json.dumps({"message_type": "partial_transcript", "text": now}))
                self.note(service, partials=1)

    def bidi(self, conn):
        service, heard, told = "gemini", 0.0, ""
        while (m := self.recv(conn)) is not None:
            v = json.loads(m[1])
            if "setup" in v:
                if "inputAudioTranscription" not in v["setup"]:
                    self.note(service, wrong="no transcription asked for")
                self.send(conn, json.dumps({"setupComplete": {}}))
                continue
            real = v.get("realtimeInput", {})
            if real.get("audioStreamEnd"):
                self.send(conn, json.dumps({"serverContent": {"inputTranscription": {"text": " ".join(self.words)}}}))
                self.send(conn, json.dumps({"serverContent": {"turnComplete": True}}))
                self.note(service, final=True)
                continue
            audio = real.get("audio", {})
            if audio.get("mimeType") != "audio/pcm;rate=16000":
                self.note(service, wrong=f"sound as {audio.get('mimeType')}")
            pcm = base64.b64decode(audio.get("data", ""))
            heard += len(pcm) / 32000
            self.note(service, seconds=len(pcm) / 32000)
            if (now := self.so_far(heard)) != told:
                told = now
                self.send(conn, json.dumps({"serverContent": {"interimInputTranscription": {"text": now}}}))
                self.note(service, partials=1)

    def realtime(self, conn, auth):
        service, heard, told = "openai", 0.0, ""
        if auth != "Bearer ek_stand-in":
            self.note(service, wrong="not the client secret it was given")
        while (m := self.recv(conn)) is not None:
            v = json.loads(m[1])
            if v.get("type") == "input_audio_buffer.commit":
                self.send(conn, json.dumps({"type": "input_audio_buffer.committed", "item_id": "i1"}))
                self.send(conn, json.dumps({"type": "conversation.item.input_audio_transcription.completed", "item_id": "i1", "content_index": 0, "transcript": " ".join(self.words)}))
                self.note(service, final=True)
                continue
            if v.get("type") != "input_audio_buffer.append":
                self.note(service, wrong=f"a message of type {v.get('type')}")
                continue
            pcm = base64.b64decode(v["audio"])
            heard += len(pcm) / 48000  # 24 kHz, 16-bit
            self.note(service, seconds=len(pcm) / 48000)
            if (now := self.so_far(heard)) != told:
                # only what is new, as the service sends it
                self.send(conn, json.dumps({"type": "conversation.item.input_audio_transcription.delta", "item_id": "i1", "content_index": 0, "delta": now[len(told):]}))
                told = now
                self.note(service, partials=1)
