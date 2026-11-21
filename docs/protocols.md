# Protocols

Three versioned protocols: `airband.agent/1` (agents → tower), `airband.radio/1` (phone ↔ tower) and
`airband.relay/1` (how the second one travels sealed). The first two are plain JSON over HTTP with
`Authorization: Bearer <token>` when sent to the tower's own port. The types are in `crates/airband-core`
and are the source of truth; this page explains them.

Compatibility: unknown fields are ignored, and unknown `type` / `kind` values are accepted and skipped. Adding
a field or a variant is not a new version. `/2` is for a change an old peer would misread.

Two tokens open the tower's port, which listens on 127.0.0.1 and the computer's Tailscale addresses. Its own
(`token` in `~/.airband/config.json`) opens everything. A radio's (`radio_token`), which is what a direct
pairing link carries, opens `/v1/radio/` only.

## `airband.agent/1`: agents → tower

`POST /v1/agent/events`, one document per event:

```json
{
  "schema": "airband.agent/1",
  "agent": {
    "id": "8f2c…",              // stable for the agent's session, unique on this machine
    "harness": "claude-code",   // free text; the tower has no rules per harness
    "meta": { "branch": "…" },  // anything else worth keeping about the agent; the tower stores and shows it
    "cwd": "/Users/c/shop",     // its last part names the callsign ("shop two")…
    "name": "shop",             // …unless a spoken name is given
    "title": "shop 2",          // the name he gave this session in the harness, if he did: it becomes the callsign
    "pid": 5302,                // when this process dies the agent is dropped
    "deliver": { "kind": "socket", "path": "/tmp/cc-socks/5302.sock", "token": "…" }
  },
  "event": { "type": "said", "text": "…the agent's whole last message…" }
}
```

| `event.type` | Meaning |
|---|---|
| `checkin` | The agent exists. Every other event also checks it in. |
| `working` | It was given something to do; what it last said is stale and is dropped unheard. |
| `said` | It finished a turn; `text` is what it wrote. Optional `kind`: `question` (blocked until answered) or `report`; left out, the tower works it out. |
| `notice` | It needs him at the keyboard (a login, a prompt the radio cannot answer). |
| `picture` | A picture file on the tower's machine (`path`: PNG, JPEG, GIF or WebP). The tower copies it only with `shown: true`, which means the agent sent it to him, and attaches the copy to the next `said`. A picture file named in the text of a `said` is attached too, when it is in the agent's `cwd` or a temporary folder. |
| `subagent` | It started a subagent (`done: false`) or one finished (`done: true`); `what` says what it is for. Only counted. |
| `activity` | What it is doing right now, in a few words (`text`). Shown small while it works, never kept, never read aloud. |
| `model` | The model it runs on and how hard it is set to think (`name`, `effort`), as a person would say them: "Opus 4.7", "high". |
| `stopped` | Its turn is over and it wrote nothing: it was stopped, or had nothing to say. It is waiting from now, as after a `said`. With `why`, the turn failed, and he is told in those words. |
| `signoff` | It is gone. |

| `deliver.kind` | How the tower gets his reply to the agent |
|---|---|
| `socket` | A socket at `path` on the tower's machine that takes a turn as one line of JSON, after a line showing `token` when it asks for one. Claude Code's per-session inbox is one. |
| `command` | Runs `argv` with the message on standard input and `AIRBAND_AGENT_ID` in the environment. |
| `keys` | Types the message into the terminal pane `tmux` and presses Enter. Works for anything in a terminal. |
| `held` | The tower runs this agent and holds its input: his words are written there as his own turn. |
| `harness` | Through the agent's own tool, by `name`, into its `session`: whatever way that tool takes a turn of his into a session already open (Codex's queue, OpenCode's server). The tower's adapter for the tool knows how. |
| none | The agent can be heard but not answered. |

`DELETE /v1/agent/held/<id>` ends an agent the tower runs.

### Asking permission

A harness that stops to ask before its agent does something (run a command, write outside the project) can put
the question to him on the radio.

| Call | Body → answer | Meaning |
|---|---|---|
| `POST /v1/agent/permission` | `{ schema, agent, what, detail, question }` → `PermissionState` | `what` finishes the words "wants to …" and is shown to him exactly as given: "run \`npm run deploy\`". `detail` is the whole request, for the more-info page. `question`, `{ text, choices }`, is set when the agent is asking him something through its harness's own asking tool: it is shown as that question with its choices, and whatever he answers is the reply. |
| `GET /v1/agent/permission/<id>?wait=<s>` | → `PermissionState` | Held open up to `wait` seconds (25 at most) or until he answers. |
| `DELETE /v1/agent/permission/<id>` | | The harness has stopped waiting. The card closes. |

`PermissionState` is `{ id, state, message, wait_seconds }`:

| `state` | The harness should |
|---|---|
| `pending` | Ask again, for `wait_seconds` in all, then withdraw the request and ask in its own way. |
| `allow` | Go ahead. |
| `deny` | Refuse. `message` is his own words if he gave any; pass them to the agent. |
| `nobody` | Ask in its own way now: no radio has read the feed in the last 90 seconds, or `permission_wait` is 0. |
| `gone` | The same: the tower no longer knows the request. |

The rules are the tower's and hold for every harness. Only he can turn `pending` into `allow`, and waiting
never becomes a yes. Only an utterance whose `re` is the request answers it; a "yes" said to anyone else
answers nothing. The Allow and Deny buttons send the texts `Allow` and `Deny`
(`airband_core::radio::ALLOW`, `DENY`), which the tower takes without reading. Any other words are read by a
model: a plain yes to exactly what was asked allows it; a no, a condition or another instruction refuses it and
goes back as `message`; words that settle nothing, or no answer from the model, leave it `pending` and he is
told it needs a yes or a no. The message for a request carries `permission: true`, and a client offers its two
answers only.

### Putting another harness on the radio

- No code: `airband report --id "$SESSION" --harness codex --tmux "$TMUX_PANE" said "$LAST_MESSAGE"`, from
  whatever the harness can run. `… permission "run \`$COMMAND\`"` waits for him and exits 0 for yes, 1 for no
  (his reason on standard output), 2 for no answer.
- An adapter: a module in `crates/airband-cli/src/adapters/`, selected as `airband hook <harness>`.

A hook that runs with `AIRBAND_HELD_ID` set belongs to an agent the tower runs: `airband hook` reports it under
that id with `deliver.kind` `held`.

## `airband.radio/1`: radio ↔ tower

The radio always asks; the tower never calls the phone. All under `/v1/radio/`. A radio names itself in the
`x-airband-radio` header so the tower can keep its place in a call apart from another radio's; for a sealed
request the tower uses the pairing's name.

Call mode, one spoken transmission at a time:

| Call | Body → answer | Meaning |
|---|---|---|
| `GET next` | → `Transmission`, or 204 | Held open up to 25 s. Answered only for a radio that says it is on a call (see `POST presence`). With `?voice=own` the radio has a voice of its own: the tower makes no speech for it and sends words only. `joined=1` on the first request after he turns reading aloud on: what was said more than ten minutes before is left for the chat. `have=<id>` is the last transmission the radio got: if the tower handed it another since, in an answer that never arrived, it hands that one over again. |
| `POST heard` | `{ id }` | That transmission was spoken to the end. Only now is it off the queue. |
| `POST unheard` | `{ id, why }` | It was handed over and no sound came of it. It goes back in the queue, and that radio is handed nothing for a minute. |
| `POST presence` | `Presence` `{ on_call, showing_chat }` | Where the radio stands: reading aloud on or off, the chat in front of him or not. Sent when either changes, and every quarter of a minute while the radio has nothing else to ask (the microphone is open). Every other request says the same: in the sealed request's `presence`, or for a radio paired directly in the header `x-airband-presence`. The tower infers none of it from which requests a radio makes. A radio it has not heard from for 60 s is gone. |
| `POST utterance` | `Utterance` → `Outcome` | What he said. Chat mode sends through it too. Answered at once: `{ state: "taken" }`. See "What he says". |
| `POST ready` | | Free for the next transmission. Something it took and said nothing about goes back in the queue. |

```json
{ "id": "9a41…", "callsign": "shop two", "kind": "needs-you", "say": "Shop two. The checker passes…", "waiting": 2, "speech": "…" }
```

One transmission is out at a time; questions go before reports; each agent has at most one message waiting
(its latest). A transmission that was taken but never `heard` is offered again.

An `Utterance` is `{ text, id, re, scope, label, attachments, others }`. `id` is random and the same on every
retry: the tower takes each id once. `re` is the message he is answering, when he picked one. `scope` is the
room he was in (`group:<name>`). `label` is the button's words when it is a quick message. `attachments` are
ids of files sent with `POST attachment`. `others` are the agents on his screen that another of his computers
has, each `{ callsign, group, said, ago }`.

A `Reply` is `{ say, done, listen, speech, ask, elsewhere }`. `done` says that what he asked was done whole and
`say` only confirms it: a radio with a screen already shows his words in the chat, so it shows `say` only when
`done` is not set. `say` is spoken back and `listen` asks for the
microphone again. `ask` lists the agents his words could be for, likeliest first, when the tower could not
tell. `elsewhere` is the callsign of one of `others`: the radio sends his words to that computer.

Chat mode and the rest:

| Call | Body → answer | Meaning |
|---|---|---|
| `GET feed?scope=…&since=<rev>&wait=<s>` | → `Feed` | Every line new or changed after change count `since`, oldest change first; pass `rev` back next time. `since=0` gets the newest lines. `scope`: `all`, `group:<name>`, `agent:<callsign>`. Held open up to 25 s. `roster=<roster_rev>`: also answer when the roster changes. `answers` holds what came of things this radio said (see "What he says"); one arriving ends the wait too. `after=<seq>` is the older form (new lines only) and is still answered. |
| `GET message/<id>?form=detail\|full\|speech\|spoken` | `{ text }`, or `{ speech }` | The fuller spoken version, exactly what the agent wrote, the id of a clip of the whole message read aloud, or (`spoken`) the words of that clip for a radio that reads them in a voice of its own. |
| `GET speech/<id>?from=<block>` | `{ rate, data, samples, next, done, text, marks }` | The next part of a clip of the tower's voice: IMA ADPCM blocks, base64. `text` comes with the first part; `marks` are pairs of (where a word starts in `text`, milliseconds into the clip) when the voice gives timings. 404 when the tower has no voice. |
| `GET ears` | `{ by: "live", dialect, token, set, phrases }`, `{ by: "tower" }` or `{ by: "none", phrases }` | What the phone should listen with. `live`: the phone speaks to the service itself in `dialect`, with `token` (a short-lived pass) and `set` (what that service has to be told: a region, a language, a model). `tower`: the phone records and sends the recording to `POST hear`. `none`: nothing is set to listen here. `phrases` are names he is likely to say. |
| `POST hear` | `{ clip, data, samples, last }` → `{ text }` | Part of a recording, for a tower whose listening service takes a finished clip: 16 kHz mono in the same compressed blocks speech comes in, ten seconds a part. The last part is answered with the words. |
| `GET picture/<id>?from=<byte>` | `{ data, next, done }` | One part (120 KB at most, base64) of a picture's kept copy. |
| `POST attachment` | `{ id, name, from, data, total }` → `{ next }`, or `{ error }` | One part of a file he is attaching, 120 KB at most. `total` is how long the whole file is; the parts may come in any order, several at once, and `next` says how much of the file is whole from its start. |
| `POST dismiss` | `{ message }` → `{}`, or `{ error }` | He does not mean to answer this question. It stops being open. The agent is told nothing. |
| `POST unsend` | `{ message }` → `{}`, or `{ error }` | Take back something he said that is still `queued`. |
| `POST taken` | `{ callsigns }` → `{}` | The names agents go by on his other computers, as his radio shows them ("shop 1"). The tower moves any of its own agents with one of those numbered names to the next free number. A radio tells only the later of two computers. |
| `POST team` | `{ agent, team }` → `{ callsign, team }`, or `{ error }` | Put an agent in another team, one that exists or a new one. A name he gave the agent goes with it; a number is replaced by the next free one there. |
| `POST folders` | `{ under }` → `{ path, shown, up, folders }`, or `{ error }` | The folders inside one of his on this computer, by name and without the hidden ones, for choosing where a new team works. `under` empty is his home folder; nothing outside it is listed. |
| `POST project` | `{ name, folder, make }` → `{ say }`, or `{ error }` | Make a new team: one plain word for its name, and the folder its agents work in (inside his home folder; made first with `make`). A tower that runs agents itself starts the team's first one there with nothing to do yet. |
| `POST takeover` | `{ agent }` → `{}`, or `{ error }` | Have the tower take over an agent he started in a terminal: that session ends there and carries on under a held launcher of the agent's own harness. Refused while it is in the middle of something. |
| `GET reads` | `{ now: { provider, offer }, offers: [{ provider, offer, label, about, price, preferred, free, login, key, has_key, key_name, keys_at, asks }] }` | What can read the agents' messages on this tower, from the tower's own catalogue and in the order to show it, and which one reads now (`now` is absent when none does). `has_key` says only that the tower keeps a key for that provider; one key serves all a provider's offers. |
| `POST reads` | `{ kind, offer, key, set }` → `{ reads }`, or `{ error }` | Choose what reads, by a `provider` (as `kind`) and `offer` from `GET reads`. `key` empty uses one the tower keeps. The tower tries it with a real call before keeping it. A radio cannot name a command to run. |
| `POST stop` | `{ agent }` → `{}`, or `{ error }` | Stop what an agent the tower runs is doing. What he said to it meanwhile it then reads as one turn. |
| `POST end` | `{ agent }` → `{}`, or `{ error }` | End an agent the tower runs: its input is closed, and it finishes and signs off. |
| `GET accounts?fresh=1` | → `{ schema, accounts, note, checked_at }` | Every account of every harness whose sessions can be moved between logins. Kept for five minutes; `fresh` checks again. |
| `POST accounts/use` | `{ harness, id }` → the same, or `{ error }` | Move that harness's sessions to that account. |
| `POST push` | `{ provider, token }` → `{}` | Where this radio can be woken: its address at a push service (`apns` is Apple's). An empty `token` takes it away. |
| `POST tools` | `{ tools }` → `{}` | The coding tools he uses, by the ids their rows give, the one he uses most first. A new agent asked for without naming a tool is started with the first; with none said and no usual launcher set, he is asked. |
| `POST banner` | `{ provider, which, token }` → `{}` | Where this radio's banner can be changed while the app is asleep. `which` is `start` (the address that puts one up) or `update` (the address of the one that is up). An empty `token` takes it away. |

### The feed

A `Feed` is `{ schema, rev, messages, roster, roster_rev, seq, open, projects, quick, colours, station, direct, listens }`.

- `open`: the id of every message that needs him and is unanswered, whether or not it is in `messages`.
- `projects`: the names a new agent can be launched in.
- `quick`: the quick messages to offer, each `{ label, text }`: six standard ones, then his own
  (`airband quick add`). Sending one is an utterance with that `text` and `label`; the chat shows the label.
- `colours`: for each team, its place in the client's own list of team colours. Given once and kept.
- `station`: what this computer is called.
- `direct`: addresses where a paired radio may reach this tower without the relay, `http://<address>:<port>`:
  its local network addresses when `airband lan` is on, and its Tailscale addresses (see The sealed route).
- `listens`: this tower has a key of its own for a listening service. Never the key.

A message is `{ seq, id, at, from, group, to, kind, text, long, options, recommended, read, own_session, queued,
seen, cancelled, permission, pictures, links, agent, rev, open }`.

- `agent` is the id of the agent that said it, the same as `id` in the roster; `from` is only what it is called
  now. A message from `you` is something he said: through a radio, or typed to an agent at its own keyboard
  (with any pictures he pasted), which the tower adds with the time he typed it. So a radio orders messages by
  `at`.
- `kind` is `needs-you`, `report`, `fyi` or `stalled`; a client shows a kind it does not know as a report.
  `stalled` means the agent said what it would do next and stopped, with nothing asked.
- `text` is one sentence, or up to about 90 words when the message answers something he asked to have
  investigated or leaves him several things to do. The news comes first and a question last, and what was
  shipped keeps its number. A note on the way from an agent long at work is an `fyi` message. Keys and tokens
  are masked. `long`: `message/<id>` has more.
- `options` are answers he can tap, up to three from the judge or four from a harness's own question.
  `recommended` is the position of the one the agent itself recommended, never a default.
- `read`: two or three words for something in the message he should be able to read whole ("the refund
  email"). A client offers the full message under that name.
- `own_session`: the agent said it takes this answer only from him in its own session, and not from a message
  passed on to it. The reading model decides this from the message. For an agent the tower does not run, what
  a radio sends is such a message, so a client says so and offers to take the agent over; after that his words
  reach it as his own. No `options` come with it.
- `queued`: he said it to an agent the tower runs, which is busy; it waits until the agent stops. `seen` is
  when the agent took it in. `cancelled`: he took it back.
- `links`: up to four web links the agent wrote, which the short text leaves out.
- `pictures`: up to four of `{ id, width, height, bytes }`. A copy is a JPEG, 1600 pixels at most on its long
  side. Nothing is sent until a client asks for it.
- `open`: it needs him and nothing has answered it. A question stays open until he says something to the agent
  that asked, he dismisses it, the agent asks a newer one or speaks past it three times, or six hours pass. A
  permission request is open while its harness waits.

A roster entry is `{ id, callsign, group, harness, waiting, state, subagents, reachable, doing, held, model,
effort }`. `state` is `working`, `asking`, `stalled` or `idle`. `doing` is what a working agent is doing right
now, asked of its adapter every two seconds while a radio is reading the feed. `held` marks an agent the tower
runs.

### The reading queue

There is no separate list of what is unheard. Each line an agent says carries where it stands with being read
out (`aloud` in the `messages` table): `waiting`, `handed` to a radio, `heard` on a radio, or `dropped` with a
reason: `routine` (the judge found nothing for his ears), `superseded` (its agent said something newer),
`answered` (it was waiting on him and he spoke to the agent, or it went back to work), `dismissed`,
`agent_gone`, `stale` (said long before he turned reading aloud on), `permission_ended`. One function changes
it (`Inner::set_aloud`, `tower/state.rs`), and every change is a line in the tower's log:
`aloud: shop two (9a41…) waiting → dropped (superseded)`. `airband status` and `GET /v1/status` list what is
unheard and the last twenty changes. A report whose agent has gone is still read out; a question it was
waiting on is not.

### What he says

An `Utterance` carries an `id` that is the same on every retry, and one without is refused. The tower writes
a receipt for it and answers at once, `{ "state": "taken" }`: it has it. No request waits while a model works
out who the words are for. What came of it is told three ways, all the same `Outcome`:

- in the feed of the radio that said it: `answers: [{ id, state: "answered", reply }]`, which also ends a
  waiting `GET feed`;
- to the same utterance sent again: `{ state: "taken" }` while it is being answered, then
  `{ state: "answered", reply }` for good, from the receipt, across restarts;
- for something the relay kept and the tower would not take when it came back: `{ id, state: "rejected", why }`
  in the feed (`why`: `too_old`, left more than a week before).

A radio keeps what he typed in its outbox until it has the outcome. It sends the next only then, so his words
reach his agents in the order he said them. If no outcome comes by the feed it asks again every quarter of a
minute. A receipt with no answer that this tower is not answering was left by a tower that stopped part way:
the words are taken up again, unless the record of deliveries shows they already reached their agents.

A message of an agent's that was to be read out while he had reading aloud on, and was not, says so in the feed
(`not_read`: `superseded`, `answered`, `dismissed`, `agent_gone`, `permission_ended`). A radio marks the line
and shows the reason when he asks.

Nothing is decided by matching his words.

- A leading tag or callsign names who it is for: typed tags (`@shop2`, `@shop` for the group, `@all`), or a
  spoken callsign ("shop two, …", "all shop, …", "everyone, …").
- With `re`, it goes to the agent that said that message.
- Otherwise a model reads his words against the last thing each agent said, among the agents in `scope` or all
  of them, and against the projects, tools, models and efforts a new agent could be started with. It picks an
  agent, or says he wants a new one: "with fable high, find out why checkout is slow". Only a project the tower
  knows and names a launcher lists are taken from its answer. When it is not sure, the tower asks "Who is that
  for?" and gives the candidates in `ask`.
- When no model answers: with one agent running it is for that one, with several he is asked.

Tower commands come only from a radio's buttons, as a word behind a slash and nothing after it: `/poke`,
`/more`, `/repeat`, `/status`, `/skip`, `/hold`, `/resume` (`COMMANDS` in `routing.rs`). `/poke` sends "Keep
going." to every stalled agent, within `scope` if one is given. `/launch <project> [with <model> [<effort>]:]
<task>` starts a new agent. The same words said or typed without the slash are a message for an agent.

A callsign is the project's folder name plus a number in order of arrival ("shop one"), unless he named the
session himself: then it is the project plus that name ("shop 2" → shop two, "removals" → warehouse removals).
For Claude Code the name is the one given to `/rename` or to `/remote-control <name>`. A session that holds a
number someone's name asks for moves to the next free one.

An account is `{ id, harness, label, live, limits, problem }`. `label` is what to show (for Claude Code, the
email); `live` marks the one in use. Each of `limits` is `{ name, used, resets_at }`: the window ("week",
"5 hours"), from 0 to 1, and milliseconds since 1970. `problem` says why the account could not be checked.
Nothing that could sign in as an account is ever sent.

### Listening

`dialect` in the answer to `GET ears` names a live dialect: a line of `LISTENERS` in `crates/airband-radio/src/ears.rs`, which is also the `by.interface` of the provider's row. The radio
streams the microphone to that service as he speaks, and falls back to one upload if the connection cannot be
made. With a key of his own in the phone, the radio gets its pass from the service itself and asks neither the
tower nor the relay.

## `airband.relay/1`: sealed requests

The tower is behind a home router and cannot be called from outside, so both ends call out to a relay:

    phone  ──POST call──▶  relay  ◀──GET pull───  tower      the phone's request, sealed
    phone  ◀──response───  relay  ◀──POST reply──  tower      the tower's answer, sealed

Pairing gives the phone and the tower one 32-byte secret; the relay never has it. From it both derive, with
HKDF-SHA256, the channel's name, the pull token that lets only the tower pull from the channel, the radio token
that lets only the radio send to it (`Keys::radio_token`), and one ChaCha20-Poly1305 key per direction (RFC
8439, a random 96-bit nonce on each message). The relay stores the hash of each token.

Inside each sealed request is `{ t, id, method, path, body, schema }`: an `airband.radio/1` call with the time
and a random id. The tower answers 408 to one more than two minutes off its clock, drops one it has answered
before, answers 400 to a `schema` it does not speak, and serves only `/v1/radio/`. The answer is
`{ id, status, body }`, and the phone refuses one that is not to its own request.

`airband.relay/1` is part of how the keys are derived, so a later version cannot read or be read by this one.

### The sealed route

`POST /v1/sealed/<channel>` on the tower takes the same sealed request and answers with the sealed answer:
404 for a channel it does not serve, 400 for a request the pairing's key does not open or that it has answered
before. The tower serves it on every address it listens on. On its local network addresses (private IPv4
ranges, when `airband lan` is on) it serves nothing else.

A radio paired through a relay tries the addresses in the feed's `direct` first and uses the relay when none
answers. Something he said goes by the relay first, so it keeps its place behind anything the relay holds, and
straight only when the relay cannot be reached.

### The relay's routes

Every request on a channel carries a token, `Authorization: Bearer <token>`, and the relay compares its SHA-256
with the hash it holds, in constant time.

| Who | Routes | Token |
|---|---|---|
| The tower | `PUT /v1/channels/<channel>` (open), `GET …/pull` (held 25 s), `POST …/reply/<id>`, `POST …/push`, `DELETE` (close; `airband unpair` sends it) | The pull token. Opening a channel the relay does not know also takes the relay's admission key (`x-airband-admission`), if it has one. 403 with another channel's token. |
| The radio | `POST …/call`, `POST …/queue` | The radio token. 401 without it, with a wrong one, or on a channel that does not exist: the same answer for all three. |

The tower gives the relay the radio token's hash in `x-airband-radio-hash` when it opens the channel; a channel
is not opened without it, and nobody changes it afterwards.

The relay has no speech service and lends none: what speaks and what listens is a model on the phone, a key
of his own on the phone, or a service set on his computer.

Limits, each answered with 429. Per channel in a minute: 3000 calls and 60 messages left in `queue`;
from one internet address, twice that across all channels. At most 16 calls wait for an answer on a
channel at once. No message is larger than 256 KB.

### Notifications

A radio that has given the tower an address (`POST push`) is woken for a message judged `needs-you` or
`report`, a permission card, or a question a harness puts to him. Once per message, and not on a radio that is
on a call or showing the chat: such a radio asks for what follows a message within a moment of getting it, and
one that follows without showing it says so (`showing_chat: false` in its `Presence`).

The notification is `{ id, from, group, colour, station, text, kind, options, permission }` (`Notice` in
`radio.rs`). The tower seals it with a third key from the pairing secret (direction `push`) and asks the relay
to pass it on:

| Call | Body → answer | Meaning |
|---|---|---|
| `POST /v1/channels/<channel>/push` | `{ provider, to, sealed, question }` → `{ sent, gone, reason }` | `to` is the phone's address, `sealed` the notification in base64 (2400 bytes at most before base64), `question` whether it is one. 200 sent; 410 the address is no longer good and the tower forgets it; 429 more than 60 from this channel in ten minutes, or 240 from one internet address; 503 this relay has no such push service; 502 the service refused or could not be reached. |

The relay holds the push service's key and adds the same visible words to every notification ("airband", "New
message" or "A question for you"), which is what shows if the phone cannot open it. The relay and the push
service learn only that a notification was sent, when, to which phone, and whether it was a question. The phone
opens it with `open_notice` in the radio core. An answer given on the notification is an ordinary
`POST utterance` with `re` set to the notification's `id`.

### The banner while the app is asleep

A push service delivers a banner (a Live Activity) as it is given it. It cannot be sealed, so it is never given
anything of his work: the tower sends three numbers and the relay builds the rest.

| Call | Body → answer | Meaning |
|---|---|---|
| `POST /v1/channels/<channel>/banner` | `{ provider, to, event, counts: { waiting, working, unread }, urgent }` → `{ sent, gone, reason }` | `event` is `start`, `update` or `end`; `to` is the address for that (`POST banner`). `urgent` lights the screen (more agents are waiting than before). The statuses are a notification's; 429 is more than 90 from this channel in ten minutes, or 360 from one internet address. |

`Counts` (`core/relay.rs`) is how many agents are waiting for him, how many are working and how many reports
he has not seen. It can hold no text: a test fails to compile if a field that could is added, and a relay test
checks that everything Apple is handed is those numbers and the words written in `relay/push.rs` ("airband",
"An agent needs you"). A banner also carries one number that says which of his computers it is for, taken from
the channel's name (`banner_slot`).

A banner is up while an agent is waiting for him and he is not in the app. The tower puts it up, changes it
no more often than every 20 seconds and 40 times an hour, and takes it down when nobody is waiting. The app
takes it down when he opens it. With no relay there is no banner.

### When the tower is away

A relay that has not heard from a tower for 45 seconds answers a call with 503. What he said is then left with
the relay: `POST …/queue`, the same sealed request, with the utterance's own id in `x-airband-id`. The relay
keeps at most 200 per channel for a week, and one left twice under the same id is kept once. When the tower
pulls again it is handed them oldest first, one at a time, each under a request id that starts with `held-`;
its reply takes that one off the relay. The tower takes a kept request only if it is an utterance with an id,
and tells the radio in its feed when it will not take one (`rejected`). The radio shows a kept message as with
the relay until the tower's outcome for it arrives.
While some are still kept, the relay turns back a new utterance (409 to a call that carries
`x-airband-ordered`) and the radio leaves that one with the relay too, so everything arrives in order. Files
he attaches wait on the phone.

### What is written down, and where it stays

Nothing is uploaded: no log, no trail and no diagnostic leaves the device it was made on.

- **The phone** keeps, in a file of the app's own beside the saved chat, the last 300 requests that failed and
  every try at sending something he said: the time, the kind of request (`POST /v1/radio/utterance`, with nothing after a `?`), the way it went
  (relay, own network, kept at the relay), how it ended and how long it took. Each failure is one named cause
  (`LinkError` in `radio/link.rs`): no connection, timed out, the relay says the computer is not connected
  (503), earlier messages still on their way (409), the computer took it and has not answered (504), another
  relay status, the relay would not keep it, an answer that cannot be read, or the computer's own refusal with
  its explanation. What the tower is told in passing (he heard this, he saw that) and did not get is counted. A message that has not gone shows how its last try
  went, each way, under its bubble; one that cannot go, or that half an hour did not send, stays in the chat
  with the reason and Send again. Settings, Connection shows how each computer was last reached and copies
  the record to the clipboard. Nothing he or an agent said is in it.
- **The tower** writes to `~/.airband/airband.log` on his computer: a line for everything he says (its id,
  how it came, what became of it, how long it took), a line for each request it refuses and each answer that
  does not reach the relay, and every change to the reading queue.
- **The relay** writes one line for a radio's request that fails, from what it handled itself: the route
  (`call` or `queue`), the status, the time taken, the size of what was sent, how many requests were in flight
  on the channel, whether messages are kept for its tower and whether the tower has been pulling, and the
  first six characters of the channel's name. It cannot write what was sent, because that is sealed and it has
  no key. It writes no token and no internet address.

One thing passes through Apple unsealed, when a relay is in use and an agent is waiting while the app is
asleep: the three numbers on the banner (how many agents are waiting, working, and how many reports are
unread). The relay passes them on and neither keeps nor logs them; the tower's log says that a banner was
changed, not the numbers.

### What it does not give

Forward secrecy. Recorded traffic plus a later-stolen pairing secret can be read; `airband unpair` and pairing
again makes a new secret. Details and tests: `crates/airband-core/src/relay.rs`.

The pairing link (`crates/airband-core/src/pairing.rs`) is `airband://pair?relay=…&secret=…` or, with no
relay, `airband://pair?url=…&token=…`.

## The model that reads everything

One model does the judging, the routing, the notes on the way, and reading his answer to a permission request.
It is given no tools. `airband setup` or `airband model` chooses it; it is `summarizer` in
`~/.airband/config.json`:

```json
{ "kind": "command", "offer": "own", "argv": ["claude", "-p", "--model", "haiku", "…"] }
{ "kind": "gemini", "offer": "flash-lite", "key": "key:gemini", "model": "gemini-flash-latest" }
{ "kind": "openai", "offer": "luna", "url": "https://api.openai.com/v1", "key": "key:openai", "model": "…" }
{ "kind": "none" }
```

`kind` is a provider's id and `offer` is which of its offers (a provider can do one job more than one way).
The key is the provider's: one key serves all its offers.

`command` is any program that reads a prompt on standard input and writes the answer to standard output.
`openai` is any service that speaks OpenAI's chat API, a model on his own computer included. `key` is written
as the name of a kept key (`airband keys`).

When it does not answer he is told why on his phone (`reads_failed` in the feed, `failed` in `GET reads`), and
no other model is asked. Then, or when the kind is `none`, nothing is worked out from the words: he gets each message as
written, marked a report unless its harness said what it is, and his words go to whoever he names.
