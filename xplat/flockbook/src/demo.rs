//! Deterministic demo data. Neutral names, no real sessions, no sockets.
//!
//! 6 workspaces, 16 tabs: 6 working, 2 blocked, 5 done, 3 idle. The default selection of the snapshot is
//! Quarry / ranking, whose conversation is long and mixed (bubbles, prose, headings, lists, a table, code, tool
//! chips, CJK).

use crate::source::DataSource;
use herdr_core::{Agent, AgentStatus, ConversationItem, ItemKind, Pane, Tab, ToolCall, ToolChip, Workspace};
use herdr_core::PermState;
use std::collections::HashMap;

pub struct DemoSource {
    workspaces: Vec<Workspace>,
    conversations: HashMap<String, Vec<ConversationItem>>,
}

/// (workspace, [(tab label, agent name, status)]).
const TREE: &[(&str, &[(&str, &str, AgentStatus)])] = &[
    ("Orchard", &[("cart drawer", "orchard-ui-dev", AgentStatus::Working), ("pricing", "orchard-api-dev", AgentStatus::Done), ("e2e", "orchard-e2e-dev", AgentStatus::Done)]),
    ("Atlas", &[("backfill", "atlas-backfill-dev", AgentStatus::Working), ("schema", "atlas-schema-dev", AgentStatus::Working), ("importer", "atlas-import-dev", AgentStatus::Done)]),
    ("Harbor", &[("deploy", "harbor-deploy-ops", AgentStatus::Blocked), ("ci", "harbor-ci-dev", AgentStatus::Done), ("runbook", "harbor-runbook-docs", AgentStatus::Idle)]),
    ("Lighthouse", &[("getting started", "lighthouse-start-docs", AgentStatus::Blocked), ("api ref", "lighthouse-api-docs", AgentStatus::Idle)]),
    ("Tidepool", &[("parser", "tidepool-parse-dev", AgentStatus::Working), ("release", "tidepool-release-dev", AgentStatus::Idle)]),
    ("Quarry", &[("ranking", "quarry-rank-dev", AgentStatus::Working), ("bench", "quarry-bench-perf", AgentStatus::Working), ("notes", "quarry-notes-docs", AgentStatus::Done)]),
];

fn item(id: &str, kind: ItemKind, ts: f64) -> ConversationItem {
    ConversationItem { id: id.into(), kind, timestamp: Some(ts) }
}

fn chip(id: &str, symbol: &str, text: &str, tool: &str, summary: &str, output: Option<&str>) -> ToolChip {
    ToolChip {
        id: id.into(),
        symbol: symbol.into(),
        text: text.into(),
        calls: vec![ToolCall { id: format!("{id}-c"), tool_name: tool.into(), summary: summary.into(), input: summary.into(), output: output.map(Into::into), is_error: false }],
    }
}

const BASE_TS: f64 = 1_790_000_000.0;

fn ranking_conversation() -> Vec<ConversationItem> {
    use ItemKind::*;
    vec![
        item("u1", UserText("Search ranks the old bestseller above the new listing. Can you find out why?".into()), BASE_TS),
        item("c1", ToolChips(vec![chip("c1a", "doc.text", "Read 6 files", "Read", "src/ranking/score.py", Some("ok")), chip("c1b", "terminal", "Ran 2 commands", "Bash", "pytest tests/ranking -q", Some("14 passed"))]), BASE_TS + 20.0),
        item(
            "a1",
            AssistantText(
                "The cause is clear in the table below: clicks are summed with no decay, so a 2-year-old listing with 912 clicks beats a better match from this week.\n\n\
                 ## Proposal\n\n\
                 1. Use `log1p(clicks)` so popularity cannot dominate.\n\
                 2. Decay clicks with a 30-day half-life.\n\
                 3. Keep the BM25 weight at 0.8."
                    .into(),
            ),
            BASE_TS + 80.0,
        ),
        item("u2", UserText("Try it and show me the same query.".into()), BASE_TS + 400.0),
        item("c2", ToolChips(vec![chip("c2a", "pencil", "Edited a file", "Edit", "src/ranking/score.py", Some("ok")), chip("c2b", "terminal", "Ran a command", "Bash", "python -m ranking.query \"gala apples\"", Some("3 rows"))]), BASE_TS + 420.0),
        item(
            "a2",
            AssistantText(
                "The new-season listing is first now, and the old bestseller still ranks second on relevance.\n\n\
                 | Listing | Rank before | Rank after |\n|---|---|---|\n\
                 | Gala apples, new season | 3 | **1** |\n| Gala apples, 10 kg (2024) | 1 | 2 |\n| Royal Gala, organic | 4 | 3 |\n\n\
                 Next I'll replay last week's query log to check the change does not hurt long queries."
                    .into(),
            ),
            BASE_TS + 460.0,
        ),
        item("u3", UserText("好，請再用中文總結一次結果，並列出風險。".into()), BASE_TS + 600.0),
        item(
            "a3",
            AssistantText(
                "結果摘要：新上架的商品現在排第一，舊的暢銷商品仍然排第二。\n\n\
                 ### 風險\n\n\
                 - 長尾查詢可能受到*衰減*影響，需要用上週的查詢紀錄重放驗證。\n\
                 - `half_life_days`設為 30 只是起點，之後要用 A/B 測試調整。\n\n\
                 Latency stays flat: p95 went from 41 ms to 43 ms."
                    .into(),
            ),
            BASE_TS + 640.0,
        ),
        item("u4", UserText("Show me the final scoring function.".into()), BASE_TS + 800.0),
        item(
            "a4",
            AssistantText(
                "Here it is. Fresh listings keep their clicks, old ones fade out:\n\n\
                 ```python\n\
                 def score(doc, bm25, now):\n    # Clicks fade with a 30-day half-life.\n    age_days = (now - doc.created).days\n    fresh = 0.5 ** (age_days / 30)\n    return bm25 * 0.8 + math.log1p(doc.clicks * fresh) * 0.2\n\
                 ```"
                    .into(),
            ),
            BASE_TS + 840.0,
        ),
        item("c3", ToolChips(vec![ToolChip { id: "c3a".into(), symbol: "terminal".into(), text: "Running Bash...".into(), calls: vec![ToolCall { id: "c3a-c".into(), tool_name: "Bash".into(), summary: "python -m ranking.replay logs/last-week.jsonl".into(), input: "python -m ranking.replay logs/last-week.jsonl".into(), output: None, is_error: false }] }]), BASE_TS + 860.0),
    ]
}

/// How a demo conversation ends.
#[derive(Clone, Copy)]
enum End {
    /// The agent's last words.
    Say(&'static str),
    /// A finished tool step, then nothing.
    Tool(&'static str, &'static str),
    /// A tool still running.
    Running(&'static str, &'static str),
    /// A permission request.
    Ask(&'static str),
    /// A question with options (AskUserQuestion).
    AskQuestion,
}

/// Per agent: (the user's request, the agent's first answer, the ending).
fn script(agent: &str) -> (&'static str, &'static str, End) {
    match agent {
        "orchard-ui-dev" => ("Make the cart drawer slide in from the right.", "The drawer is a fixed panel today. I will move it into a transform transition.", End::Running("Edit", "src/cart/Drawer.tsx")),
        "orchard-api-dev" => ("Round prices to the currency's minor unit.", "Prices were rounded twice. I removed the second pass.", End::Say("Done. Totals now match the invoice to the cent.")),
        "orchard-e2e-dev" => ("Add a checkout test for a guest user.", "The guest flow needs a fixture with no saved address.", End::Say("The guest checkout test passes in 11 s.")),
        "atlas-backfill-dev" => ("Backfill 2023 events into the new table.", "I split the job into monthly batches to keep locks short.", End::Tool("terminal", "Ran 3 commands")),
        "atlas-schema-dev" => ("Draft the migration for the events partition key.", "A composite key on (tenant, day) covers every query in the log.", End::Running("Read", "migrations/0042_partition.sql")),
        "atlas-import-dev" => ("Import the partner CSV exports.", "The exports use semicolons. I added a delimiter sniffer.", End::Say("All 4 files imported. 12 rows were skipped as duplicates.")),
        "harbor-deploy-ops" => ("Ship build 418 to staging.", "The image is built and tagged. Next is the rollout.", End::Ask("./scripts/deploy.sh --env staging")),
        "harbor-ci-dev" => ("Why is the lint job slow?", "The cache key changed on every run. I pinned it to the lockfile hash.", End::Say("Lint now takes 40 s instead of 3 min.")),
        "harbor-runbook-docs" => ("Write the rollback runbook.", "I listed the three rollback paths in order of risk.", End::Say("The runbook is ready for review.")),
        "lighthouse-start-docs" => ("Rewrite the install step for Windows.", "The old text assumes a Unix shell. I need to know who reads this page before I write it.", End::AskQuestion),
        "lighthouse-api-docs" => ("Generate the API reference from the schema.", "The schema has 38 endpoints. I grouped them by resource.", End::Say("Reference generated. Two endpoints lack descriptions.")),
        "tidepool-parse-dev" => ("The parser fails on nested quotes.", "I wrote a failing case first. It breaks on an escaped quote in a key.", End::Tool("pencil", "Edited a file")),
        "tidepool-release-dev" => ("Prepare the 0.9 release notes.", "I collected 14 merged changes since the last tag.", End::Say("Notes drafted. Waiting for the version bump.")),
        "quarry-bench-perf" => ("Benchmark the new scoring function.", "I set up 10 000 queries against a warm index.", End::Say("Warm-up is done. The first 2 000 queries are running now.")),
        "quarry-notes-docs" => ("Summarize today's ranking changes.", "I read the diff and the benchmark output.", End::Say("Summary saved to the notes page.")),
        _ => ("Please pick up the next task.", "I read the notes and the open issues.", End::Say("Ready for the next task.")),
    }
}

/// The demo's pending question, same shape as an AskUserQuestion call.
fn demo_question() -> herdr_core::QuestionRequest {
    use herdr_core::{QOption, Question};
    let opt = |label: &str, detail: &str, recommended: bool| QOption { label: label.into(), detail: detail.into(), recommended };
    herdr_core::QuestionRequest::new(
        "toolu_demo_q",
        vec![Question::new(
            "Audience",
            "Who is the Getting Started page for?",
            vec![
                opt("New contributors", "Developers joining the repo: layout, running the docs site locally, making a first change. The index has no setup steps, so this fills the biggest gap.", true),
                opt("Operators who deploy", "People who run deploys: the Harbor flow, environment setup, and what to check after a deploy.", false),
                opt("API users", "Consumers of the services: calling Orchard and Atlas, with request and response basics.", false),
            ],
            false,
        )],
    )
}

fn scripted_conversation(agent: &str) -> Vec<ConversationItem> {
    use ItemKind::*;
    let (ask, first, end) = script(agent);
    let mut v = vec![
        item("g-u1", UserText(ask.into()), BASE_TS),
        item("g-c1", ToolChips(vec![chip("g-c1a", "doc.text", "Read 3 files", "Read", "README.md", Some("ok"))]), BASE_TS + 10.0),
        item("g-a1", AssistantText(first.into()), BASE_TS + 30.0),
    ];
    match end {
        End::Say(t) => v.push(item("g-a2", AssistantText(t.into()), BASE_TS + 60.0)),
        End::Tool(symbol, text) => v.push(item("g-c2", ToolChips(vec![chip("g-c2a", symbol, text, "Bash", "run", Some("ok"))]), BASE_TS + 60.0)),
        End::Running(tool, cmd) => v.push(item(
            "g-c2",
            ToolChips(vec![ToolChip { id: "g-c2a".into(), symbol: "terminal".into(), text: format!("Running {tool}..."), calls: vec![ToolCall { id: "g-c2a-c".into(), tool_name: tool.into(), summary: cmd.into(), input: cmd.into(), output: None, is_error: false }] }]),
            BASE_TS + 60.0,
        )),
        End::AskQuestion => v.push(item("g-q1", ItemKind::Question(demo_question()), BASE_TS + 50.0)),
        End::Ask(cmd) => v.push(item(
            "g-p1",
            // The title the conversation parser writes for a pending Bash call (Swift `ConversationParser.verb`), so the
            // demo card reads like a real one.
            ItemKind::PermissionRequest(herdr_core::PermissionRequest { title: "Wants to run a command".into(), command: cmd.into(), tool_name: "Bash".into(), tool_use_id: "toolu_demo".into(), state: PermState::Pending, detail: String::new(), open_calls: 1 }),
            BASE_TS + 50.0,
        )),
    }
    v
}

impl DemoSource {
    pub fn new() -> DemoSource {
        let mut workspaces = Vec::new();
        let mut conversations = HashMap::new();
        let mut n = 0;
        for (wi, (wname, tabs)) in TREE.iter().enumerate() {
            let mut out = Vec::new();
            for (ti, (label, agent, status)) in tabs.iter().enumerate() {
                n += 1;
                let pane_id = format!("p{n}");
                let session = format!("demo-session-{n}");
                let pane = Pane {
                    id: pane_id.clone(),
                    terminal_id: format!("term_{n}"),
                    cwd: format!("~/{}", wname.to_lowercase()),
                    agent: Some(Agent { name: agent.to_string(), kind: "claude".into(), status: *status, session_id: Some(session), title: label.to_string() }),
                    label: None,
                };
                let conv = if *agent == "quarry-rank-dev" { ranking_conversation() } else { scripted_conversation(agent) };
                conversations.insert(pane_id, conv);
                out.push(Tab { id: format!("t{n}"), label: label.to_string(), number: ti as i64 + 1, panes: vec![pane], status: *status });
            }
            workspaces.push(Workspace { id: format!("w{}", wi + 1), label: wname.to_string(), tabs: out });
        }
        DemoSource { workspaces, conversations }
    }

    /// Workspace and tab ids of Quarry / ranking, the demo's default selection.
    pub fn default_selection(&self) -> (String, String) {
        let ws = self.workspaces.iter().find(|w| w.label == "Quarry").expect("demo workspace");
        (ws.id.clone(), ws.tabs[0].id.clone())
    }
}

impl Default for DemoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl DataSource for DemoSource {
    fn label(&self) -> String {
        "demo".into()
    }
    fn workspaces(&self) -> Vec<Workspace> {
        self.workspaces.clone()
    }
    fn connected(&self) -> bool {
        true
    }
    fn conversation(&self, pane: &Pane) -> Vec<ConversationItem> {
        self.conversations.get(&pane.id).cloned().unwrap_or_default()
    }
    fn revision(&self, _pane: &Pane) -> Option<u64> {
        // The demo data never changes.
        Some(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_core::AgentStatus::*;

    #[test]
    fn tree_shape_matches_the_reference() {
        let d = DemoSource::new();
        let ws = d.workspaces();
        assert_eq!(ws.len(), 6);
        let tabs: Vec<_> = ws.iter().flat_map(|w| &w.tabs).collect();
        assert_eq!(tabs.len(), 16);
        assert_eq!(tabs.iter().filter(|t| t.status == Blocked).count(), 2);
        assert_eq!(tabs.iter().filter(|t| t.status == Working).count(), 6);
        assert!(tabs.iter().any(|t| t.status == Done) && tabs.iter().any(|t| t.status == Idle));
    }

    #[test]
    fn ranking_conversation_is_rich() {
        let d = DemoSource::new();
        let (w, t) = d.default_selection();
        let ws = d.workspaces();
        let pane = &ws.iter().find(|x| x.id == w).unwrap().tabs.iter().find(|x| x.id == t).unwrap().panes[0];
        let items = d.conversation(pane);
        assert!(items.iter().any(|i| matches!(i.kind, ItemKind::UserText(_))));
        assert!(items.iter().any(|i| matches!(i.kind, ItemKind::ToolChips(_))));
        assert!(items.iter().any(|i| matches!(&i.kind, ItemKind::AssistantText(s) if s.contains("```"))));
        assert!(items.iter().any(|i| matches!(&i.kind, ItemKind::AssistantText(s) if s.contains("|---|"))));
        assert!(items.iter().any(|i| matches!(&i.kind, ItemKind::AssistantText(s) if s.contains("風險"))));
    }

    #[test]
    fn every_pane_has_its_own_last_line() {
        let d = DemoSource::new();
        let mut seen = std::collections::HashSet::new();
        let mut running = 0;
        for w in d.workspaces() {
            for t in &w.tabs {
                let items = d.conversation(&t.panes[0]);
                assert!(items.len() >= 3, "{} has a short conversation", t.label);
                let pv = crate::overview::preview_of(&items);
                assert!(!pv.text.is_empty());
                if pv.text.starts_with("Running") {
                    running += 1;
                }
                assert!(seen.insert(pv.text.clone()), "duplicate preview {:?}", pv.text);
            }
        }
        assert_eq!(seen.len(), 16);
        assert!(running >= 2, "a mix with running tools");
    }

    #[test]
    fn demo_data_is_public_safe() {
        let d = DemoSource::new();
        let text = format!("{:?}{:?}", d.workspaces(), d.conversations);
        for bad in [concat!("/Us", "ers/"), concat!("lim", "inchen"), concat!("alf", "red"), concat!("thought", "kit")] {
            assert!(!text.to_lowercase().contains(bad));
        }
    }
}
