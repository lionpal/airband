# What has been tried, and what has not

Kept true as things change. A claim on this page was run, not only compiled.

Checked on a Mac: the tower; the Claude Code adapter (a live session heard and answered end to end);
`airband report` with tmux delivery; the chat feed with scopes, group and multi-agent sends; untagged replies
finding the right agent among three; a message typed while the tower was unreachable arriving once when it
came back; the relay, run locally over plain HTTP (sealed traffic, a forged request and a wrong token
refused). The judging was tried on 24 real end-of-turn messages from working sessions
(`airband triage < message.txt` judges one): 10 came out fyi, 7 needs-you, 7 report.

The relay is live at `https://relay.airband.dev`: one small server in the United States (us-east-2) that keeps
its channels, kept messages and the waiting list in DynamoDB. `tools/e2e-relay.py` was run against it (13 of 13),
against a relay on this Mac with a folder and with real DynamoDB tables (19 of 19 each), and the store's own
rules (an id kept once under five writes at once, the most a channel may have, the count put right) against the
real tables. A speech pass and a notification to a made-up address (Apple answered `BadDeviceToken`) went
through it. Not run: the deploy workflow as rewritten for this server; the relay was shipped by hand with the
same commands. The relay that ran in Tokyo before is still up and is no longer what the name points at. CI builds and tests on Linux, Windows and macOS, builds the radio core for
Android and compiles the iPhone app.

Launching a new agent, the database (a retried message delivered once across a tower restart) and the installed
tower with a real session checking in were all tried on the Mac.

The real tower was reached through the hosted relay from the terminal client. One-tap answers, reminders (with the
interval shortened) and an inbox merged from two towers were tried against throwaway towers and in the simulator.

`tools/e2e.py --claude` passed all 51 of its checks when permission and accounts were added against a throwaway tower: names, team states, pictures, links,
quick messages, routing by words within a team and across teams, a follow-up, poke, a new agent from an idea,
permission allowed, refused, refused with a reason and left unanswered, and a real Claude Code session whose
permission prompt was allowed from the radio. The permission card was seen in the simulator.

Accounts: listing, allowances, refreshing a saved login and switching both ways pass in `tools/e2e.py` against
made-up logins and a stand-in for Anthropic's servers, and the page was seen in the simulator
with three made-up accounts. Against real logins: the login in use was saved and its weekly allowance read, on one
Mac with one account. A real switch between two accounts, and `airband accounts add`, have not been run.

A new agent on a named model: `/launch shop with haiku low, …` started a real Claude Code session on this Mac
that came up as "Haiku 5.5 with low effort", answered, reported and signed off. The longer summary for findings
was tried in `tools/e2e.py` on one made-up investigation report (now 52 checks, 55 with `--claude`).

What each agent is doing: pushed by a script and read from a made-up Claude Code transcript in `tools/e2e.py`
(62 checks with `--claude`), and seen in the simulator as the bubble under the teams bar and in the agents list.
Not yet seen with a real Claude Code session's own transcript.

Homebrew: the formula was installed on a second Mac (Apple silicon, no airband on it) with
`brew install lionpal/airband/airband`, from a copy of the tap on that Mac's disk; the binary came from
`https://airband.dev/releases/` and `airband --version` answered from Homebrew's folder. The tap is public at `lionpal/homebrew-airband`, and the
command was run again from it on that Mac. Not run: `airband install` from Homebrew's copy (its
choice of Homebrew's path has a unit test only), `brew test` (that Mac's developer tools were too old for
Homebrew to run any test), the Intel Mac binary, and the Release workflow itself (0.1.1 was built and published
by hand; the two Linux binaries printed their version in a container).

Not checked: the phone app on a phone. The iPhone app has never run on a phone, so
nothing about its audio, CarPlay or background behaviour is known. The tower has not been run on Windows or Linux
(it builds and its tests pass there). Android: no Kotlin app.

Notifications: the relay's own code was run against Apple's push service with a made-up address and got
`BadDeviceToken` back (the key, the signed pass and HTTP/2 work). `tools/e2e-push.py` (19 checks, no model)
runs a real tower against a stand-in relay and opens each notification with Python's own ChaCha20-Poly1305. In
the simulator, with a tower and a relay on this Mac and Apple's sandbox push service in between: a question
arrived with the agent's name and its own answers as buttons, tapping one answered it on the tower, a permission
card was denied from its notification, and with the Communication Notifications entitlement the badge showed as
the sender's picture. Not seen: any of it on a phone, the production push service delivering to one, the move of
the pairing links to the shared Keychain group on a phone that was already paired, a typed reply, Allow behind
Face ID, and the "not sent" notification.
