# Airband: architecture

Airband is a group chat with your agents and a phone line to them. Agents report in when they stop; a dispatcher
decides what is worth your attention and who your reply is for; you read or hear a line or two and answer in a few
words. This page is the map: what each part owns, and where a new thing goes.

## The rule

**Every rule lives in the Rust core. Everything else is an edge.** An edge turns something outside into one of
three versioned protocols, or one of them into something a person can see or hear. Edges decide nothing: not order,
not importance, not who a message is for, not when to retry.

```
 agents ──▶ adapters ──airband.agent/1──▶ ┌────────────┐ ◀──airband.radio/1──▶ radio core ──▶ hosts ──▶ you
 (any harness, any model, any job)        │ the tower  │      (direct, or sealed          (phone, terminal,
 agents ◀── delivery kinds ◀───────────── │ dispatcher │       airband.relay/1)            car, desktop)
                                          └────────────┘
```

## What each part owns

| Part | Where | Owns | Must not |
|---|---|---|---|
| Protocols and rules | `crates/airband-core` | The three protocols' types; callsigns; who a tag or spoken name means; what can be said aloud; pairing links; the relay's sealing | Do any I/O. Everything here is tested directly. |
| The tower | `crates/airband-cli/src/tower/` | What is remembered and the rules about it (`state`), events in (`intake`), importance and wording (`judge`), recipients and controls (`route`), reaching agents (`deliver`), starting new ones (`launch`), call mode's queue (`pump`), copies of pictures (`pictures`), the voice (`speech`), the listening pass (`ears`), the database (`store`), routes (`http`), the relay connection (`relay_link`) | Have a rule that depends on which harness an agent runs in or which client is asking. (It does know each *way of reaching* an agent, one of which is Claude Code's inbox: a delivery kind, in `deliver`.) |
| Adapters | `crates/airband-cli/src/adapters/` | Turning one harness's notifications into `airband.agent/1` events, and setting that harness up to send them (`airband install` asks each adapter) | Hold a harness up, or decide anything |
| The radio | `crates/airband-radio` | The whole client side. `desk`: every paired computer as one, what is waiting and done, where his messages go, who has the floor on a call. Under it, per computer: reaching the tower (`link`), the call loop (`call`), sending over bad connections (`outbox`), what is kept on the device (`store`), sound in and out (`voice`, `ears`) | Draw, make sound or capture it |
| Hosts | `apps/apple`, `crates/airband-cli/src/terminal.rs` | Drawing the view the desk gives, playing and capturing audio, the device's own voice and recognition, keeping pairing links safe | Decide order, routing, retries, or what counts as waiting or done |
| The relay | `crates/airband-relay` | Passing sealed bytes between a radio and its tower; remembering which channels exist | Hold a key or read a message |
| Deployment | `infra/`, `Dockerfile`, `.github/workflows/` | Running the relay | |

## Where a new thing goes

| You want to add | Do this | Touches |
|---|---|---|
| **Another agent tool** (Codex, Gemini CLI, a CI bot, a scheduled job, anything that does work and stops) | No code: call `airband report` from whatever it can run. Or one file in `adapters/` (notification to events; optionally how to install itself into the harness) and one line in `ADAPTERS`. | `adapters/` only |
| **Another agent tool that can be launched** from the phone | An entry in `launchers` in `~/.airband/config.json`: a command with `{cwd}`, `{task}`, `{project}`, `{session}` filled in | Config only |
| **A fact about an agent** the tower has no field for (its model, a branch, a ticket, what kind of agent it is) | Put it in `agent.meta`. The tower keeps and shows it. | Nothing |
| **Another way to reach an agent** (an HTTP callback, a queue, an agent protocol's own channel) | A variant of `Deliver` in `airband-core/src/agent.rs`; its arm in `tower/deliver.rs` | Two files |
| **Something else an agent can report** (progress, a file, a cost) | A variant of `Event`. Old towers skip what they do not know. | `agent.rs`, `tower/intake.rs` |
| **Permission prompts from another agent tool** | No code: `airband report … permission "<what>"` and act on its exit code. Or two functions in its adapter: its prompt to a `PermissionAsk`, and yes or no to whatever the tool reads back. The asking, waiting and rules are `tower/permit.rs` and are shared. | `adapters/` only |
| **Accounts for another agent tool** (several logins, switched while sessions run) | Five functions in its adapter (`AccountBook`: list with allowances, switch, save, add, forget) and one line in `ADAPTERS`. The tower's cache, the routes, the radio core and the app's page are shared and know nothing about the tool. `adapters/claude_accounts.rs` is the model. | `adapters/` only |
| **The models and efforts he can ask a new agent for** ("with fable high, …") | `models`, `efforts`, `with_model` and `with_effort` on that launcher in `~/.airband/config.json`. His words are read by the routing model against whatever the launcher lists (`tower/route.rs`); `routing.rs` keeps the set phrases for typed commands and for when no model answers. | Config only |
| **What an agent is doing right now**, for another agent tool | No code: `airband report … doing "<text>"` as it goes. Or one function in its adapter (`glance`) that reads it from wherever the tool keeps it; the tower asks only while someone is looking. `claude_code::activity` reads the end of the session's transcript. | `adapters/` only |
| **The model and thinking level an agent runs on** | No code: `airband report … model "<name>" [effort]`. Or the same adapter function, `glance`. | `adapters/mod.rs` |
| **What he types to an agent at its own keyboard**, in the chat | One function in its adapter (`told`): his own prompts since a place in the tool's record, each with an id so none is taken twice. | `adapters/mod.rs`, `tower/state.rs` |
| **Which team an agent belongs in** | He moves it from a radio (`POST team`), or the tower does: an adapter function (`touched`) gives the files and commands of its latest steps, and a model says which project that work is part of. | `tower/place.rs` |
| **Running another agent tool from the tower**, so his words are its own turns and it can be stopped | No code: a launcher with `held` in `~/.airband/config.json`: the command that reads a message a line, the arguments for a new and a carried-on session, what a line of input and a stop look like. Each such agent runs under `airband hold`, a small process of its own that keeps it going while the tower restarts. | `config.rs`, `tower/held.rs`, `hold.rs` |
| **Another model, or no model** | Set `summarizer` in `~/.airband/config.json` to any command that reads a prompt and writes an answer. A new *kind* of summariser (a direct API call) is a variant of `Summarizer` and an arm in `tower/judge.rs`. | Config, or two files |
| **A better judgement** of what is worth showing | `tower/judge.rs`. Try it with `airband triage < message.txt` on real messages first. | One file |
| **A better guess** at who a reply is for | `tower/route.rs` for inference; `airband-core/src/routing.rs` for tags and spoken names (pure, with tests) | One of two files |
| **Something else to say to the tower** ("mute shop", "summarise today") | A `Control` in `routing.rs` and its arm in `tower/route.rs` | Two files |
| **Another client** (Android, Windows, a watch, a car) | A host: implement `DeskHost` (ten calls: six for sound, `changed`, `status`, `note`, `watching`) over the generated bindings, draw `Desk.view()`, and keep the links. The Swift one is `apps/apple/Sources/RadioModel.swift`, about 190 lines. `terminal.rs` (about 260 lines) is a single-computer host on `RadioHost`. | A new folder under `apps/` |
| **Another transport** to the tower (a push wake-up, a local socket) | A variant of `Route` in `airband-radio/src/link.rs` | One file |
| **Something else a client can ask** | A route in `tower/http.rs`, its types in `airband-core/src/radio.rs`, a method on `Radio` | Three files |
| **Another place to keep relay data** | A variant of `Store` in `airband-relay/src/store.rs` | One file |

If a change does not fit a row, it is probably a new protocol field. Add the field as optional, default it, and
old peers keep working.

## Sound

Outbound and inbound are set separately on the tower (`airband voice …`, `airband ears …`), and each has a
`device` option that uses the phone's own (Apple's on an iPhone).

| | Interface | Where | Providers |
|---|---|---|---|
| Outbound: text to audio | `Synth` (text in, PCM out a piece at a time) | `tower/speech.rs` | Gemini, Microsoft |
| Inbound: audio to text | a function from (pass, microphone) to text | `airband-radio/src/ears.rs` | Microsoft |
| The device's own voice and recognition | `speak`, `listen` on `RadioHost` | each host | Apple's, on iPhone |
| Raw audio in and out | `play`, `record_start`, `record_read`, `record_stop` on `RadioHost` | each host | |

The tower makes speech the moment a sentence exists and hands it over in parts (`GET speech/<id>`), compressed
four to one (`airband-core/src/audio.rs`); the radio decodes each part and gives the host plain PCM, so playback
starts on the first part and a host needs no codec. For listening the phone goes straight to the service with a
ten-minute pass from the tower, and the rule for when he has finished speaking (`Endpointer`) is in the core, the
same on every platform. Keys never leave the tower.

What adding a provider really touches:

- **A voice**: an `impl Synth` in `tower/speech.rs`, its settings as a variant of `config::Voice`, and its
  `airband voice …` words in `install.rs`. Three files, all in the tower; no app release.
- **A listening service**: the code that talks to it in `airband-radio/src/ears.rs`, an arm in `tower/ears.rs`
  that makes its pass, a variant of `config::Ears`, its `airband ears …` words in `install.rs`, and whatever its
  pass needs in `EarsGrant` (which today has a `region`, because the one service is Microsoft's). Five places in
  three crates, and an app release, because the phone talks to the service itself.
- **A host**: the six sound calls on `DeskHost`.

## Several computers

Each computer runs its own tower and has its own pairing link. A phone pairs with as many as it likes: the app
runs one radio per link and shows them as one inbox, each message labelled with its computer's name (the tower's
`station`, by default the short host name). An answer goes back through the radio its question came from. On a
call only one computer has the floor at a time: its transmission, his answer, its reply; the others wait. Towers
do not talk to each other.

## Pictures

When an agent reads a picture, takes a screenshot or sends him a file, its adapter reports the file's path
(`picture` in `airband.agent/1`). The tower (`tower/pictures.rs`) keeps a small JPEG copy in `~/.airband/pictures`
for 30 days and attaches up to four to the next thing that agent says. The phone gets only ids and sizes with
the message; the radio core fetches a copy when the screen needs it (`Radio.picture`), in parts, and keeps the
file for 14 days. A host draws from that file at the size it shows, so a row of screenshots costs little memory.
Claude Code cannot look at or send video, so there is none.

## Who is who

An agent is known by its id, which its harness gives it and which does not change while it runs. Its callsign is
what it is called: it can change when he names the session, and a later agent can be given a callsign an earlier
one had. So every log line records the id of the agent that said it, and "which agent" is always answered by id:
whether a question is still open, who a reply to a message goes to, what an agent was last told. The room is the
callsign's project. A client gets both (`agent` and `from` on a message, `id` and `callsign` in the roster).

Each radio has its own place in a call on the tower (what it is about to be told, whether it said "stand by",
who it last spoke with), by the name it gives or, through the relay, by its pairing.

A radio follows the chat by change count (`since`), not by position: a line it already has comes again when its
short version is made or its sender is renamed, so every radio ends up with the current form of every line.

## The feed, and what is in it

A host draws `Desk.view()`: one feed of reports, questions and what he said, oldest first; his teams, each in the
state of whichever of its agents most needs him; whether a poke would do anything; and the quick messages to
offer. A question that is still open carries its answers. All of it is worked out in the core, so every host
shows the same thing.

- **Stalled and Poke.** The judge can find that an agent said what it would do next and stopped. `Desk.poke`
  sends "Keep going." to every such agent.
- **Quick messages** are data, not code: the tower serves the list (`airband_core::radio::quick_defaults` and
  `quick` in its configuration), so a new one needs no app release.
- **Subagents** are counted from what an adapter reports and shown beside the team.
- **What each team is doing** comes with the roster (`DeskRoom.doing`, and per agent in `DeskRoom.members`). The
  phone shows it as a small bubble out of the team's chip, one team at a time, and beside each agent in the
  list behind the menu button. It is never part of the feed.
- **Permission.** A harness's permission prompt arrives as a question with Allow and Deny (`tower/permit.rs`).
  The tower never answers one itself, no answer is never a yes, and the card shows the command word for word.
  If no radio is there the harness is told at once and shows its own prompt as before.

## Accounts

Someone with several subscriptions can see how much of each is used and move every running session on a computer
to another one, from the Accounts page (tap the title, then Accounts) or with `airband accounts` on the computer.
It is deliberately not something he can say: a switch moves every session at once, so it takes a tap on the
account. `tower/accounts.rs` keeps the list for five minutes and looks at saved logins every three hours so none
lapses. How a tool's logins are kept and swapped is its adapter's business.

For Claude Code (`adapters/claude_accounts.rs`): saved logins are keychain items of airband's own, the list of
their emails is `~/.airband/claude-code/accounts.json`, and a switch saves the outgoing login, swaps the login
Claude Code reads and names the account in `~/.claude.json`; running sessions pick it up within 30 seconds. How
much is used comes from the question Claude Code's own usage page asks, and costs nothing. macOS only so far.
A login's refresh token changes each time it is used, so airband has to be the only keeper of saved logins on
a computer: another tool holding copies of the same logins would log airband's out, and its own. Under `AIRBAND_CLAUDE_*` the keychain items and Anthropic's addresses can be pointed elsewhere, which is
how `tools/e2e.py` tries all of it against made-up logins.

## The homepage

`site/index.html` is the whole of airband.dev: one static page, served from a private bucket through a CDN
(`AirbandSite` in `infra/cdk.mjs`), deployed by its own workflow so that changing it never restarts the relay.
Its waiting-list form posts an email address to the relay (`POST /v1/waitlist`), which keeps it in the relay's
database. That one route takes JSON from the homepage's origin only, counts sign-ups per caller, per minute and
per day before it touches the database, and writes at most two at a time.

## What is kept

`~/.airband/airband.db` (SQLite) holds the chat log, every agent event exactly as it arrived, a receipt for every
utterance the tower has taken, and every attempt to deliver his words. Intent is written before anything is done
about it: an utterance has a receipt before it is routed, and an event is recorded before it is acted on. That is
what makes a retry safe across a restart, and the raw events are what the judge and the adapters should be tested
against.

## Compatibility

All three protocols follow one rule: unknown fields are ignored, and unknown `type` and `kind` values are accepted
and skipped. Adding is never a new version. `/2` is for a change an old peer would misread. There is a test for
this (`round_trips_and_tolerates_the_future`).

## Names

Airband is the product and the channel. *The tower* is the daemon on your computer that dispatches on it. A
*radio* is a client. An *agent* is anything that does work on your behalf and reports when it stops; the code also
calls one a *session*.

A *room* is one project: every agent working in the same folder. Rooms and callsigns make themselves from where
agents run and what he named their sessions; nothing is set up by hand.

## For contributors

- A rule goes in `airband-core` if it needs no I/O, and gets a test there.
- A host that finds itself deciding something has found a missing function in the radio core.
- The tower has no `if harness == …` anywhere. Keep it that way.
- Messages from agents are untrusted input to the model that judges them. That model is given no tools.
- `tools/e2e.py` starts a throwaway tower and goes through everything a radio can do, with two agents in one
  project and agents in separate projects; `--claude` adds a real Claude Code session asking permission. Run it
  before a release. It makes real model calls, so it is not in CI.
- Say what was run and what was only compiled. `docs/state.md` is the record; keep it true.
