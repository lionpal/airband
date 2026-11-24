//! How the model that reads for the tower judges real-looking messages, through the tower's own prompt and the
//! usual reader (Claude on this computer, with its own login). It spends model calls, so it runs only when asked:
//!
//!     AIRBAND_EVAL_RUNS=5 cargo test --release -p airband-cli judge_eval -- --ignored --nocapture
//!
//! The tower's home is a throwaway one, as in every test (`config::home`); only `claude` reads its own login.
//! The messages here are made up, never his.
use super::*;

/// What a case should come back with.
enum Choices {
    /// Exactly these, in this order (case aside).
    These(&'static [&'static str]),
    /// This many, none of them a bare yes or no.
    Picks(usize),
    /// None: he answers in his own words, or has to go and do something.
    Nothing,
}

struct Case {
    name: &'static str,
    message: &'static str,
    kind: Kind,
    choices: Choices,
    /// Some: the one the agent itself recommended, or Some(None) for no mark at all. None: not checked.
    recommended: Option<Option<u32>>,
    /// The last thing he told the agent, if anything.
    asked: Option<&'static str>,
}

const RABBITS: &str = "I went through the sprite pipeline end to end.\n\n\
- The cat art set is still rabbits only: 14 sheets, all bunny 1 through bunny 4.\n\
- Your reference photos are in assets/identity/ and are what the face model was tuned on.\n\
- A full sheet through Meshy costs about 35 credits; four sheets would be about 140. You have 212 left.\n\
- The free tier makes one sheet a day at 512px, which is enough to check the style.\n\n\
Three decisions are yours: whether cat style stays rabbits-only, whether to keep your photos for identity, and \
whether to spend about 140 Meshy credits. Nothing is built; should I make one free sheet of bunny 1 and send it \
to you on Discord?";

const RABBITS_NEEDS_YOU: &str = "Here's where the sticker pack stands.\n\n\
**The art:** every cat sheet so far is drawn as rabbits (bunny 1 to bunny 4). That came from the first brief, \
and nobody changed it.\n\n\
**Identity:** the face model is tuned on your own photos in assets/identity/. It works, but they stay in the repo.\n\n\
**Cost:** Meshy charges about 35 credits a sheet, so all four would be about 140. The free tier makes one \
512px sheet a day.\n\n\
**Needs you:**\n\
- Whether cat style stays rabbits-only\n\
- Whether to keep your photos for identity\n\
- Whether to spend about 140 Meshy credits\n\n\
Nothing is built; should I make one free sheet of bunny 1 and send it to you on Discord?";

const CASES: &[Case] = &[
    Case { name: "his message, with a needs-you list", message: RABBITS_NEEDS_YOU, kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: None },
    Case { name: "his message, after what he asked", message: RABBITS_NEEDS_YOU, kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: Some("Look at the sticker pack and tell me what it would take to finish it.") },
    Case { name: "his message as it came, after what he asked", message: "Three decisions are yours: whether cat style stays rabbits-only, whether to keep your photos for identity, and whether to spend about 140 Meshy credits. Nothing is built; should I make one free sheet of bunny 1 and send it to you on Discord?", kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: Some("What's left before the sticker pack can ship?") },
    Case { name: "needs-you list, closing do-you-want", message: "**Done:** the signup flow is rebuilt and the tests pass.\n\n**Needs you:**\n- The wording of the terms checkbox\n- Whether minors can sign up at all\n- Which analytics events to keep\n\nDo you want me to deploy what's there to staging in the meantime?", kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: None },
    Case { name: "his message, as it came", message: "Three decisions are yours: whether cat style stays rabbits-only, whether to keep your photos for identity, and whether to spend about 140 Meshy credits. Nothing is built; should I make one free sheet of bunny 1 and send it to you on Discord?", kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: None },
    Case { name: "his message, after a report", message: RABBITS, kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: None },
    Case { name: "plain should-question", message: "I updated the migration and the tests pass locally. Should I push it to main?", kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: Some(None), asked: None },
    Case { name: "want me to", message: "The fix for the double charge is in and the payments tests pass. Want me to open a pull request?", kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: None },
    Case { name: "long report, closing yes-or-no", message: "Done with the search rewrite.\n\n## What changed\n- Queries go through the new index (search/index.rs), 4x faster on the 2M row set.\n- Typo tolerance is on for words of five letters or more.\n- The old endpoint still answers, marked deprecated in the docs.\n\n## Tests\n- 212 pass, 0 fail. Load test: p95 38 ms, was 160 ms.\n\n## Left\n- The admin page still calls the old endpoint.\n\nShall I deploy it to staging now?", kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: None },
    Case { name: "open decisions, closing yes-or-no", message: "Here is where the billing work stands.\n\nOpen decisions for you, whenever you have a minute:\n1. The price of the team tier ($12 or $15 a seat).\n2. Whether the legacy v1 API keeps working after March.\n3. Which domain the receipts are sent from.\n\nNone of those block the checkout page. Should I go ahead and build it with the current prices?", kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: None, asked: None },
    Case { name: "open decisions, nothing to pick", message: "Three decisions are yours before I can go on: what the team tier costs, whether the v1 API keeps working after March, and which domain receipts come from. Tell me what you want on each.", kind: Kind::NeedsYou, choices: Choices::Nothing, recommended: None, asked: None },
    Case { name: "A or B", message: "The importer hits rows with a missing date in about 2% of the file. It can skip those rows and list them at the end, or stop at the first one. Skip them or stop?", kind: Kind::NeedsYou, choices: Choices::Picks(2), recommended: Some(None), asked: None },
    Case { name: "one of three", message: "For the new cache I can use Postgres, SQLite, or keep the JSON files we have. Which do you want?", kind: Kind::NeedsYou, choices: Choices::Picks(3), recommended: Some(None), asked: None },
    Case { name: "A or B, agent recommends B", message: "Two ways to fix the parser: (a) patch the old one, quick but it will break again on nested lists, or (b) rewrite it on the new grammar, about two hours. I recommend (b). Which do you want?", kind: Kind::NeedsYou, choices: Choices::Picks(2), recommended: Some(Some(1)), asked: None },
    Case { name: "yes-or-no, agent recommends yes", message: "The cache layer saves 3 ms on a 400 ms request and caused both outages this month. I'd recommend removing it entirely. Should I remove it?", kind: Kind::NeedsYou, choices: Choices::These(&["Yes", "No"]), recommended: Some(Some(0)), asked: None },
    Case { name: "open-ended wording", message: "The welcome email is wired up and sends on sign-up. What should it say? I need the wording from you; I have left a placeholder.", kind: Kind::NeedsYou, choices: Choices::Nothing, recommended: Some(None), asked: None },
    Case { name: "open-ended names", message: "The beta flag is ready. Which customers should get it first? Give me their names or account ids.", kind: Kind::NeedsYou, choices: Choices::Nothing, recommended: Some(None), asked: None },
    Case { name: "go do it at the computer", message: "I need you to run `gh auth login` in a terminal on your Mac; the token I had expired. I'll carry on once it's done.", kind: Kind::NeedsYou, choices: Choices::Nothing, recommended: Some(None), asked: None },
    Case { name: "go plug in the key", message: "The deploy needs your hardware key. Plug in the YubiKey and touch it when the prompt shows on your Mac.", kind: Kind::NeedsYou, choices: Choices::Nothing, recommended: Some(None), asked: None },
    Case { name: "a report, no question", message: "Fixed the flaky upload test: it raced the temp folder cleanup. Pushed 4f2a1c to main, CI is green.", kind: Kind::Report, choices: Choices::Nothing, recommended: Some(None), asked: None },
];

/// What was wrong with one answer, or nothing.
fn wrong(case: &Case, j: &Judged) -> Option<String> {
    let said = || format!("kind {:?}, options {:?}, recommended {:?}", j.kind, j.options, j.recommended);
    if j.kind != case.kind {
        return Some(said());
    }
    let ok = match case.choices {
        Choices::These(these) => j.options.len() == these.len() && j.options.iter().zip(these).all(|(a, b)| a.eq_ignore_ascii_case(b)),
        Choices::Picks(n) => j.options.len() == n && !j.options.iter().any(|o| o.eq_ignore_ascii_case("yes") || o.eq_ignore_ascii_case("no")),
        Choices::Nothing => j.options.is_empty(),
    };
    let marked = case.recommended.is_none_or(|r| r == j.recommended);
    (!ok || !marked).then(said)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spends real model calls: AIRBAND_EVAL_RUNS=5 cargo test --release -p airband-cli judge_eval -- --ignored --nocapture"]
async fn judge_eval() {
    let runs: usize = std::env::var("AIRBAND_EVAL_RUNS").ok().and_then(|r| r.parse().ok()).unwrap_or(3);
    let cfg: Config = serde_json::from_value(json!({ "token": "own", "radio_token": "radio", "name": "Charles", "summarizer": { "kind": "claude", "offer": "haiku" } })).unwrap();
    let settings = Arc::new(Settings::held(cfg));
    let at_once = Arc::new(tokio::sync::Semaphore::new(8));
    let mut asked = tokio::task::JoinSet::new();
    for (at, _) in CASES.iter().enumerate() {
        for _ in 0..runs {
            let (settings, at_once) = (settings.clone(), at_once.clone());
            asked.spawn(async move {
                let _turn = at_once.acquire().await.unwrap();
                let item = Item { notice: false, full: CASES[at].message.into(), kind: None, say: None, asked: CASES[at].asked.map(String::from) };
                (at, wrong(&CASES[at], &prepare(&settings, &item).await))
            });
        }
    }
    let mut got: Vec<Vec<Option<String>>> = CASES.iter().map(|_| Vec::new()).collect();
    while let Some(done) = asked.join_next().await {
        let (at, w) = done.unwrap();
        got[at].push(w);
    }
    let mut all = 0;
    for (case, got) in CASES.iter().zip(&got) {
        let right = got.iter().filter(|w| w.is_none()).count();
        all += right;
        println!("{right}/{runs}  {}", case.name);
        for w in got.iter().flatten() {
            println!("        got {w}");
        }
    }
    println!("{all}/{} in all", CASES.len() * runs);
}
