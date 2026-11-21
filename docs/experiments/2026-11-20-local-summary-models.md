# Can a model on the computer write the summaries?

2026-11-20. Run on a Mac mini with an M4 and 16 GB of memory, while several coding agents were working on it.

## Why

The tower reads every message an agent writes and decides two things: how much it deserves the person's
attention (`needs-you`, `report`, `stalled` or `fyi`), and what to tell them of it in a sentence or two. By
default Claude Haiku does that through the person's own Claude login. Some people want nothing to leave their
computer, so the question was whether a small open model running on the same machine can do this job, how long
it takes, and how much memory it holds.

## What was run

- 20 real messages from one tower's history, 330 to 2,370 characters each: 8 that Haiku had judged
  `needs-you`, 8 `report`, 4 `fyi`. The messages themselves are not in this repository.
- Each message went through `airband triage`, which is the tower's own prompt and code path, pointed at a
  llama.cpp server (`llama-server`, build 12589) on the same computer through the `local` provider row.
- Three models, each as a 4-bit GGUF with thinking turned off:
  `unsloth/Qwen3.5-4B-GGUF:Q4_K_M` (2.7 GB), `unsloth/gemma-4-E2B-it-GGUF:Q4_K_M` (3.1 GB),
  `LiquidAI/LFM3-1.7B-GGUF:Q4_K_M` (1.1 GB).
- Measured per model: the first call after loading, every call's wall time, three calls at once, and the
  server's memory as `top` reports it.
- "Same verdict" counts how often the model put a message in the same one of the four classes as Haiku had.
  Haiku is the reference here, not ground truth.

Two rounds. In the first the model was asked in words for one line of JSON, as Haiku is. In the second the
server was held to the exact shape of the answer with a JSON schema (`response_format`), with the summary
written before the verdict.

## Results

Round 1, asked in words:

| Model | Typical call | Slowest | Three at once | Same verdict as Haiku | Fell back to the whole message |
|---|---|---|---|---|---|
| Qwen 3.5 4B | 3.2 s | 6.1 s | 16.3 s | 14 of 20 | 0 |
| Gemma 4 E2B | 1.6 s | 2.5 s | 4.4 s | 8 of 20 | 0 |
| LFM3 1.7B | 24.6 s | 31.0 s | 56.8 s | 5 of 20 | several |

Round 2, held to the shape:

| Model | First call | Typical call | Slowest | Three at once | Same verdict as Haiku | Missed a `needs-you` |
|---|---|---|---|---|---|---|
| Qwen 3.5 4B | 8.6 s | 3.5 s | 6.3 s | 17.1 s | 15 of 20 | 0 of 8 |
| Gemma 4 E2B | 3.0 s | 1.7 s | 2.4 s | 4.4 s | 8 of 20 | 5 of 8 |
| LFM3 1.7B | 3.0 s | 1.2 s | 2.5 s | 5.8 s | 10 of 20 | 5 of 8 |

Memory, round 2: `top` showed the server at 1.6 GB (Qwen), 0.5 GB (Gemma) and 0.5 GB (LFM3). That figure
leaves out the model file, which the server maps into memory, so the real cost is about the download size on
top of it: roughly 4 GB for Qwen. This total was not confirmed by a second method.

## What it shows

- **Holding the output to a shape matters most for the smallest model.** LFM3 went from broken replies and
  25-second calls to valid replies in 1.2 seconds. It had been running to its length limit.
- **Qwen 3.5 4B is the only one of the three that can be trusted with the verdict.** It never called a message
  that needed the person anything less. Its five disagreements were three reports it called `fyi` and two it
  called `needs-you`. Its summaries run longer than Haiku's (165 characters typical against 83).
- **The two faster models miss most of what needs the person.** Each called five of the eight `needs-you`
  messages something quieter. For this product that is the worst kind of mistake: the person is not told an
  agent is waiting.
- **Qwen is about as fast as Haiku through a Claude login for one message, and much slower for several at
  once.** Three together took 17 seconds, each waiting on the others.

## What it does not show

- 20 messages from one person's agents is a small sample.
- The prompt was written for Haiku and has not been tuned for any of these models.
- Whether the summaries keep the right facts was read by eye on a few and not scored.
- The same test on a Mac with less memory, or one without agents running, would give different times.

## Decision

Offer Qwen 3.5 4B as the local model for summaries, described as about 3 seconds a message and about 4 GB of
memory. Do not offer LFM3 1.7B or Gemma 4 E2B for this job until they stop missing `needs-you`.

## Run it again

`tools/bench-local-models.py` is the script. It needs a folder holding `msgs/00.txt`, `msgs/01.txt`, … and a
`meta.json` listing, for each, `{"i": 0, "kind": "<Haiku's verdict>", "chars": <length>, "haiku": "<Haiku's
summary>"}`; a tower's own history (`~/.airband/airband.db`, table `messages`, columns `full`, `kind`,
`short`) is where these came from.

```
BENCH_DIR=<that folder> LLAMA_SERVER=<path to llama-server> AIRBAND=target/release/airband \
  python3 tools/bench-local-models.py qwen3.5-4b=unsloth/Qwen3.5-4B-GGUF:Q4_K_M
```
