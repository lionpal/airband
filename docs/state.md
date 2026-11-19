# What has been tried, and what has not

Kept true as things change. A claim on this page was run, not only compiled.

Checked on a Mac: the tower; the Claude Code adapter (a live session heard and answered end to end);
`airband report` with tmux delivery; the chat feed with scopes, group and multi-agent sends; untagged replies
finding the right agent among three; a message typed while the tower was unreachable arriving once when it
came back; the relay, run locally over plain HTTP (sealed traffic, a forged request and a wrong token
refused). The judging was tried on 24 real end-of-turn messages from working sessions
(`airband triage < message.txt` judges one): 10 came out fyi, 7 needs-you, 7 report.

The relay is live at `https://relay.airband.dev`: a request and its answer were passed through it by hand over HTTPS,
and it keeps its channel list in Postgres. CI builds and tests on Linux, Windows and macOS, builds the radio core for
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
`brew install airband-dev/airband/airband`, from a copy of the tap on that Mac's disk; the binary came from
`https://airband.dev/releases/` and `airband --version` answered from Homebrew's folder. Not done: the public tap
repository `airband-dev/homebrew-airband` does not exist yet (the tap is kept in `lionpal/homebrew-airband`,
private), so the command does not work for anyone else yet. Not run: `airband install` from Homebrew's copy (its
choice of Homebrew's path has a unit test only), `brew test` (that Mac's developer tools were too old for
Homebrew to run any test), the Intel Mac binary, and the Release workflow itself (0.1.1 was built and published
by hand; the two Linux binaries printed their version in a container).

Not checked: the phone app on a phone. The iPhone app has never run on a phone, so
nothing about its audio, CarPlay or background behaviour is known. The tower has not been run on Windows or Linux
(it builds and its tests pass there). Android: no Kotlin app. No push notifications: the phone sees new messages while the app is open or call mode is on.
