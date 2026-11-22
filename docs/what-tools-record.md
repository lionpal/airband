# What each coding tool records about the things it makes

airband shows a pull request or a published page as a card only when the tool that made it wrote that down
itself, as data. It never works out what an address is from how it reads. This is what each tool was found to
write, from real sessions on 2026-11-21 (Claude Code 2.1.316, Codex CLI 0.171.0, OpenCode 2.3.2). Names in the
examples are changed; the shapes are as captured.

## Claude Code

A pull request the session opened: one line in the session's transcript, written as soon as `gh pr create`
comes back (it is written twice). This is what Claude Code's own screen shows "PR #194" from.

    {"type":"pr-link","sessionId":"08613ff5-…","prNumber":194,"prUrl":"https://github.com/acme/shop/pull/194","prRepository":"acme/shop","timestamp":"2026-11-22T00:47:59.530Z"}

The hook for the command itself (`PostToolUse`, tool `Bash`) has only the command's printed output, as text.
airband reads the transcript line (`adapters/claude_code.rs`, `pull_requests`). There is no title in it, so the
card is called by the repository and the number.

A page it published: the `PostToolUse` hook for the `Artifact` tool carries the tool's answer.

    {"hook_event_name":"PostToolUse","tool_name":"Artifact","tool_input":{"file_path":"/x/shop/plan.html","icon":"test"},
     "tool_response":{"url":"https://claude.ai/artifact/3nMv…","artifact_id":"1687a527-…","title":"Refund plan","updated":false,"owned":true,"audience":"owner","seq":1}}

An issue it opened with `gh`: nothing is recorded. It is a plain link.

## Codex

Nothing. The address of a pull request appears only inside a command's printed output and in the agent's own
message, in the `codex exec --json` stream and in the session file under `~/.codex/sessions/` alike:

    {"type":"item.completed","item":{"id":"item_4","type":"command_execution","command":"/bin/zsh -lc \"gh pr create --draft …\"","aggregated_output":"https://github.com/acme/shop/pull/195\n","exit_code":0,"status":"completed"}}
    {"type":"item.completed","item":{"id":"item_5","type":"agent_message","text":"[Draft PR #195](https://github.com/acme/shop/pull/195) is open."}}

So a Codex pull request is a plain link. Codex cloud tasks were not tried.

## OpenCode

Nothing for a pull request. A command is a `tool` part whose state has `input`, `output` (text), `title`, `time`
and `metadata`; the address is only in the output text and in the agent's message. A session has a `share_url`
of its own, empty until he shares the session; sharing makes a public page, so it was not tried, and airband
does not read it yet.

## Any other tool

`airband report --id <id> made <address> [title]` says a thing was made, from a script.
