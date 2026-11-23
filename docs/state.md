# What has been tried, and what has not

Kept true as things change. A claim on this page was run, not only compiled.

## Every push

CI builds and tests on Linux, Windows and macOS, builds the radio core for Android, compiles the iPhone app,
and runs three end-to-end suites that need no model: `tools/e2e-relay.py` (the relay and what it keeps for a
tower that is away), `tools/e2e-held.py` (agents the tower runs, with a stand-in for the agent) and
`tools/e2e-links.py` (what agents make that lives at a web address). The suite
that needs a model, `tools/e2e.py`, runs on the computer: `scripts/check.sh --full`.

## The tower

Checked on a Mac: the tower; the Claude Code adapter (a live session heard and answered end to end);
`airband report` with tmux delivery; the chat feed with scopes, group and multi-agent sends; untagged replies
finding the right agent among three; a message typed while the tower was unreachable arriving once when it
came back; launching a new agent; a retried message delivered once across a tower restart. The judging was
tried on 24 real end-of-turn messages from working sessions (`airband triage < message.txt` judges one): 10
came out fyi, 7 needs-you, 7 report.

`tools/e2e.py --claude` covers, against a throwaway tower: names, team states, pictures, links, quick messages,
routing by words within a team and across teams, a follow-up, poke, a new agent from an idea, the longer
summary for findings, what each agent is doing (read from a made-up Claude Code transcript, not yet a real
one), permission allowed, refused, refused with a reason and left unanswered, accounts against made-up logins
and a stand-in for Anthropic's servers, and a real Claude Code session whose permission prompt was allowed
from the radio.

A new agent on a named model: `/launch shop with haiku low, …` started a real Claude Code session that came up
as "Haiku 5.5 with low effort", answered, reported and signed off.

Accounts against real logins: the login in use was saved and its weekly allowance read, on one Mac with one
account. A real switch between two accounts, and `airband accounts add`, have not been run.

The tower has not been run on Windows or Linux (it builds and its tests pass there).

## The relay

Live at `https://relay.airband.dev`: one small server in us-east-2 that keeps its channels, kept messages and
the waiting list in DynamoDB. `tools/e2e-relay.py` was run against it, against a relay on a Mac with a folder
and with real DynamoDB tables, and the store's own rules (an id kept once under five writes at once, the most a
channel may have, the count put right) against the real tables. A speech pass went through it. The deploy
workflow has shipped it from a push.

The real tower was reached through the hosted relay from the terminal client.

## The phone

The iPhone app is in TestFlight. One-tap answers, reminders (with the interval shortened), an inbox merged from
two towers, the permission card, the Accounts page and what each agent is doing were seen in the simulator.
Nothing on this page records its audio, CarPlay or background behaviour on a phone. Android: no Kotlin app.

Notifications: the relay's own code was run against Apple's push service with a made-up address and got
`BadDeviceToken` back (the key, the signed pass and HTTP/2 work). `tools/e2e-push.py` (no model) runs a real
tower against a stand-in relay and opens each notification with Python's own ChaCha20-Poly1305. In the
simulator, through Apple's sandbox push service: a question arrived with the agent's name and its own answers
as buttons, tapping one answered it on the tower, and a permission card was denied from its notification. Not
seen: the production push service delivering to a phone, the move of the pairing links to the shared Keychain
group on a phone that was already paired, a typed reply, Allow behind Face ID, and the "not sent" notification.

The folder the app and its extension share (App Group `group.dev.airband`): the team colours' one move out of
the Keychain, and answer buttons kept newest first with a change number that tells a process to register again, are tested in the core
(`shared::tests`, with a stand-in for the Keychain item). The app was only compiled with it, for the simulator and
unsigned; it has not run signed with the group, on a phone or in the simulator. The group has to be made and
given to both identifiers at developer.apple.com before the next TestFlight build (`scripts/testflight.sh`).

## Homebrew

The formula was installed on a second Mac (Apple silicon, no airband on it) with
`brew install lionpal/airband/airband` from the public tap, `lionpal/homebrew-airband`, and
`airband --version` answered from Homebrew's folder. Not run: `airband install` from Homebrew's copy (its
choice of Homebrew's path has a unit test only), `brew test`, the Intel Mac binary, and the Release workflow
itself (0.1.1 was built and published by hand; the two Linux binaries printed their version in a container).

## Not recorded here as run

`airband setup` and `airband keys`; the keys in the login keychain with a signed build; the direct connection
on the same network; progress notes; a speech key of his own on the phone; names kept unique across two
computers; dismissing a question.
