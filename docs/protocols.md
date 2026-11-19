# Protocols

Three versioned protocols: `airband.agent/1` (agents → tower), `airband.radio/1` (phone ↔ tower) and
`airband.relay/1` (how the second one crosses the internet sealed). The first two are plain JSON over HTTP with
`Authorization: Bearer <token>` (the token in `~/.airband/config.json`) when sent straight to the tower. The types are in `crates/airband-core`
and are the source of truth; this page explains them.

Compatibility rule for both: unknown fields are ignored, and unknown `type` / `kind` values are accepted and
skipped. Adding a field or a variant is not a new version. `/2` is for a change an old peer would misread.

## `airband.agent/1`: agents → tower

`POST /v1/agent/events`, one document per event:

```json
{
  "schema": "airband.agent/1",
  "agent": {
    "id": "8f2c…",              // stable for the agent's session, unique on this machine
    "harness": "claude-code",   // free text; the tower has no rules per harness
    "meta": { "model": "…" },   // anything else worth keeping about the agent; the tower stores and shows it
    "cwd": "/Users/c/shop",     // its last part names the callsign ("shop two")…
    "name": "shop",             // …unless a spoken name is given
    "title": "shop 2",          // the name he gave this session in the harness, if he did: it becomes the callsign
    "pid": 5302,                // when this process dies the agent is dropped
    "deliver": { "kind": "claude-code-inbox", "socket": "/tmp/cc-socks/5302.sock", "token": "…" }
  },
  "event": { "type": "said", "text": "…the agent's whole last message…" }
}
```

| `event.type` | Meaning |
|---|---|
| `checkin` | The agent exists. Every other event also checks it in. |
| `working` | It was given something to do; what it last said is stale and is dropped unheard. |
| `said` | It finished a turn; `text` is what it wrote. Optional `kind`: `question` (blocked until answered) or `report`; left out, the tower works it out. |
| `notice` | It needs him at the keyboard (a permission prompt, a login). |
| `picture` | It looked at a picture or made one for him: `path` is a file on the tower's machine (PNG, JPEG, GIF or WebP). `shown: true` means it meant him to see it. The tower keeps a small copy and attaches it to the next `said`. A picture file named in the text of a `said` is attached the same way, with no event. |
| `subagent` | It started a subagent (`done: false`) or one finished (`done: true`); `what` says what the subagent is for. Only counted, for showing how much is going on under one agent. |
| `activity` | What it is doing right now, in a few words (`text`). Shown small while it works, never kept, never read aloud. For a harness the tower cannot ask; `airband report … doing "<text>"` sends one. |
| `signoff` | It is gone. |

| `deliver.kind` | How the tower gets his reply to the agent |
|---|---|
| `claude-code-inbox` | Claude Code's per-session inbox socket. |
| `command` | Runs `argv` with the message on standard input and `AIRBAND_AGENT_ID` in the environment. |
| `tmux` | Types the message into pane `target` and presses Enter. Works for anything in a terminal. |
| none | The agent can be heard but not answered. |

### Asking permission

A harness that stops to ask before its agent does something (run a command, write outside the project) can put
the question to him on the radio. It is a separate call because the harness is waiting on the answer.

| Call | Body → answer | Meaning |
|---|---|---|
| `POST /v1/agent/permission` | `{ schema, agent, what, detail }` → `PermissionState` | `what` finishes the words "wants to …" and is shown to him exactly as given: "run \`npm run deploy\`". `detail` is the whole request, for the more-info page. |
| `GET /v1/agent/permission/<id>?wait=<s>` | → `PermissionState` | Held open up to `wait` seconds (25 at most) or until he answers. |
| `DELETE /v1/agent/permission/<id>` | | The harness has stopped waiting. The card closes. |

`PermissionState` is `{ id, state, message, wait_seconds }`:

| `state` | The harness should |
|---|---|
| `pending` | Ask again, for `wait_seconds` in all, then withdraw the request and ask in its own way. |
| `allow` | Go ahead. |
| `deny` | Refuse. `message` is his own words if he gave a reason; pass them to the agent. |
| `nobody` | Ask in its own way now: no radio has read the feed in the last 90 seconds, or `permission_wait` is 0. |
| `gone` | The same: the tower no longer knows the request. |

The rules are the tower's and hold for every harness. It never decides: only he can turn `pending` into `allow`.
Waiting never becomes a yes. A tapped Allow or Deny answers the card it is on. A spoken "yes" or "no" answers a
request only when exactly one is waiting. Any other words said in answer to a card refuse it and are passed on as
the reason, and handing the choice back to the agent ("do whatever you think") is turned away. The message for a
request carries `permission: true`, and a client offers its two answers and nothing else.

### Putting another harness on the radio

- **No code:** call `airband report` from whatever the harness offers (a notify command, a wrapper script):
  `airband report --id "$SESSION" --harness codex --tmux "$TMUX_PANE" said "$LAST_MESSAGE"`.
  For a permission prompt: `airband report --id "$SESSION" --harness codex permission "run \`$COMMAND\`"` waits
  for him and exits 0 for yes, 1 for no (his reason on standard output), 2 for no answer.
- **An adapter:** a function in `crates/airband-cli/src/adapters/` that maps the harness's notification JSON to an
  `AgentEvent`, selected as `airband hook <harness>`. `claude_code` there is the model: about forty lines.
- **A new way to deliver:** a variant of `Deliver` in `airband-core/src/agent.rs` and its arm in `tower/deliver.rs`.

## `airband.radio/1`: radio ↔ tower

The radio always asks; the tower never calls the phone. All under `/v1/radio/`.

| Call | Body → answer | Meaning |
|---|---|---|
| `GET next` | → `Transmission`, or 204 | Held open up to 25 s. Polling is also how the tower knows a radio is there: 40 s without one and it stops summarising. |
| `POST heard` | `{ "id" }` | That transmission was spoken to the end. Only now is it off the queue. |
| `POST utterance` | `{ "text", "id", "re", "scope" }` → `{ "say", "listen" }` | What he said. `re`: the message he is answering, when he picked one. `scope`: the room he was in (`group:<name>`). `say` is spoken back; `listen` reopens the microphone. |
| `POST ready` | | Free for the next transmission. |

```json
{ "id": "9a41…", "callsign": "shop two", "kind": "question", "say": "Shop two. The checker passes…", "waiting": 2 }
```

Chat mode reads the same messages as a feed, and sends through the same `utterance` call:

| Call | Answer | Meaning |
|---|---|---|
| `GET feed?scope=…&since=<rev>&wait=<s>` | `{ schema, rev, messages, roster, seq, open, projects, station }` | Every line new or changed after change count `since`, oldest change first, each cut to a line or two; pass `rev` back next time. `since=0` gets the newest lines. `scope`: `all`, `group:<name>`, `agent:<callsign>`. Held open up to 25 s. `after=<seq>` is the older form (new lines only) and is still answered. |
| `GET speech/<id>?from=<block>` | `{ rate, data, samples, next, done }` | The next part of a clip of the tower's voice, as far as it has been made: IMA ADPCM blocks, base64. A `Transmission` or a reply names its clip in `speech`. |
| `GET ears` | `{ kind, token, region, language, phrases }` | What the phone should listen with: `device`, or a service and a ten-minute pass for it. |
| `GET message/<id>?form=detail\|full` | `{ text }` | The fuller spoken version, or exactly what the agent wrote. |
| `GET picture/<id>?from=<byte>` | `{ data, next, done }` | One part (120 KB at most, base64) of a picture's kept copy. Ask again from `next` until `done`. |

A message: `{ seq, id, at, from, group, to, kind, text, long, options, recommended, read, permission, pictures, links, agent, rev, open }`.
`agent` is the id of the agent that said it, the same as `id` in the roster and stable while that agent runs;
`from` is only what it is called now. `kind` is `needs-you`, `report` or `fyi`; a client that meets a kind it does
not know shows it as a report. `options` are up to three answers he can tap, `recommended` the position of the one
the agent itself recommended (never a default), and `open` means it needs him and nothing has answered it.
The feed's `open` lists every such message's id, `projects` the names a new agent can be launched in
(`/launch <project> <task>` as an utterance), and `station` what this computer is called.

`kind` can also be `stalled`: the agent said what it would do next and stopped, with the work unfinished and
nothing asked. `/poke` as an utterance (or the spoken word "poke") sends "Keep going." to every agent in that
state, within `scope` if one is given. Each roster entry has a `state` (`working`, `asking`, `stalled`, `idle`)
and a count of `subagents`.

A roster entry may carry `doing`: what a working agent is doing right now, in its harness's own words for its
latest step. The tower asks each working agent's adapter every two seconds, and only while a radio has read the
feed in the last 40 seconds. The feed carries `roster_rev`, a number that changes whenever the roster does; a
radio that passes it back as `roster=` has its waiting `GET feed` answered when the roster changes, not only
when a message does.

The feed's `colours` gives each team a number from 0: its place in the client's own list of team colours. A team
is given the first number no other team has, once, and keeps it.

The feed's `quick` lists the quick messages to offer: six standard ones, then his own (`airband quick add`).
Each is `{ label, text }`. Sending one is an utterance whose `text` is the whole instruction and whose `label`
is the button's words; the chat shows the label.

Accounts, for a harness whose sessions can be moved between logins while they run:

| Call | Body → answer | Meaning |
|---|---|---|
| `GET accounts?fresh=1` | → `{ schema, accounts, note, checked_at }` | Every account of every such harness on this computer. Kept for five minutes; `fresh` checks again (at most every 20 s). |
| `POST accounts/use` | `{ harness, id }` → the same, or `{ error }` | Move that harness's sessions to that account. `error` is words for him. |

An account is `{ id, harness, label, live, limits, problem }`. `id` means something only to the adapter that listed
it; `label` is what to show (for Claude Code, the email); `live` marks the one in use. Each of `limits` is
`{ name, used, resets_at }`: `name` is the window ("week", "5 hours"), `used` runs from 0 to 1, `resets_at` is
milliseconds since 1970. `problem`, when present, says why the account could not be checked and is shown in place
of the limits. Nothing that could sign in as an account is ever sent. Signing in to a new account happens at
the computer (`airband accounts add`), never from a radio.

A new agent can be asked for in any words, on a model and at an effort if he names them: "with fable high, find
out why checkout is slow", "can you get opus to look into the slow import". The same model call that works out
who a reply is for decides this: whether he wants a new agent at all, which project the work belongs in, and
which of the launcher's `models` and `efforts` he named. Only a project the tower knows and names the launcher
lists are taken from its answer. Typed commands (`/launch shop with opus: …`) are read by rule, and the same
rules stand in for set phrases when no model answers. The reply says what was started: "Launching in shop with
fable, high."

A short text is one sentence, with one exception: when the last thing he told an agent was to investigate,
review, compare or explain something and the message is the answer, `text` is an executive summary of up to
about 90 words, conclusion first. It is spoken on a call like any other.

Two tokens open the tower's port. Its own opens everything. A radio's, which is what a direct pairing link
carries, opens `/v1/radio/` only. A radio names itself in the `x-airband-radio` header so the tower can keep its
place in a call apart from another radio's; through the relay the tower uses the pairing's name.

A message may carry `read`: the judge's two or three words for something in it that he should be able to read
whole ("the refund email"), when he asked to be shown it or cannot decide without it. A client offers the full
message under that name. It is the model's call each time, never a rule about particular words.

A message carries `links`: up to four web links the agent wrote (an artifact, a pull request), which the short
text leaves out.

A message carries `pictures`: up to four of `{ id, width, height, bytes }`, the ones the agent meant him to see
first, then ones it named, then the last it looked at. A copy is a JPEG, 1600 pixels at most on its long side,
usually 50 to 300 KB. Nothing is sent until a client asks for it, and it comes in parts because one sealed
message through a relay is limited to 256 KB.

Who a message is for is in its text: leading tags when typed (`@shop2`, `@shop` for the group, `@all`), a
leading callsign when spoken ("shop two, …", "all shop, …", "everyone, …"). No tag: the message in `re` if
there is one. Otherwise the tower reads his words against the last thing each agent said and picks the one they
are about, among the agents in `scope` or among all of them; when it cannot tell, it asks "Who is that for?".

A callsign is the project's folder name plus a number in order of arrival ("shop one"), unless he named the
session himself: then it is the project plus that name ("shop 2" → shop two, "removals" → warehouse removals).
For Claude Code the name is the one given to `/rename` or to `/remote-control <name>`. A session that already
holds a number someone's name asks for moves to the next free one, and its messages move with it.

One transmission is out at a time; questions go before reports; each agent has at most one message waiting
(its latest). A transmission that was taken but never `heard` is offered again.

### Adding a host (Android, Windows, a watch)

A host of the chat implements `DeskHost` from `airband-radio` and draws what `Desk.view()` returns. Ten calls:
`speak` and `listen` (the device's own voice and recognition), `play`, `record_start`, `record_read`,
`record_stop` (raw audio), `changed` (ask for the view again), `status` (for a call screen), `note` (something
he must be told) and `watching` (is the chat in front of him). The desk holds every paired computer as one and
decides what is waiting, what is done, where a message goes and who has the floor on a call. Swift and Kotlin get
the interface generated (`crates/uniffi-bindgen`). A host for one computer and no chat can use `Radio` and
`RadioHost` (eight calls) directly, as `airband radio` does in `crates/airband-cli/src/terminal.rs` (about 260
lines). Hosts decide nothing about order, routing, retries, or what is open.

## `airband.relay/1`: crossing the internet

The tower is behind a home router and cannot be called, so both ends call out to a relay:

    phone  ──POST call──▶  relay  ◀──GET pull───  tower      the phone's request, sealed
    phone  ◀──response───  relay  ◀──POST reply──  tower      the tower's answer, sealed

Pairing gives the phone and the tower one 32-byte secret; the relay never has it. From it both derive the
channel's name, the token that lets only the tower pull from the channel (the relay stores its hash), and
one ChaCha20-Poly1305 key per direction (RFC 8439, a random 96-bit nonce on each message). Inside each sealed request is a `airband.radio/1` call with the time
and a random id: the tower refuses one that is more than two minutes off its clock or that it has answered
before, and serves only `/v1/radio/` to it. The phone refuses an answer that is not to its own request.

Versions: `airband.relay/1` is part of how the keys are derived, so a later version cannot read or be read by this
one. Each sealed request names the radio protocol it is in, and a tower that does not speak it answers 400.
`DELETE /v1/channels/<channel>` with the tower's pull token closes a channel; `airband unpair` sends it.

Not provided: forward secrecy. Recorded relay traffic plus a later-stolen pairing secret can be read;
`airband unpair` and pairing again makes a new secret. Details and tests: `crates/airband-core/src/relay.rs`.

The pairing link (`crates/airband-core/src/pairing.rs`) is the only thing a host stores:
`airband://pair?relay=…&secret=…` or, with no relay, `airband://pair?url=…&token=…`.

## The summarising model

`~/.airband/config.json`, `summarizer`:

```json
{ "kind": "command", "argv": ["claude", "-p", "--model", "haiku", "…"] }
```

Any command that reads a prompt on standard input and writes the answer to standard output, so any model
through any CLI. `{ "kind": "none" }` reads the first two sentences as written. Short plain messages are read
as written without a model either way.
