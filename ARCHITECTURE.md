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
| Protocols and rules | `crates/airband-core` | The three protocols' types; callsigns; who a tag or spoken name means; the words a radio's buttons send; what can be said aloud; masking credentials (`redact`); pairing links; the relay's sealing | Do any I/O. It has no HTTP client and reads no file. Everything here is tested directly. |
| Edges | `crates/airband-io` | Files written whole (`files`); a file of ours kept and what happens when it is damaged, decided once (`kept`: set aside, or left as it is with every save refused); another tool's file, never set aside or written over when unreadable (`foreign`); the HTTP client and reading a service's status (`http`); the catalogue fetched and kept (`catalog`) | Hold a rule of the product |
| The tower | `crates/airband-cli/src/tower/` | What is remembered and the rules about it (`state`), events in (`intake`), importance and wording (`judge`), recipients and controls (`route`), permission (`permit`), reaching agents (`deliver`), starting new ones (`launch`), running them itself (`held`), which team one belongs in (`place`), call mode's queue (`pump`), copies of pictures (`pictures`), files he attaches (`attachments`), the voice (`speech`), the listening pass (`ears`), accounts (`accounts`), the database (`store`), routes (`http`), the relay connection and sealed requests (`relay_link`), waking his phone (`push`) | Have a rule that depends on which harness an agent runs in or which client is asking |
| Adapters | `crates/airband-cli/src/adapters/` | Turning one harness's notifications into `airband.agent/1` events, and setting that harness up to send them (`airband install` asks each adapter) | Hold a harness up, or decide anything |
| The radio | `crates/airband-radio` | The whole client side. `desk`: every paired computer as one, what is waiting and done, where his messages go, who has the floor on a call. Under it, per computer: reaching the tower (`link`), the call loop (`call`), sending over bad connections (`outbox`), what is kept on the device (`store`), sound in and out (`voice`, `ears`) | Draw, make sound or capture it |
| Hosts | `apps/apple`, `crates/airband-cli/src/terminal.rs` | Drawing the view the desk gives, playing and capturing audio, keeping pairing links and a speech key of his own safe | Decide order, routing, retries, or what counts as waiting or done |
| The relay | `crates/airband-relay` | Passing sealed bytes between a radio and its tower; remembering which channels exist; handing a sealed notification to a push service; the homepage's waiting list | Hold a pairing key or read a message |
| Deployment | `infra/`, `.github/workflows/` | Running the relay: one small server in us-east-2 with DynamoDB behind it (`AirbandRelayUs` in `infra/cdk.mjs`, what runs on the server in `infra/host/`), deployed by `deploy-relay.yml` on a push that changes the relay | |

## Where a new thing goes

| You want to add | Do this | Touches |
|---|---|---|
| Another agent tool (a CI bot, anything that does work and stops) | No code: call `airband report` from whatever it can run. | Nothing |
| Another coding tool with an adapter of its own | One file in `adapters/` that implements `Harness` and one line in `HARNESSES`. It says only what the tool has: its hook events (the shared installer in `adapters/hooks.rs` puts them in the tool's file), or how to watch its server, and how his words get into one of its sessions. A request for leave goes through `hooks::leave`, which owns the length rules for every tool. Its row in `catalog/tools/` says what it is called and how it is started. | `adapters/`, one row |
| A coding tool that speaks the Agent Client Protocol, started from the phone | One row in `catalog/tools/`: `{ id, name, maker, launchers: [{ name, held: { by: "acp", program: […] } }] }`. No code: the holder's ACP driver (`acp.rs`) runs it and reports for it. The rows are compiled in; a catalogue fetched later can rename a tool and never change or add a program. | One row |
| A launcher of his own (another way to start a tool, or a tool airband has no row for) | An entry in `launchers` in `~/.airband/config.json`, under a name of its own: the `harness` it starts, and either a command (`argv`, with `{cwd}`, `{task}`, `{project}`, `{session}` filled in) or `held` (`by: lines` with the line templates, or `by: acp` with the program). The file holds only his own; the ones that come with airband are never written there. | Config only |
| The models and efforts he can ask a new agent for ("with fable high, …") | For a program started with arguments: `models`, `efforts`, `with_model` and `with_effort` on the launcher, and one he names that the row does not list is refused in words. An agent that speaks the Agent Client Protocol lists its own when a session opens (its settings of the protocol's kinds `model` and `thought_level`): its holder passes the two lists to the tower, which offers what the tool last listed (nothing until one of its agents has been started on this computer), and sets what he asked for before the agent is said to be there. One the agent does not have starts nothing, and he is told. | The row, or nothing |
| Another way to drive a held program | A kind of `Held` in `airband-core/src/tools.rs` and its driver in `hold.rs`. The tower says the same typed lines to every holder (`hold::Tell`) and asks it where it has got to (`hold::State`). | Two files |
| Another way to reach an agent (an HTTP callback, a queue) | A variant of `Deliver` in `airband-core/src/agent.rs`; its arm in `tower/deliver.rs`. A tool that takes his words in a way of its own needs neither: `Deliver::Harness` and the adapter's `deliver`. | Two files |
| Something else an agent can report (a file, a cost) | A variant of `Event`. Old towers skip what they do not know. | `agent.rs`, `tower/intake.rs` |
| Permission prompts from another agent tool | No code: `airband report … permission "<what>"` and act on its exit code. Or two functions in its adapter: its prompt to a `PermissionAsk`, and yes or no to whatever the tool reads back. | `adapters/` only |
| Accounts for another agent tool (several logins, switched while sessions run) | Five functions in its adapter (`AccountBook`: list with allowances, switch, save, add, forget). `adapters/claude_accounts.rs` is the model. | `adapters/` only |
| What an agent is doing right now, and its model and effort | No code: `airband report … doing "<text>"` and `… model "<name>" [effort]`. Or one function in its adapter (`glance`). | `adapters/` only |
| Progress notes, what he types at an agent's own keyboard, and which team it belongs in, for another agent tool | One adapter function each: `steps` (the labels of its steps since a moment), `told` (his prompts, with any pictures he pasted), `touched` (the files and commands of its latest steps, from which a model says which project the work is part of, in `tower/place.rs`). | `adapters/` only |
| Another model, or no model | `airband setup`, or `summarizer` in the config. When it does not answer he is told on his phone why (`judge::Trouble`, `Feed.reads_failed`) and gets each message as written; no second model is asked. A new kind is a variant of `Summarizer`, an arm in `tower/judge.rs` and a row in `PROVIDERS` (`setup.rs`). | Config, or three files |
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

`airband setup` asks what makes summaries and tries the choice with a real call. What listens and what speaks
are chosen in the app and run on the phone: nothing of either is set or done on the computer.
A provider is one file under `catalog/providers/`: what it can do, whether it needs a key, and what else it
has to be told.

A key is kept once in `keys` in `~/.airband/config.json`, under a name, with its provider; each use of it is
written as `key:<name>`. `airband keys` lists keys by name and use and never shows one.

No secret is written in that file. Every one is taken out and kept encrypted, with a mark left in its place
(`vault.rs`), in one of two stores behind the same three calls (`get`, `put`, `settle`):

| Store | Used by | Encrypted with |
|---|---|---|
| The login keychain, one item | a build signed with a developer certificate (`scripts/sign-mac.sh`) on a Mac | his login password, by the system. Read without a prompt only by the same signed program |
| `~/.airband/keys.sealed` | everything else: Linux, Windows, an unsigned build. A signed build whose keychain will not take the keys says so and keeps nothing; a machine with no id of its own keeps nothing either | ChaCha20-Poly1305, with a key made from the machine's own id and a salt beside the file. A copy on another machine or in a backup opens nothing; a program running as him on this machine can still read it |

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

The phone owns sound. What listens to him and what speaks to him are chosen on the phone and run by the
phone: a model on it, or a service it reaches itself with a key of his kept in its Keychain. His computer
sends words and receives words. It makes no speech, hears no recording and holds no key for either, and the
relay lends nothing.

| | Interface | Where | Providers |
|---|---|---|---|
| Outbound: text to audio | `Synth` (text in, PCM out a piece at a time), one file for each under `airband-radio/src/voices/` | `airband-radio/src/mouth.rs` makes the calls, on the phone | Any service that speaks OpenAI's `/audio/speech` (a row); services with a route of their own by the protocol's name (`generate-content`, `ssml`, `timed-stream`), every address and header from the row |
| Outbound, made on the phone | a file in `catalog/models/` | `airband-radio/src/models.rs`, run by its engine (`engines/ptts.rs`) | Phonon (Gradium, open weights, no key) |
| Inbound, on the phone | a file in `catalog/models/` | `airband-radio/src/models.rs`, run by the host's engine `phonon-coreml` (`run_model`) | Phonon 2.5 English (Fermion Research, open weights, no key) |
| Inbound, live as he speaks | a dialect: a file under `airband-radio/src/ears/` and a line of `LISTENERS` | `airband-radio/src/ears.rs` | by the protocol's name (`usp`, `bidi`, `chunks`); which company speaks it is the row's to say |
| Inbound, a finished recording | `transcribe` (a WAV in, words out), a line of `UPLOADS` | `airband-radio/src/hears.rs` | Any service that speaks OpenAI's `/audio/transcriptions` (a row) |

A transmission is words (`Transmission.say`); the radio says them with the voice he chose (`voice.rs`,
`mouth.rs`) and tells the tower when they were heard. For listening the phone goes straight to the service: a
live one gives a short-lived pass for his key (`ears.rs` asks for it), and one that takes a finished recording
is sent the recording (`hears.rs`). The rule for when he has finished speaking (`Endpointer`) is in the core.
The names he is likely to say (his agents and their teams, from the roster the phone already has, and the
words of the trade the catalogue lists) go to a recogniser that can be told what to expect.

With nothing chosen for a job, nothing does it: he is told where to choose (Settings, under Listening or
Voice) at the moment he opens the microphone or turns reading aloud on, and nothing is used in its place. A
terminal radio (`airband radio`) prints what is said and takes a typed line; it makes no sound.

What he chooses is one mechanism in the phone's core (`airband-radio/src/choices.rs`). For each job (what
writes his summaries, what listens, what speaks) `Desk::choices(job)` is the one list: for listening and for the voice, a model on the phone or
a service with a key of his own; each with how it stands (ready, needs a key,
needs a download, being fetched, failed and why). `Desk::choose_for` picks one: a key is tested first and a model is fetched first. The test is the
phone's own: one request the provider's row names (`key.check`: a method, an address, how the key is
presented, and which statuses mean good, wrong key, or refused), chosen to cost nothing, read by its status
alone (`keys.rs`). A key that does not pass is not kept, and he is told which of those it was. One thing is chosen for a job at a time; that rule is in that file
and `ears.rs` and `mouth.rs` are handed the one thing to use. The choice is kept by the core
(`choices.json`); a key is kept by the host where its system keeps secrets (`DeskHost::secret`), goes only
to its service, and is never handed to a screen. A choice that stops working stays chosen and says why:
nothing else listens or speaks in its place. Summaries are written on his computer, so that list is the
computer's own (`GET reads`), with the one in use named by its id. The app draws one `ChoicePicker(job:)`
on the first run and in Settings alike.

The phone can also do both jobs itself, with a model on the device (`models.rs`). The program for it (its
`engine`, named by the model's row) is in the app and the model is not. An engine is either in the core
(`engines.rs`, one module each) or the host's own, which the host names in `device()` and runs when asked
through `run_model`; a model is listed only where its engine is and on a system its row allows. What is chosen
and loaded belongs to the desk (`Senses`), not to the process. The model is fetched late: he chooses it on the Voice or Listening page and only then is it fetched, once, from
`airband.dev/models/<name>/<version>/`, already in the form the phone runs, each file checked against its
hash. `scripts/host-model.sh` puts a model there. The voice (Gradium's Phonon, 65 MB) runs in the core and
stays loaded while it is chosen. The listening model (Fermion Research's Phonon 2.5 English, 345 MB, iOS 18) runs on the Neural Engine
through the host (`run_model`): the core records, has the recording so far read again about twice a second
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
setup` and `airband model` are driven by the table for reading, the phone's pickers for listening and the
voice, and so are the calls. A
setting in use is `{"kind": <row id>, "offer": <offer id>, "key": …, <what differs from the offer>}`. A
provider can do one job more than one way (two voice models): each is an offer with an id of its own, and
the key is the provider's, asked for once. How a key is presented to a service on a common interface is in
the row too (`by.with.auth`: the header and what stands before the key). What can read on a tower is the
tower's to say: `GET reads` lists its own offers and which one reads now, by provider and offer.

The catalogue in use is one shared value (`catalog::current()`, an `Arc`), replaced when a newer signed
copy is taken; the tower and the app load and fetch it with the same code (`airband_radio::catalog_load`,
`catalog_refresh`). A later copy can add an offer to a provider a build knows only when it is reached
exactly as one of that provider's compiled-in offers is; any other is left out and logged.

The interfaces, and what adding one touches:

| Job | Interface (`by.interface`) | A new provider is | A new interface is |
|---|---|---|---|
| Reading | `chat`: OpenAI Chat Completions (`POST {url}/chat/completions`), which nearly every model service and local model server accepts | a row | a function and a line of `READERS` in `tower/judge.rs`; a tower release |
| Reading | `command`: a program on his computer, prompt on standard input, answer on standard output | a row with its arguments, compiled in | |
| Reading | `on-this-computer`: a model the tower fetches and runs itself | a row naming the model and the program that runs it, compiled in | |
| Speaking | `speech`: OpenAI's speech endpoint (`POST {url}/audio/speech`, raw PCM) | a row | a file under `airband-radio/src/voices/` and a line of `VOICES`; an app release |
| Speaking | `generate-content`, `ssml`, `timed-stream`: a wire protocol that is one service's own, named for the protocol, for what the common one lacks (word timings, SSML). The row says where it is reached (`url`, which may name his settings in braces), how the key is presented (`auth`), what else is sent (`headers`) and the rate of the sound (`rate`) | a row | |
| Speaking or listening, on the phone | a file in `catalog/models/` | the row, with what its engine is told (`with`: rate, language, form, device, the system it needs, what to say while it is made ready), and the files under `airband.dev/models/<name>/<version>/` | a new engine: one module and one row of `HERE` in `airband-radio/src/engines.rs`, or one entry in the host's own table (`RadioModel.engines`), and an app release |
| Listening, a finished recording | `transcribe`: OpenAI's transcription endpoint (`POST {url}/audio/transcriptions`) | a row | a function and a line of `UPLOADS` in `airband-radio/src/hears.rs` |
| Listening, live as he speaks | `usp`, `bidi`, `chunks`: no common interface exists, so each is a dialect the phone speaks, named for its wire protocol. The row says where the stream is (`stream`), where its pass is asked for (`pass`) and how the key is presented (`auth`) | a row naming it | a file under `airband-radio/src/ears/` and a line of `LISTENERS`; an app release, because the phone talks to the service itself |

What the phone listens live with is one thing it makes for itself, `Pass { dialect, token, set, phrases }`
(`ears.rs`): `set` is whatever the service has to be told, so a service that needs a model or an address fits
without a new field. A live service gives a short-lived pass for his key; a service that only takes a
finished recording is sent it with the key. The phone's keys are kept by provider in its Keychain, so one key
serves a provider's voice and its listening.

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
