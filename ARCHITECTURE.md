# Airband: architecture

Airband is a group chat with your agents and a phone line to them. Agents report in when they stop; a dispatcher
decides what is worth your attention and who your reply is for; you read or hear a line or two and answer in a few
words. This page is the map: what each part owns, and where a new thing goes.

## The rules

Every rule lives in the Rust core. Everything else is an edge. An edge turns something outside into one of
three versioned protocols, or one of them into something a person can see or hear. Edges decide nothing: not order,
not importance, not who a message is for, not when to retry.

Nothing is decided by matching his words. A model reads what he says; when no model answers, nothing is guessed.
The only things read by rule are what a radio's own buttons send (a word behind a slash, Allow, Deny), tags and
callsigns that name an agent, and the shape of a credential.

```
 agents ──▶ adapters ──airband.agent/1──▶ ┌────────────┐ ◀──airband.radio/1──▶ radio core ──▶ hosts ──▶ you
 (any harness, any model, any job)        │ the tower  │      (sealed: straight, or        (phone, terminal,
 agents ◀── delivery kinds ◀───────────── │ dispatcher │       through the relay)           car, desktop)
                                          └────────────┘
```

## What each part owns

| Part | Where | Owns | Must not |
|---|---|---|---|
| Protocols and rules | `crates/airband-core` | The three protocols' types; callsigns; who a tag or spoken name means; the words a radio's buttons send; what can be said aloud; masking credentials (`redact`); pairing links; the relay's sealing | Do any I/O. Everything here is tested directly. |
| The tower | `crates/airband-cli/src/tower/` | What is remembered and the rules about it (`state`), events in (`intake`), importance and wording (`judge`), recipients and controls (`route`), permission (`permit`), reaching agents (`deliver`), starting new ones (`launch`), running them itself (`held`), which team one belongs in (`place`), call mode's queue (`pump`), copies of pictures (`pictures`), files he attaches (`attachments`), the voice (`speech`), the listening pass (`ears`), accounts (`accounts`), the database (`store`), routes (`http`), the relay connection and sealed requests (`relay_link`), waking his phone (`push`) | Have a rule that depends on which harness an agent runs in or which client is asking |
| Adapters | `crates/airband-cli/src/adapters/` | Turning one harness's notifications into `airband.agent/1` events, and setting that harness up to send them (`airband install` asks each adapter) | Hold a harness up, or decide anything |
| The radio | `crates/airband-radio` | The whole client side. `desk`: every paired computer as one, what is waiting and done, where his messages go, who has the floor on a call. Under it, per computer: reaching the tower (`link`), the call loop (`call`), sending over bad connections (`outbox`), what is kept on the device (`store`), sound in and out (`voice`, `ears`) | Draw, make sound or capture it |
| Hosts | `apps/apple`, `crates/airband-cli/src/terminal.rs` | Drawing the view the desk gives, playing and capturing audio, keeping pairing links and a speech key of his own safe | Decide order, routing, retries, or what counts as waiting or done |
| The relay | `crates/airband-relay` | Passing sealed bytes between a radio and its tower; remembering which channels exist; handing a sealed notification to a push service; lending a speech pass; the homepage's waiting list | Hold a pairing key or read a message |
| Deployment | `infra/`, `.github/workflows/` | Running the relay: one small server in us-east-2 with DynamoDB behind it (`AirbandRelayUs` in `infra/cdk.mjs`, what runs on the server in `infra/host/`), deployed by `deploy-relay.yml` on a push that changes the relay | |

## Where a new thing goes

| You want to add | Do this | Touches |
|---|---|---|
| Another agent tool (a CI bot, anything that does work and stops) | No code: call `airband report` from whatever it can run. | Nothing |
| Another coding tool with an adapter of its own | One file in `adapters/` that implements `Harness` and one line in `HARNESSES`. It says only what the tool has: its hook events (the shared installer in `adapters/hooks.rs` puts them in the tool's file), or how to watch its server, and how his words get into one of its sessions. A request for leave goes through `hooks::leave`, which owns the length rules for every tool. Its row in `catalog/tools/` says what it is called and how it is started. | `adapters/`, one row |
| A coding tool that speaks the Agent Client Protocol, started from the phone | One row in `catalog/tools/`: `{ id, name, maker, launchers: [{ name, held: { by: "acp", program: […] } }] }`. No code: the holder's ACP driver (`acp.rs`) runs it and reports for it. The rows are compiled in; a catalogue fetched later can rename a tool and never change or add a program. | One row |
| A launcher of his own (another way to start a tool, or a tool airband has no row for) | An entry in `launchers` in `~/.airband/config.json`, under a name of its own: the `harness` it starts, and either a command (`argv`, with `{cwd}`, `{task}`, `{project}`, `{session}` filled in) or `held` (`by: lines` with the line templates, or `by: acp` with the program). The file holds only his own; the ones that come with airband are never written there. | Config only |
| The models and efforts he can ask a new agent for ("with fable high, …") | `models`, `efforts`, `with_model` and `with_effort` on the launcher. One he names that the launcher does not offer is refused in words. | The row |
| Another way to drive a held program | A kind of `Held` in `airband-core/src/tools.rs` and its driver in `hold.rs`. The tower says the same typed lines to every holder (`hold::Tell`) and asks it where it has got to (`hold::State`). | Two files |
| Another way to reach an agent (an HTTP callback, a queue) | A variant of `Deliver` in `airband-core/src/agent.rs`; its arm in `tower/deliver.rs`. A tool that takes his words in a way of its own needs neither: `Deliver::Harness` and the adapter's `deliver`. | Two files |
| Something else an agent can report (a file, a cost) | A variant of `Event`. Old towers skip what they do not know. | `agent.rs`, `tower/intake.rs` |
| Permission prompts from another agent tool | No code: `airband report … permission "<what>"` and act on its exit code. Or two functions in its adapter: its prompt to a `PermissionAsk`, and yes or no to whatever the tool reads back. | `adapters/` only |
| Accounts for another agent tool (several logins, switched while sessions run) | Five functions in its adapter (`AccountBook`: list with allowances, switch, save, add, forget). `adapters/claude_accounts.rs` is the model. | `adapters/` only |
| What an agent is doing right now, and its model and effort | No code: `airband report … doing "<text>"` and `… model "<name>" [effort]`. Or one function in its adapter (`glance`). | `adapters/` only |
| Progress notes, what he types at an agent's own keyboard, and which team it belongs in, for another agent tool | One adapter function each: `steps` (the labels of its steps since a moment), `told` (his prompts, with any pictures he pasted), `touched` (the files and commands of its latest steps, from which a model says which project the work is part of, in `tower/place.rs`). | `adapters/` only |
| Another model, or no model | `airband setup`, or `summarizer` in the config. `summarizer_fallback` is a second one, asked when the first does not answer. A new kind is a variant of `Summarizer`, an arm in `tower/judge.rs` and a row in `PROVIDERS` (`setup.rs`). | Config, or three files |
| A better judgement of what is worth showing | `tower/judge.rs`. Try it with `airband triage < message.txt` on real messages first. | One file |
| A better guess at who a reply is for | `tower/route.rs` for the model's reading; `airband-core/src/routing.rs` for tags and spoken names | One of two files |
| Something else a button can tell the tower (`/mute`, `/today`) | A `Control` and its word in `COMMANDS` in `routing.rs`, and its arm in `tower/route.rs`. A radio's button sends the word behind a slash. | Two files |
| Another client (Android, Windows, a watch, a car) | A host: implement `DeskHost` (ten calls: four for raw audio, `changed`, `status`, `note`, `hearing`, `words`, `watching`) over the bindings `crates/uniffi-bindgen` generates, draw `Desk.view()`, and keep the links. The Swift one is `apps/apple/Sources/RadioModel.swift`. `terminal.rs` is a single-computer host on `RadioHost`. | A new folder under `apps/` |
| Another transport to the tower (a local socket) | A variant of `Route` in `airband-radio/src/link.rs` | One file |
| Another push service (Android's, a desktop's) | A struct beside `Apns` in `airband-relay/src/push.rs` and an arm in `Push::send`. The client opens the notification with `open_notice` and reports its address with `Desk.set_push`. | One file, and the client |
| Something else a client can ask | A route in `tower/http.rs`, its types in `airband-core/src/radio.rs`, a method on `Radio` | Three files |
| Another place to keep relay data | A variant of `Store` in `airband-relay/src/store.rs` | One file |

If a change does not fit a row, it is probably a new protocol field. Add the field as optional, default it, and
old peers keep working.

## Settings and keys

`airband setup` asks what makes summaries, what speaks and what listens, and tries each choice with a real call.
A provider is one file under `catalog/providers/`: what it can do, whether it needs a key, and what else it
has to be told.

A key is kept once in `keys` in `~/.airband/config.json`, under a name, with its provider; each use of it is
written as `key:<name>`. `airband keys` lists keys by name and use and never shows one.

No secret is written in that file. Every one is taken out and kept encrypted, with a mark left in its place
(`vault.rs`), in one of two stores behind the same three calls (`get`, `put`, `settle`):

| Store | Used by | Encrypted with |
|---|---|---|
| The login keychain, one item | a build signed with a developer certificate (`scripts/sign-mac.sh`) on a Mac | his login password, by the system. Read without a prompt only by the same signed program |
| `~/.airband/keys.sealed` | everything else: Linux, Windows, an unsigned build, a shell with no keychain | ChaCha20-Poly1305, with a key made from the machine's own id and a salt beside the file. A copy on another machine or in a backup opens nothing; a program running as him on this machine can still read it |

The loopback token, which hooks and commands on this computer show the tower, always stays in the settings
file: it opens nothing from outside, and a shell over SSH cannot read the keychain.

## Agents the tower runs

The tower can only leave an agent he started in a terminal a message from outside. One started through a
launcher with `held` is a process whose input the tower holds, so his words are its own turns, it can be
stopped, and what he says while it works waits until it stops (`tower/held.rs`). Each runs under
`airband hold`, a process apart from the tower, so it goes on while the tower restarts.

`POST takeover` ends a terminal session and carries it on as a held one, with a held launcher of the agent's own
harness. A held agent's hooks are told its id in `AIRBAND_HELD_ID`; `airband hook` puts that id and the `held`
delivery on every event (`adapters/mod.rs`), unless the launcher already starts the harness under that id.

## Sound

Outbound and inbound are set separately (`airband setup`, or `airband voice …` and `airband ears …`).

| | Interface | Where | Providers |
|---|---|---|---|
| Outbound: text to audio | `Synth` (text in, PCM out a piece at a time), one file for each under `airband-radio/src/voices/` | `tower/speech.rs` makes the calls | Any service that speaks OpenAI's `/audio/speech` (a row); Gemini, Microsoft and ElevenLabs by their own routes |
| Outbound, made on the phone | a file in `catalog/models/` | `airband-radio/src/models.rs`, run by `mouth.rs` | Phonon (Gradium, open weights, no key) |
| Inbound, on the phone | a file in `catalog/models/` | `airband-radio/src/models.rs`, run by the host (`transcribe`) | Phonon 2.5 English (Fermion Research, open weights, no key) |
| Inbound, live as he speaks | a dialect: a file under `airband-radio/src/ears/` and a line of `LISTENERS` | `airband-radio/src/ears.rs` | Microsoft, Google, ElevenLabs |
| Inbound, a finished recording | `transcribe` (a WAV in, words out), a line of `UPLOADS` | `airband-radio/src/hears.rs` | Any service that speaks OpenAI's `/audio/transcriptions` (a row) |

The tower makes speech the moment a sentence exists and hands it over in parts (`GET speech/<id>`), compressed
four to one (`airband-core/src/audio.rs`); the radio decodes each part and gives the host plain PCM, so a host
needs no codec. For listening the phone goes straight to the service with a short-lived pass from the tower,
and the rule for when he has finished speaking (`Endpointer`) is in the core. The tower's keys never leave it.

With nothing set (`device`), a terminal prints and takes a typed line. A phone has no voice or recognition
worth using: nothing is spoken and he types.

What he chooses is one mechanism in the phone's core (`airband-radio/src/choices.rs`). For each job (what
writes his summaries, what listens, what speaks) `Desk::choices(job)` is the one list: a model on the phone,
a service with a key of his own, or his computer's choice, each with how it stands (ready, needs a key,
needs a download, being fetched, failed and why). `Desk::choose_for` picks one: a service is tried with a
real call and a model is fetched first. One thing is chosen for a job at a time; that rule is in that file
and `ears.rs` and `mouth.rs` are handed the one thing to use. The choice is kept by the core
(`choices.json`); a key is kept by the host where its system keeps secrets (`DeskHost::secret`), goes only
to its service, and is never handed to a screen. A choice that stops working stays chosen and says why:
nothing else listens or speaks in its place. Summaries are written on his computer, so that list is the
computer's own (`GET reads`), with the one in use named by its id. The app draws one `ChoicePicker(job:)`
on the first run and in Settings alike.

The phone can also do both jobs itself, with a model on the device (`models.rs`). The program for it is in the
app and the model is not: he chooses it on the Voice or Listening page and only then is it fetched, once, from
`airband.dev/models/<name>/<version>/`, already in the form the phone runs, each file checked against its
hash. `scripts/host-model.sh` puts a model there. The voice (Gradium's Phonon, 65 MB) runs in the core and
stays loaded while it is chosen; the radio asks for `next?voice=own` and the tower sends words and makes no
speech. The listening model (Fermion Research's Phonon 2.5 English, 345 MB, iOS 18) runs on the Neural Engine
through the host (`transcribe`): the core records, has the recording so far read again about twice a second
to show his words as he talks, and has all of it read once more when he stops.

A call starts from when he joins: what was said before is in the chat and is not read out. The microphone never
opens by itself: a question is read to him and then waits in the chat (`airband-radio/src/call.rs`).

Every outside service, and every model that runs on a phone, is one small file in the catalogue: `catalog/providers/<id>.json`
and `catalog/models/<kind>.json` (beside them: one per coding tool, one per program the tower fetches to run a
model, and the words a recogniser should expect). Building `airband-core` puts them together into one
document that is compiled into the tower and the app (`crates/airband-core/src/catalog.rs`), so both know all
of it with no network. A newer copy is fetched from `airband.dev/catalog/` when he opens a chooser, and by
the tower twice a day (one piece of code for both, `airband-radio/src/models.rs`). So a new voice, model id or
price needs `scripts/publish-catalog.sh` and no new build.

A copy is taken only when all of these hold (`catalog::accept`):

- it is signed with airband's catalogue key, its `rev` is higher than the one in hand, and it has not run out
  (`expires`, 180 days from when it was published), so a copy replaced long ago cannot be served again;
- it is well formed (`catalog::check`): names are plain names, no provider is listed twice, a model's files
  stay inside its own folder;
- for every provider this build has compiled in, the interface it is reached through and everything that
  interface is told are unchanged, and so is the usual of every setting marked `reach` (an address, a
  region). A signed catalogue cannot send a key he has entered somewhere new, or have a known service called
  another way;
- nothing that starts a program on his computer is added or changed. Those offers (`command`,
  `on-this-computer`) are honoured only as compiled in, and the tower checks that again where the program is
  started (`tower/judge.rs`). The program fetched to run a local model (`catalog/servers/`) is always the
  compiled-in row;
- a version of a model this build knows keeps its files. New files are a new version, kept in a folder of
  its own (`models/<kind>/<version>`); the version already on the phone stays in use until the new one is
  whole.

Rows are read one at a time. One a build cannot read is left out and counted, the rest are kept, and the
tower's log says what was left out. `catalog/rev` holds the number and what the rows hashed to when it was
given: the tests fail when a row changed and the number did not (`scripts/catalog-stamp.sh` gives the next
one), and a copy with the same number and other rows is logged.

A way of calling a service is code and cannot arrive in a catalogue. A row names an interface and tells it
what it needs: `"by": { "interface": "<name>", "with": { … } }`. The code for each is in a table where the job
is done, looked up by that name: `READERS` in `tower/judge.rs`, `VOICES` in `airband-radio/src/voices.rs`,
`LISTENERS` in `airband-radio/src/ears.rs` and `UPLOADS` in `airband-radio/src/hears.rs`. Nothing else lists
interfaces. A program lists and calls only the offers it has the code for, so an old build reads a new
catalogue and shows what it can use. An offer can also say what its interface must be able to do (`needs`); a
build whose interface cannot leaves it out, where it would otherwise call the service the old way.

A provider's row says which jobs
the service does (reading, speaking, listening), through which interface, and what it has to be told, and what
to show where he chooses it: its maker and model, its price and the day that price was read, whether it is one
of the few shown first, and for a voice a few voices with their makers' sample clips. `airband
setup`, `airband model`, `airband voice` and `airband ears` are driven by the table, and so are the calls. A
setting in use is `{"kind": <row id>, "key": …, <what differs from the row>}`. What can read on a tower is
the tower's to say: `GET reads` lists its own offers and which one reads now, by id.

The interfaces, and what adding one touches:

| Job | Interface (`by.interface`) | A new provider is | A new interface is |
|---|---|---|---|
| Reading | `chat`: OpenAI Chat Completions (`POST {url}/chat/completions`), which nearly every model service and local model server accepts | a row | a function and a line of `READERS` in `tower/judge.rs`; a tower release |
| Reading | `command`: a program on his computer, prompt on standard input, answer on standard output | a row with its arguments, compiled in | |
| Reading | `on-this-computer`: a model the tower fetches and runs itself | a row naming the model and the program that runs it, compiled in | |
| Speaking | `speech`: OpenAI's speech endpoint (`POST {url}/audio/speech`, raw PCM) | a row | a file under `airband-radio/src/voices/` and a line of `VOICES`; an app and a tower release |
| Speaking | `gemini`, `azure`, `elevenlabs`: a route of its own, for what the common one lacks (ElevenLabs' word timings, Microsoft's own route). The row says where it is reached | a row | |
| Speaking or listening, on the phone | a file in `catalog/models/` | the row and the files under `airband.dev/models/<name>/<version>/` | the code that runs the model; an app release |
| Listening, a finished recording | `transcribe`: OpenAI's transcription endpoint (`POST {url}/audio/transcriptions`) | a row | a function and a line of `UPLOADS` in `airband-radio/src/hears.rs` |
| Listening, live as he speaks | `azure`, `gemini`, `elevenlabs`: no common interface exists, so each is a dialect the phone speaks | a row naming it | a file under `airband-radio/src/ears/` and a line of `LISTENERS`; an app release, because the phone talks to the service itself |

What the phone listens with comes as one message, `{ kind, token, set, phrases }`, whoever sends it: the phone
itself from a key of his own, or the tower. `set` is whatever the service
has to be told, so a service that needs a model or an address fits without a new field. A live service gives a
short-lived pass for a key, and the key stays where it is. A service that only takes a finished recording has
no such pass: with the key on the phone the recording goes straight to it, and with the key on the computer the
phone sends the recording to the tower (`POST hear`), which makes the call. The phone's own keys are kept by
provider in its Keychain, so one key serves a provider's voice and its listening.

## Reaching the tower

A phone paired through a relay sends every request sealed with the pairing's key. The tower's feed lists the
addresses it can be reached at on the local network (`direct`), and the radio posts the same sealed request
straight to `/v1/sealed/<channel>` there (`link.rs` `straight`, `relay_link.rs` `sealed_router`). When no
address answers it uses the relay. On local network addresses the tower serves only that one route.
`airband lan off` turns it off.

## Several computers

Each computer runs its own tower and has its own pairing link. The app runs one radio per link and shows them
as one inbox, each message labelled with its computer's name (the tower's `station`, by default the short host
name). An answer goes back through the radio its question came from. On a call only one computer has the floor
at a time. Towers do not talk to each other; the radio carries what they need. It tells the later of two towers
which numbered names the other uses (`POST taken`), so no two agents on his screen are called the same. It
sends the agents of his other computers with what he says (`others`), so a tower can answer that his words are
for one of those (`elsewhere`).

## Pictures

The tower copies only a picture an agent sent him (`picture` with `shown`), or one named in what it wrote that
lies in its own folder or a temporary one. One the agent only looked at is never opened. A picture he pastes
into Claude Code comes through with his typed message. The tower (`tower/pictures.rs`) keeps a small JPEG copy
in `~/.airband/pictures` for 30 days and attaches up to four to a message. The radio core fetches a copy when
the screen needs it (`Radio.picture`), in parts, and keeps the file for 14 days. There is no video.

## Who is who

An agent is known by its id, which does not change while it runs. Its callsign can change when he names the
session, and a later agent can be given a callsign an earlier one had. So every log line records the id of the
agent that said it, and a client gets both (`agent` and `from` on a message, `id` and `callsign` in the roster).

A radio follows the chat by change count (`since`), not by position: a line it already has comes again when its
short version is made or its sender is renamed.

## The feed, and what is in it

A host draws `Desk.view()`: one feed of reports, questions and what he said, oldest first; his teams, each in the
state of whichever of its agents most needs him; whether a poke would do anything; and the quick messages to
offer. All of it is worked out in the core, so every host shows the same thing.

- Summaries. The news comes first and the question last. What the work left outside the conversation is
  always kept with its number: a commit, a pull request, a build, a deploy (the prompt in `judge.rs`).
- Masking. Keys and tokens in what is said are replaced by their prefix and dots before the chat keeps, sends
  or summarises it (`redact.rs`). What an agent is given is not touched.
- Open questions. A question stays open until he answers, he dismisses it (`POST dismiss`), the agent speaks
  past it three times, or six hours pass (`tower/state.rs` `open_ids`).
- Notes on the way. For an agent at work for four minutes without a word, the labels of its steps go to the
  model, which writes one line if he would look for something in them (`progress_loop`, `judge::progress`). No
  notification, not read aloud.
- Stalled and Poke. The judge can find that an agent said what it would do next and stopped. `Desk.poke`
  sends "Keep going." to every such agent.
- Quick messages are data the tower serves (`quick_defaults` in `radio.rs`, and `quick` in its configuration).
- Subagents are counted from what an adapter reports. What each team is doing comes with the roster
  (`DeskRoom.doing`, and per agent in `DeskRoom.members`), never in the feed.
- Permission. A harness's permission prompt arrives as a question with Allow and Deny (`tower/permit.rs`).
  The buttons send `airband_core::radio::ALLOW` and `DENY`. Words of his own said in reply to the request are
  read by a model (`judge::consent`), and only a plain yes allows it. A "yes" not said in reply answers none.
  The tower never answers one itself.

## Notifications

What deserves a notification is the tower's decision (`tower/push.rs`): a question, a report, a permission card,
when the phone is neither on a call nor showing the chat. The tower seals it and the relay hands it to the push
service, so neither can read it. On the iPhone a notification extension (`apps/apple/Notify`) opens it with the
radio core. An answer given there is sent in the background (`Desk.answer_now`), or kept and sent later.

## Accounts

Someone with several subscriptions can see how much of each is used and move every running session on a computer
to another one, from the Accounts page or with `airband accounts`. A switch takes a tap on the account and
cannot be said. `tower/accounts.rs` keeps the list for five minutes and looks at saved logins every three hours
so none lapses.

For Claude Code (`adapters/claude_accounts.rs`): saved logins are keychain items of airband's own, and a switch
swaps the one Claude Code reads; running sessions pick it up within 30 seconds. macOS only so far. A login's
refresh token changes each time it is used, so airband has to be the only keeper of saved logins on a computer.

## The homepage

`site/index.html` is the whole of airband.dev: one static page (`AirbandSite` in `infra/cdk.mjs`), deployed by
its own workflow. Its waiting-list form posts an email address to the relay (`POST /v1/waitlist`), which takes
it from the homepage's origin only and limits how many it takes.

## What is kept

`~/.airband/airband.db` (SQLite) holds the chat log, every agent event as it arrived (credentials masked), a
receipt for every utterance the tower has taken, and every attempt to deliver his words. An utterance has a
receipt before it is routed, and an event is recorded before it is acted on, which makes a retry safe across a
restart. Events, receipts and deliveries go after a month; the newest 5,000 messages stay.

Whether he has heard a message is kept on the message (`aloud`), not in a second list, and changes through one
function that logs each change with its reason (`Inner::set_aloud`). Whether a radio is there is one record
per radio, which is what the radio itself last said (on a call or not, showing the chat or not, with every
request), read by one clock (`Presence`, `THERE`). Something he says is answered at once with "taken" and its
outcome arrives in the feed, so no request waits on a model. On the phone, every failed request ends in one named cause
(`LinkError`), and the trail of them stays on the phone. No log, trail or diagnostic is uploaded anywhere; the
relay logs only its own failed calls, with no content and six characters of the channel. `docs/protocols.md`,
"What is written down, and where it stays".

## Compatibility

Unknown fields are ignored, and unknown `type` and `kind` values are accepted and skipped. Adding is never a
new version. There is a test for this (`round_trips_and_tolerates_the_future`).

## Names

The tower is the daemon on your computer. A radio is a client. An agent is anything that does work on your
behalf and reports when it stops; the code also calls one a session. A room is one project: every agent working
in the same folder. Rooms and callsigns make themselves from where agents run and what he named their sessions.

## For contributors

- A rule goes in `airband-core` if it needs no I/O, and gets a test there.
- A host that finds itself deciding something has found a missing function in the radio core.
- The tower has no `if harness == …` anywhere, and starts no tool's watcher by name: it starts every adapter's (`adapters::watch_all`). Keep it that way.
- Messages from agents are untrusted input to the model that judges them. That model is given no tools.
- CI runs on every push to main that changes code: build and tests on Linux, Windows and macOS, the radio core
  for Android, the iPhone app, and two end-to-end suites that need no model (`tools/e2e-relay.py`,
  `tools/e2e-held.py`).
- `tools/e2e.py` goes through everything a radio can do with real model calls, so it runs on your computer:
  `scripts/check.sh --full`. Run it before a release.
- Say what was run and what was only compiled. `docs/state.md` is the record; keep it true.
