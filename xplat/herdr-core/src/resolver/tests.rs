//! Ports of the HerdrKitChecks that reach `SessionResolver` (synthetic fixtures; process lists injected).

use super::*;
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

fn set(xs: &[&str]) -> HashSet<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

fn tmp() -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().to_string_lossy().into_owned();
    (t, s)
}

fn set_mtime(p: &str, t: Date) {
    let f = fs::File::options().write(true).open(p).unwrap();
    f.set_modified(SystemTime::from(t)).unwrap();
}

fn iso(t: Date) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// `EEE MMM d HH:mm:ss yyyy` in UTC, as Claude Code writes `procStart`.
fn lstart_utc(t: Date) -> String {
    t.format("%a %b %-d %H:%M:%S %Y").to_string()
}

fn rollout_name(t: Date, id: &str) -> String {
    format!("rollout-{}-{id}.jsonl", t.with_timezone(&Local).format("%Y-%m-%dT%H-%M-%S"))
}

fn proc_(pid: i32, ppid: i32, start: Date, line: &str) -> Proc {
    Proc { pid, ppid, start, line: line.into() }
}

fn resolver(dirs: impl FnOnce(&mut SessionResolver)) -> SessionResolver {
    let mut r = SessionResolver::new("/nonexistent");
    r.open_files = Box::new(|_| vec![]);
    dirs(&mut r);
    r
}

// MARK: checksSessionFallback (ChecksAgents)

#[test]
fn session_fallback_pure_parts() {
    let sock = "/home/u/.config/herdr/herdr.sock";
    let other = "/home/u/.config/herdr/sessions/t/herdr.sock";
    let ps = format!(
        "  100     1 Sat Oct 10 13:50:00 2026     -zsh HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={sock} TERM=x
  101   100 Sat Oct 10 13:55:02 2026     codex HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={sock} TERM=x
  102   101 Sat Oct 10 13:56:00 2026     /bin/zsh -c ls HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={sock}
  103   100 Sat Oct 10 13:51:00 2026     vim HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={sock}
  200     1 Sat Oct 10 13:40:00 2026     -zsh HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={other}
  201   200 Sat Oct 10 13:59:00 2026     codex HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={other}
  300     1 Sat Oct 10 13:40:00 2026     -zsh HERDR_PANE_ID=w1:p10 HERDR_SOCKET_PATH={sock}
  301   300 Sat Oct 10 14:00:00 2026     codex HERDR_PANE_ID=w1:p10 HERDR_SOCKET_PATH={sock}
  garbage line"
    );
    let procs = parse_ps(&ps);
    assert_eq!(procs.len(), 8, "resolver: ps lines parsed");
    assert_eq!(procs[2].line, format!("/bin/zsh -c ls HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={sock}"));
    assert_eq!(agent_process("w1:p1", sock, &procs).map(|p| p.pid), Some(101), "newest shell child of the pane, this session only");
    assert_eq!(agent_process("w1:p1", other, &procs).map(|p| p.pid), Some(201), "other session's pane is separate");
    assert_eq!(agent_process("w1:p10", sock, &procs).map(|p| p.pid), Some(301), "w1:p1 does not match w1:p10");
    assert_eq!(agent_process("w9:p9", sock, &procs), None, "unknown pane");
    assert_eq!(subtree(101, &procs).into_iter().collect::<HashSet<_>>(), [101, 102].into_iter().collect(), "process subtree");

    let h = home_dir();
    let ro = format!("{h}/.codex/sessions/2026/10/10/rollout-2026-10-10T13-55-04-abc.jsonl");
    assert_eq!(log_path(&[format!("{h}/.codex/logs_2.sqlite"), ro.clone(), ro.clone()], "codex", None, &h), Some(ro.clone()), "open rollout");
    assert_eq!(
        log_path(&[ro.clone(), format!("{h}/.codex/sessions/2026/10/10/rollout-2026-10-10T14-00-00-def.jsonl")], "codex", None, &h),
        None,
        "two open rollouts: no guess"
    );
    assert!(
        log_path(std::slice::from_ref(&ro), "claude", None, &h).is_none() && log_path(&[format!("{h}/.claude/projects/x/id.jsonl")], "claude", None, &h).is_some(),
        "log kind per agent"
    );
    assert!(
        is_claimed(&ro, &set(&["abc"])) && is_claimed(&ro, &set(&[&ro])) && !is_claimed(&ro, &set(&["bc"])) && !is_claimed(&ro, &set(&[])),
        "claimed by id or path"
    );

    // codex cwd + start match on a temp sessions tree (synthetic meta lines)
    let (_t, root) = tmp();
    let start = DateTime::<Utc>::from_timestamp(1_791_000_000, 0).unwrap();
    let day = day_folder(&root, local_naive(start).date());
    fs::create_dir_all(&day).unwrap();
    let roll = |dt: i64, id: &str, cwd: &str| {
        let t = start + Duration::seconds(dt);
        let p = format!("{day}/{}", rollout_name(t, id));
        let meta = format!(
            r#"{{"timestamp":"x","type":"session_meta","payload":{{"id":"{id}","timestamp":"{}","cwd":"{cwd}"}}}}"#,
            iso(t)
        ) + "\n";
        fs::write(&p, meta).unwrap();
        p
    };
    roll(-600, "old", "/w/a");
    let hit = roll(2, "hit", "/w/a");
    roll(5, "elsewhere", "/w/b");
    assert_eq!(codex_meta(&hit).map(|m| m.0), Some("/w/a".into()), "codex session_meta read");
    let now = start + Duration::seconds(60);
    assert_eq!(codex_candidates(&root, "/w/a", start, now), vec![hit.clone()], "codex cwd + start: one match");
    assert_eq!(codex_candidates(&root, "/w/a/", start, now), vec![hit.clone()], "cwd compared standardized");
    roll(30, "second", "/w/a");
    assert_eq!(codex_candidates(&root, "/w/a", start, now).len(), 2, "two matches are both returned (caller refuses)");
}

#[test]
fn parse_ps_in_a_zone() {
    let procs = parse_ps_in("  7  1 Thu Jan  1 00:00:20 2026 agy TERM=x\n", &Utc);
    assert_eq!(procs.len(), 1);
    assert_eq!(procs[0].start, DateTime::<Utc>::from_timestamp(1_767_225_620, 0).unwrap());
    assert_eq!(procs[0].line, "agy TERM=x");
    // Weekday name checked, not against the date; bad month or time: skipped.
    assert_eq!(parse_ps_in("7 1 Mon Jan 1 00:00:20 2026 x", &Utc).len(), 1);
    assert_eq!(parse_ps_in("7 1 Xyz Jan 1 00:00:20 2026 x", &Utc).len(), 0);
    assert_eq!(parse_ps_in("7 1 Thu Foo 1 00:00:20 2026 x", &Utc).len(), 0);
    assert_eq!(parse_ps_in("7 1 Thu Jan 1 00:00 2026 x", &Utc).len(), 0);
    assert_eq!(parse_ps_in("7 1 Thu Jan 1 00:00:20 2026", &Utc).len(), 0);
}

// MARK: checksTeammateSession (ChecksAgents)

const META: &str = r#"{"type":"agent-setting","sessionId":"s"}"#;
const MATE: &str = r#"{"type":"user","sessionId":"s","teamName":"team-1","agentName":"helper","message":{"role":"user","content":"hi"}}"#;
const LEAD: &str = r#"{"type":"user","sessionId":"s","message":{"role":"user","content":"hi"}}"#;

fn data(ls: &[&str]) -> Vec<u8> {
    (ls.join("\n") + "\n").into_bytes()
}

#[test]
fn teammate_verdict_pure() {
    assert_eq!(teammate_verdict(&data(&[META, MATE])), Some(true), "a message line with teamName and agentName marks a teammate log");
    assert_eq!(teammate_verdict(&data(&[META, LEAD, MATE])), Some(false), "the first message line decides (a lead log)");
    assert!(teammate_verdict(&data(&[META])).is_none() && teammate_verdict(b"").is_none(), "no message line yet: not decided");
    let cut = |s: &str| (META.to_string() + "\n" + s).into_bytes();
    let big = "x".repeat(4096);
    assert_eq!(
        teammate_verdict(&cut(
            &(r#"{"parentUuid":null,"isSidechain":false,"teamName":"team-1","agentName":"helper","type":"user","message":{"role":"user","content":""#
                .to_string()
                + &big)
        )),
        Some(true),
        "a cut first message line with teamName and agentName marks a teammate log"
    );
    assert_eq!(
        teammate_verdict(&cut(r#"{"parentUuid":null,"isSidechain":false,"teamName":"team-1""#)),
        None,
        "a first line not yet whole is not decided"
    );
}

#[test]
fn teammate_leads() {
    let (_t, root) = tmp();
    // Lead paths come back standardized: `/` on Windows too.
    let root = if cfg!(windows) { root.replace('\\', "/") } else { root };
    let projects = format!("{root}/projects");
    let dir = format!("{projects}/-w-proj");
    fs::create_dir_all(&dir).unwrap();
    let log = |id: &str, ls: &[&str]| {
        let p = format!("{dir}/{id}.jsonl");
        fs::write(&p, ls.join("\n") + "\n").unwrap();
        p
    };
    let lead1 = log("lead1", &[META, LEAD]);
    let lead2 = log("lead2", &[META, LEAD]);
    log("mate1", &[META, MATE]);
    log("new1", &[META]);
    let mut r = resolver(|r| r.projects_dir = projects.clone());
    let ask = |p: &str, s: &str| LeadAsk::new(p, s);
    let swap = |t: &str, l: Option<&str>| LeadSwap { teammate: t.into(), lead: l.map(String::from) };
    let one = |k: &str, v: LeadSwap| -> HashMap<String, LeadSwap> { [(k.to_string(), v)].into_iter().collect() };

    let cold = r.leads(&[ask("w1:p1", "mate1")], &set(&["mate1"]));
    assert!(cold.swaps == one("w1:p1", swap("mate1", None)) && cold.leads.is_empty(), "a teammate log and no lead seen yet: no log");
    let first = r.leads(&[ask("w1:p1", "lead1")], &set(&["lead1"]));
    assert!(first.swaps.is_empty() && first.leads == set(&["lead1"]), "a lead log herdr names is shown");
    assert_eq!(r.leads(&[ask("w1:p1", "mate1")], &set(&["mate1"])).swaps, one("w1:p1", swap("mate1", Some(&lead1))), "the last lead stands in");
    assert_eq!(r.leads(&[ask("w1:p1", "new1")], &set(&["new1"])).swaps, one("w1:p1", swap("new1", Some(&lead1))), "an undecided log is not shown either");
    assert_eq!(r.leads(&[ask("w1:p1", "gone1")], &set(&["gone1"])).swaps, one("w1:p1", swap("gone1", Some(&lead1))), "a log not found yet is not shown either");
    assert_eq!(
        r.leads(&[ask("w1:p1", "mate1"), ask("w1:p2", "lead1")], &set(&["mate1", "lead1"])).swaps,
        one("w1:p1", swap("mate1", None)),
        "the last lead, now another pane's (by id), is not shown twice"
    );
    r.leads(&[ask("w1:p1", "lead1")], &set(&["lead1"]));
    let other_way = format!("{root}/projects/./-w-proj/lead1.jsonl");
    assert_eq!(
        r.leads(&[ask("w1:p1", "mate1"), ask("w1:p2", "lead2")], &set(&["mate1", &other_way])).swaps["w1:p1"].lead,
        None,
        "the last lead, now another pane's (by a path written another way), is not shown twice"
    );

    // Long lines before the first message line are skipped, not undecided.
    let shape = r#"{"type":"api-request-shape","sessionId":"s","shape":""#.to_string() + &"q".repeat(100_000) + r#""}"#;
    log("longhead1", &[META, &shape, LEAD]);
    assert_eq!(r.leads(&[ask("w4:p1", "longhead1")], &set(&["longhead1"])).leads, set(&["longhead1"]), "long lines then a lead line");
    log("longmate1", &[META, &shape, MATE]);
    assert_eq!(r.leads(&[ask("w4:p1", "longmate1")], &set(&["longmate1"])).swaps["w4:p1"].teammate, "longmate1", "long lines then a teammate line");

    // A verdict holds for one file: a log replaced, or rewritten shorter, is judged again.
    r.leads(&[ask("w2:p1", "lead2")], &set(&["lead2"]));
    let swap_file = lead2.clone() + ".new";
    fs::write(&swap_file, data(&[META, MATE])).unwrap();
    fs::rename(&swap_file, &lead2).unwrap();
    assert_eq!(r.leads(&[ask("w2:p1", "lead2")], &set(&["lead2"])).swaps["w2:p1"].teammate, "lead2", "a lead log replaced by a teammate's is judged again");
    let shrink = log("shrink1", &[META, &(LEAD.to_string() + &" ".repeat(400))]);
    r.leads(&[ask("w3:p1", "shrink1")], &set(&["shrink1"]));
    fs::write(&shrink, data(&[META, MATE])).unwrap(); // same file, truncated and rewritten
    assert_eq!(r.leads(&[ask("w3:p1", "shrink1")], &set(&["shrink1"])).swaps["w3:p1"].teammate, "shrink1", "a log rewritten shorter is judged again");

    // The last lead is judged again before it stands in.
    let back = log("back1", &[META, LEAD]);
    r.leads(&[ask("w5:p1", "back1")], &set(&["back1"]));
    fs::write(back.clone() + ".new", data(&[META, MATE])).unwrap();
    fs::rename(back.clone() + ".new", &back).unwrap();
    assert_eq!(r.leads(&[ask("w5:p1", "mate1")], &set(&["mate1"])).swaps["w5:p1"].lead, None, "a last lead that is now a teammate's log is not shown");

    // Grown, then cut back shorter (but longer than at first): judged again.
    let grown = log("grown1", &[META, LEAD]);
    r.leads(&[ask("w6:p1", "grown1")], &set(&["grown1"]));
    fs::write(&grown, [data(&[META, LEAD]), " ".repeat(10_000).into_bytes()].concat()).unwrap();
    r.leads(&[ask("w6:p1", "grown1")], &set(&["grown1"]));
    fs::write(&grown, [data(&[META, MATE]), " ".repeat(1000).into_bytes()].concat()).unwrap();
    assert_eq!(r.leads(&[ask("w6:p1", "grown1")], &set(&["grown1"])).swaps["w6:p1"].teammate, "grown1", "a log cut shorter than when last seen is judged again");

    // An undecided log is read again only when it changes; one undecided after the read limit stays so.
    let mut rc = resolver(|r| {
        r.projects_dir = projects.clone();
        r.verdict_cap = 1024;
    });
    let waiting = log("wait1", &[META]);
    rc.leads(&[ask("w7:p1", "wait1")], &set(&[]));
    rc.leads(&[ask("w7:p1", "wait1")], &set(&[]));
    assert_eq!(rc.verdict_reads(), 1, "an unchanged undecided log is not read again");
    append(&waiting, &(LEAD.to_string() + "\n"));
    assert_eq!(rc.leads(&[ask("w7:p1", "wait1")], &set(&[])).leads, set(&["wait1"]), "an undecided log that grew is read again");
    let long_pre = log("long1", &[META, &(r#"{"type":"api-request-shape","shape":""#.to_string() + &"q".repeat(4000) + r#""}"#)]);
    rc.leads(&[ask("w7:p2", "long1")], &set(&[]));
    let before = rc.verdict_reads();
    append(&long_pre, &(LEAD.to_string() + "\n"));
    let pass = rc.leads(&[ask("w7:p2", "long1")], &set(&[]));
    assert!(
        pass.swaps["w7:p2"] == swap("long1", None) && pass.leads.is_empty() && rc.verdict_reads() == before,
        "a log undecided after the read limit stays undecided and is not read again"
    );
}

fn append(p: &str, s: &str) {
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(p).unwrap().write_all(s.as_bytes()).unwrap();
}

// MARK: checksContinuedIn (the resolver part)

#[test]
fn continued_teammate_log_is_a_lead() {
    let (_t, root) = tmp();
    let projects = format!("{root}/projects");
    let dir = format!("{projects}/-w-proj");
    fs::create_dir_all(&dir).unwrap();
    let next = |from: &str, to: &str| {
        format!(r#"{{"type":"continued-in","timestamp":"2026-10-30T06:54:55.989Z","sessionId":"{from}","continuedInSessionId":"{to}"}}"#)
    };
    fs::write(format!("{dir}/old1.jsonl"), data(&[MATE, &next("old1", "mid1")])).unwrap();
    fs::write(format!("{dir}/mid1.jsonl"), data(&[LEAD, &next("mid1", "new1")])).unwrap();
    fs::write(format!("{dir}/new1.jsonl"), data(&[LEAD])).unwrap();
    let mut r = resolver(|r| r.projects_dir = projects.clone());
    let pass = r.leads(&[LeadAsk::new("w1:p1", "old1")], &set(&["old1"]));
    assert!(pass.swaps.is_empty() && pass.leads == set(&["old1"]), "a teammate's old log continued as a lead's is shown");
}

// MARK: checksCodexForeignClaim (ChecksCodexClaim)

#[test]
fn codex_foreign_claim() {
    let (_t, root) = tmp();
    let start = Utc::now() - Duration::seconds(120);
    let t = start + Duration::seconds(5);
    let day = day_folder(&root, local_naive(t).date());
    fs::create_dir_all(&day).unwrap();
    let id = "01a192a3-11b5-78a2-9440-4e4d9874a762";
    let path = format!("{day}/{}", rollout_name(t, id));
    let meta = format!(r#"{{"timestamp":"x","type":"session_meta","payload":{{"id":"{id}","timestamp":"{}","cwd":"/w/desk"}}}}"#, iso(t)) + "\n";
    fs::write(&path, meta).unwrap();
    let procs = vec![proc_(900_001, 1, start, "node /opt/bin/codex HC_TERMINAL_ID=t3")];
    let need = Need::new("hc:t3", "codex", "/w/desk", vec![900_001]);
    let resolve = |claimed_cwd: &[(&str, &str)]| {
        let mut r = resolver(|r| r.codex_dir = root.clone());
        let cc: HashMap<String, String> = claimed_cwd.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        r.resolve_with(std::slice::from_ref(&need), &set(&[id]), &cc, Some(&procs)).get("hc:t3").cloned()
    };
    assert_eq!(resolve(&[(id, "/w/other/in-ac")]), Some(path), "a claim from a pane in another cwd does not hide the matching rollout");
    assert_eq!(resolve(&[(id, "/w/desk")]), None, "a claim from a pane in the rollout's cwd still counts");
    assert_eq!(resolve(&[]), None, "a claim without a known cwd still counts");
}

// MARK: checksPiSession, checksPiTwoInOneCwd, checksHCResolverNewProgram (ChecksPiSession)

fn pi_head(id: &str, started: Date, cwd: &str) -> String {
    format!(r#"{{"type":"session","version":3,"id":"{id}","timestamp":"{}","cwd":"{cwd}"}}"#, iso(started))
}

#[test]
fn pi_session() {
    assert_eq!(pi_session_dir("/private/tmp/c9-pi-live-1", "/h/.pi/agent"), "/h/.pi/agent/sessions/--private-tmp-c9-pi-live-1--", "session folder named as pi names it");
    assert_eq!(pi_session_dir("/a/b:c", "/x"), "/x/sessions/--a-b-c--", "':' becomes '-' too");

    let (_t, agent_dir) = tmp();
    let cwd = "/w/pi-proj";
    let dir = pi_session_dir(cwd, &agent_dir);
    fs::create_dir_all(&dir).unwrap();
    let start = Utc::now() - Duration::seconds(600);
    let file = |name: &str, started: Date, c: &str, modified: Date| {
        let p = format!("{dir}/{name}.jsonl");
        fs::write(&p, pi_head(name, started, c) + "\n").unwrap();
        set_mtime(&p, modified);
        p
    };
    let s = |n: i64| start + Duration::seconds(n);
    let old = file("old", s(-3600), cwd, s(-3000));
    let mine = file("mine", s(3), cwd, s(60));
    file("elsewhere", s(5), "/w/other", s(90));
    let taken = file("taken", s(4), cwd, s(120));

    let procs = vec![proc_(910_001, 1, start, "node /opt/homebrew/bin/pi HC_TERMINAL_ID=t6")];
    let need = Need::new("hc:t6", "pi", cwd, vec![910_001]);
    let resolve = |claimed: &[&str]| {
        let mut r = resolver(|r| r.pi_agent_dir = agent_dir.clone());
        r.resolve_with(std::slice::from_ref(&need), &set(claimed), &HashMap::new(), Some(&procs)).get("hc:t6").cloned()
    };
    assert_eq!(resolve(&[&taken]), Some(mine.clone()), "the newest unclaimed file of its cwd started after the agent");
    assert_eq!(resolve(&[]), Some(taken.clone()), "newest first when nothing is claimed (/new makes a newer file)");
    fs::remove_file(&mine).unwrap();
    fs::remove_file(&taken).unwrap();
    assert_eq!(resolve(&[]), None, "an old file pi did not touch is not used");
    set_mtime(&old, s(30));
    assert_eq!(resolve(&[]), Some(old), "a resumed session (written after the agent started) is used");

    let lines = vec![
        ("/r/a.jsonl".to_string(), s(50), format!(r#"{{"type":"session","timestamp":"{}","cwd":"/r/w"}}"#, iso(s(2))).into_bytes()),
        ("/r/b.jsonl".to_string(), s(80), br#"{"type":"model_change"}"#.to_vec()),
    ];
    assert_eq!(pi_pick(&lines, "/r/w", start, &[], |_| false), Some("/r/a.jsonl".into()), "pick reads only session headers");
}

#[test]
fn pi_two_in_one_cwd() {
    let (_t, agent_dir) = tmp();
    let cwd = "/w/two";
    let dir = pi_session_dir(cwd, &agent_dir);
    fs::create_dir_all(&dir).unwrap();
    let a0 = Utc::now() - Duration::seconds(900);
    let b0 = a0 + Duration::seconds(300);
    let file = |id: &str, started: Date, modified: Date| {
        let p = format!("{dir}/{id}.jsonl");
        fs::write(&p, pi_head(id, started, cwd) + "\n").unwrap();
        set_mtime(&p, modified);
        p
    };
    let fa = file("a", a0 + Duration::seconds(1), b0 + Duration::seconds(100));
    let fb = file("b", b0 + Duration::seconds(1), b0 + Duration::seconds(200));
    let procs = vec![proc_(920_001, 1, a0, "node pi"), proc_(920_002, 1, b0, "node pi")];
    let na = Need::new("hc:ta", "pi", cwd, vec![920_001]);
    let nb = Need::new("hc:tb", "pi", cwd, vec![920_002]);
    for order in [vec![na.clone(), nb.clone()], vec![nb.clone(), na.clone()]] {
        let mut r = resolver(|r| r.pi_agent_dir = agent_dir.clone());
        let got = r.resolve_with(&order, &set(&[]), &HashMap::new(), Some(&procs));
        assert!(got.get("hc:ta") == Some(&fa) && got.get("hc:tb") == Some(&fb), "each pane gets its own pi's file ({got:?})");
    }
    fs::remove_file(&fa).unwrap();
    let mut r = resolver(|r| r.pi_agent_dir = agent_dir.clone());
    let got = r.resolve_with(&[na, nb], &set(&[]), &HashMap::new(), Some(&procs));
    assert!(!got.contains_key("hc:ta") && got.get("hc:tb") == Some(&fb), "a later pi's file is not taken by the earlier one ({got:?})");
    let lines = vec![
        ("/r/a".to_string(), b0 + Duration::seconds(100), pi_head("a", a0 + Duration::seconds(1), cwd).into_bytes()),
        ("/r/b".to_string(), b0 + Duration::seconds(200), pi_head("b", b0 + Duration::seconds(1), cwd).into_bytes()),
    ];
    assert!(
        pi_pick(&lines, cwd, a0, &[b0], |_| false) == Some("/r/a".into()) && pi_pick(&lines, cwd, b0, &[a0], |_| false) == Some("/r/b".into()),
        "pick with rivals"
    );
}

#[test]
fn hc_resolver_new_program() {
    let (_t, agent_dir) = tmp();
    let cwd = "/w/pi-new";
    let dir = pi_session_dir(cwd, &agent_dir);
    fs::create_dir_all(&dir).unwrap();
    let file = |name: &str, started: Date| {
        let p = format!("{dir}/{name}.jsonl");
        fs::write(&p, pi_head(name, started, cwd) + "\n").unwrap();
        set_mtime(&p, started + Duration::seconds(5));
        p
    };
    let first = Utc::now() - Duration::seconds(600);
    let second = Utc::now() - Duration::seconds(60);
    let one = file("one", first + Duration::seconds(2));
    let mut r = resolver(|r| r.pi_agent_dir = agent_dir.clone());
    let a = vec![proc_(930_001, 1, first, "node /opt/homebrew/bin/pi HC_TERMINAL_ID=t7")];
    let found = r.resolve_with(&[Need::new("hc:t7", "pi", cwd, vec![930_001])], &set(&[]), &HashMap::new(), Some(&a));
    assert_eq!(found.get("hc:t7"), Some(&one), "the first pi's file");
    let two = file("two", second + Duration::seconds(2));
    let b = vec![proc_(930_002, 1, second, "node /opt/homebrew/bin/pi HC_TERMINAL_ID=t7")];
    let again = r.resolve_with(&[Need::new("hc:t7", "pi", cwd, vec![930_002])], &set(&[]), &HashMap::new(), Some(&b));
    assert_eq!(again.get("hc:t7"), Some(&two), "a new pid in the pane -> its log at once, not the old one");
}

// omp (oh-my-pi, a pi fork): pi's files under ~/.omp/agent/sessions, folders named by session-paths.ts.

#[test]
fn omp_session_folder_names() {
    let d = |cwd: &str, home: &str, tmp: &str| omp_session_dir(cwd, "/s", home, tmp);
    assert_eq!(d("/home/u/src/app", "/home/u", "/tmp"), "/s/-src-app", "under home: '-' + the path from home");
    assert_eq!(d("/home/u", "/home/u", "/tmp"), "/s/-", "home itself");
    assert_eq!(d("/tmp/x/y", "/home/u", "/tmp"), "/s/-tmp-x-y", "under the temp root: '-tmp-' + the path from it");
    assert_eq!(d("/tmp", "/home/u", "/tmp"), "/s/-tmp", "the temp root itself");
    assert_eq!(d("/home/u/.t/a", "/home/u", "/home/u/.t"), "/s/-tmp-a", "the temp root wins when it is inside home");
    assert_eq!(d("/srv/app", "/home/u", "/tmp"), "/s/--srv-app--", "elsewhere: the absolute path in '--'");
    assert_eq!(
        d("/private/tmp/app", "/home/u", "/private/var/folders/ab/T"),
        "/s/--private-tmp-app--",
        "macOS: /tmp is /private/tmp, outside TMPDIR"
    );
    assert_eq!(d("/home/u/a:b\\c", "/home/u", "/tmp"), "/s/-a-b-c", "':' and '\\' become '-'");
    assert_eq!(d("/home/user2/x", "/home/u", "/tmp"), "/s/--home-user2-x--", "a sibling of home is not under home");
    // Windows: `\` separates too (path.relative); the drive's ':' becomes '-'.
    assert_eq!(d(r"C:\Users\u\src\app", r"C:\Users\u", r"C:\T"), "/s/-src-app", "Windows: under home");
    assert_eq!(d(r"C:\T\x", r"C:\Users\u", r"C:\T"), "/s/-tmp-x", "Windows: under the temp root");
    assert_eq!(d(r"D:\w\x", r"C:\Users\u", r"C:\T"), "/s/--D--w-x--", "Windows: elsewhere");
}

#[test]
fn verbatim_prefix_dropped() {
    // Windows canonicalize gives \\?\C:\...; Node's realpath gives C:\...
    assert_eq!(strip_verbatim(r"\\?\C:\Users\u"), r"C:\Users\u");
    assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\a"), r"\\srv\share\a");
    assert_eq!(strip_verbatim("/home/u"), "/home/u");
}

#[test]
fn omp_title_line_first() {
    let start = Utc::now() - Duration::seconds(600);
    let s = |n: i64| start + Duration::seconds(n);
    let title = r#"{"type":"title","v":1,"title":"Fix the build","source":"auto","updatedAt":"2026-11-10T07:35:44.761Z","pad":"    "}"#;
    let lines = vec![
        ("/r/a.jsonl".to_string(), s(50), format!("{title}\n{}\n", pi_head("a", s(2), "/r/w")).into_bytes()),
        ("/r/b.jsonl".to_string(), s(80), format!("{title}\n").into_bytes()),
    ];
    assert_eq!(pi_pick(&lines, "/r/w", start, &[], |_| false), Some("/r/a.jsonl".into()), "the session header after a title slot");
}

#[test]
fn omp_session() {
    let (_t, agent_dir) = tmp();
    let start = Utc::now() - Duration::seconds(600);
    let write = |dir: &str, name: &str, cwd: &str| {
        fs::create_dir_all(dir).unwrap();
        let p = format!("{dir}/{name}.jsonl");
        let title = r#"{"type":"title","v":1,"title":"t","source":"auto","updatedAt":"2026-11-10T07:35:44.761Z","pad":"  "}"#;
        fs::write(&p, format!("{title}\n{}\n", pi_head(name, start + Duration::seconds(3), cwd))).unwrap();
        set_mtime(&p, start + Duration::seconds(60));
        p
    };
    // A cwd outside home and temp, sessions under the default root.
    let cwd = "/w/omp-proj";
    let root = agent_dir.clone() + "/default/sessions";
    let mine = write(&format!("{root}/--w-omp-proj--"), "2026-11-10T07-35-44-761Z_a", cwd);
    write(&agent_dir.clone(), "decoy", cwd);
    let procs = vec![proc_(940_001, 1, start, "omp HC_TERMINAL_ID=t8")];
    let need = Need::new("hc:t8", "omp", cwd, vec![940_001]);
    let mut r = resolver(|r| r.omp_sessions_dir = root.clone());
    let got = r.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs));
    assert_eq!(got.get("hc:t8"), Some(&mine), "omp's file in its cwd's folder");

    // PI_CODING_AGENT_DIR in the process env moves the root; a cwd in the temp root gets the -tmp- name.
    let (_w, work) = tmp();
    let real = fs::canonicalize(&work).unwrap().to_string_lossy().into_owned();
    let base = Path::new(&real).file_name().unwrap().to_string_lossy().into_owned();
    let moved = agent_dir.clone() + "/moved";
    let other = write(&format!("{moved}/sessions/-tmp-{base}"), "2026-11-10T07-35-44-761Z_b", &work);
    let line = format!("bun /usr/local/bin/omp PI_CODING_AGENT_DIR={moved} HC_TERMINAL_ID=t9");
    let procs = vec![proc_(940_002, 1, start, &line)];
    let need = Need::new("hc:t9", "omp", &work, vec![940_002]);
    let mut r = resolver(|r| r.omp_sessions_dir = root.clone());
    let got = r.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs));
    assert_eq!(
        got.get("hc:t9"),
        Some(&other),
        "agent dir from the process env, folder from the real cwd (dir {}, real cwd {}, real tmp {})",
        r.pi_dir("omp", &work, &line),
        real_path(&work),
        real_path(&std::env::temp_dir().to_string_lossy())
    );
}

#[test]
fn omp_env_values_with_spaces() {
    let line = "omp --x PI_CODING_AGENT_DIR=/data/omp profiles/work TMPDIR=/t/my tmp HC_TERMINAL_ID=t1";
    assert_eq!(env_value(line, "PI_CODING_AGENT_DIR").as_deref(), Some("/data/omp profiles/work"), "a value runs to the next NAME=");
    assert_eq!(env_value(line, "TMPDIR").as_deref(), Some("/t/my tmp"));
    assert_eq!(env_value(line, "HC_TERMINAL_ID").as_deref(), Some("t1"), "the last value runs to the end");
    assert_eq!(env_value("omp PI_CODING_AGENT_DIR= HOME=/h", "PI_CODING_AGENT_DIR"), None, "empty is unset");
    assert_eq!(env_value("omp XPI_CODING_AGENT_DIR=/a", "PI_CODING_AGENT_DIR"), None, "whole names only");
}

#[test]
fn pi_folder_shared_by_two_cwds() {
    // /w/a-b and /w/a/b both name the folder --w-a-b--: the two pi are not rivals.
    let (_t, agent_dir) = tmp();
    let dir = pi_session_dir("/w/a-b", &agent_dir);
    assert_eq!(dir, pi_session_dir("/w/a/b", &agent_dir));
    fs::create_dir_all(&dir).unwrap();
    let a0 = Utc::now() - Duration::seconds(900);
    let b0 = a0 + Duration::seconds(300);
    let file = |id: &str, started: Date, cwd: &str| {
        let p = format!("{dir}/{id}.jsonl");
        fs::write(&p, pi_head(id, started, cwd) + "\n").unwrap();
        set_mtime(&p, started + Duration::seconds(10));
        p
    };
    let fa = file("a", b0 + Duration::seconds(100), "/w/a-b");
    let fb = file("b", b0 + Duration::seconds(1), "/w/a/b");
    let procs = vec![proc_(960_001, 1, a0, "node pi"), proc_(960_002, 1, b0, "node pi")];
    let na = Need::new("hc:ta", "pi", "/w/a-b", vec![960_001]);
    let nb = Need::new("hc:tb", "pi", "/w/a/b", vec![960_002]);
    let mut r = resolver(|r| r.pi_agent_dir = agent_dir.clone());
    let got = r.resolve_with(&[na, nb], &set(&[]), &HashMap::new(), Some(&procs));
    assert!(got.get("hc:ta") == Some(&fa) && got.get("hc:tb") == Some(&fb), "each pi keeps its own cwd's file ({got:?})");
}

#[test]
fn omp_relative_agent_dir() {
    // PI_CODING_AGENT_DIR=./.omp-agent is resolved from omp's cwd, as omp's path.resolve does.
    let start = Utc::now() - Duration::seconds(600);
    let (_w, work) = tmp();
    let real = fs::canonicalize(&work).unwrap().to_string_lossy().into_owned();
    let dir = format!("{work}/.omp-agent/sessions/-tmp-{}", Path::new(&real).file_name().unwrap().to_string_lossy());
    fs::create_dir_all(&dir).unwrap();
    let p = format!("{dir}/2026-11-10T07-35-44-761Z_c.jsonl");
    fs::write(&p, pi_head("c", start + Duration::seconds(3), &work) + "\n").unwrap();
    set_mtime(&p, start + Duration::seconds(60));
    let procs = vec![proc_(970_001, 1, start, "omp PI_CODING_AGENT_DIR=./.omp-agent HC_TERMINAL_ID=t3")];
    let need = Need::new("hc:t3", "omp", &work, vec![970_001]);
    let mut r = resolver(|r| r.omp_sessions_dir = "/nonexistent/sessions".into());
    let got = r.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs));
    assert_eq!(
        got.get("hc:t3"),
        Some(&p),
        "a relative agent dir under the agent's cwd (dir {})",
        r.pi_dir("omp", &work, "omp PI_CODING_AGENT_DIR=./.omp-agent")
    );
}

#[test]
fn omp_two_in_one_cwd_apart() {
    // Two omp in one cwd with their own PI_CODING_AGENT_DIR: different folders, so no rivals.
    let (_t, base) = tmp();
    let cwd = "/w/omp-two";
    let a0 = Utc::now() - Duration::seconds(900);
    let b0 = a0 + Duration::seconds(300);
    let file = |agent: &str, id: &str, started: Date| {
        let dir = format!("{base}/{agent}/sessions/--w-omp-two--");
        fs::create_dir_all(&dir).unwrap();
        let p = format!("{dir}/{id}.jsonl");
        fs::write(&p, pi_head(id, started, cwd) + "\n").unwrap();
        set_mtime(&p, started + Duration::seconds(10));
        p
    };
    // A's /new after B started: a header later than B's start, in A's own folder.
    let fa = file("a", "a", b0 + Duration::seconds(100));
    let fb = file("b", "b", b0 + Duration::seconds(1));
    let procs = vec![
        proc_(950_001, 1, a0, &format!("omp PI_CODING_AGENT_DIR={base}/a HC_TERMINAL_ID=ta")),
        proc_(950_002, 1, b0, &format!("omp PI_CODING_AGENT_DIR={base}/b HC_TERMINAL_ID=tb")),
    ];
    let na = Need::new("hc:ta", "omp", cwd, vec![950_001]);
    let nb = Need::new("hc:tb", "omp", cwd, vec![950_002]);
    let mut r = resolver(|r| r.omp_sessions_dir = format!("{base}/default/sessions"));
    let got = r.resolve_with(&[na, nb], &set(&[]), &HashMap::new(), Some(&procs));
    assert!(got.get("hc:ta") == Some(&fa) && got.get("hc:tb") == Some(&fb), "each omp reads its own folder ({got:?})");
}

// MARK: checksGrok (resolver part)

#[test]
fn grok_resolver() {
    let sid = "00000000-0000-7000-8000-000000000001";
    let h = home_dir();
    let open = vec![
        format!("{h}/.grok/logs/unified.jsonl"),
        format!("{h}/.grok/sessions/%2Fw%2Fa/{sid}/events.jsonl"),
        format!("{h}/.grok/memtrace/1-2.jsonl"),
    ];
    assert_eq!(log_path(&open, "grok", None, &h), Some(format!("{h}/.grok/sessions/%2Fw%2Fa/{sid}/updates.jsonl")), "open events.jsonl names the folder");
    let mut two = open.clone();
    two.push(format!("{h}/.grok/sessions/%2Fw%2Fb/s2/events.jsonl"));
    assert_eq!(log_path(&two, "grok", None, &h), None, "two sessions: no guess");
    let g = vec!["/g/sessions/x/s3/events.jsonl".to_string()];
    assert!(
        log_path(&g, "grok", Some("/g"), &h) == Some("/g/sessions/x/s3/updates.jsonl".into()) && log_path(&g, "grok", None, &h).is_none(),
        "GROK_HOME of the agent"
    );
    assert_eq!(log_path(&open, "claude", None, &h), None, "not for other agents");
    assert!(grok_home("grok --cwd /w GROK_HOME=/g TERM=x") == "/g" && grok_home("grok TERM=x") == h.clone() + "/.grok", "GROK_HOME from the agent's env");
    let active = format!(r#"[{{"session_id":"s-old","pid":7,"cwd":"/w"}},{{"session_id":"{sid}","pid":42,"cwd":"/w"}}]"#);
    assert!(
        grok_session(active.as_bytes(), &[41, 42]) == Some(sid.into())
            && grok_session(active.as_bytes(), &[40]).is_none()
            && grok_session(b"x", &[42]).is_none(),
        "active session by pid"
    );
    assert!(is_claimed(&format!("{h}/.grok/sessions/%2Fw%2Fa/{sid}/updates.jsonl"), &set(&[sid])), "claimed by the folder's id");
}

/// Grok through `resolve`: the agent's GROK_HOME, `active_sessions.json` by pid (nothing open).
#[test]
fn grok_resolve_active_sessions() {
    let (_t, g) = tmp();
    let sid = "00000000-0000-7000-8000-000000000002";
    fs::create_dir_all(format!("{g}/sessions/%2Fw/{sid}")).unwrap();
    fs::write(format!("{g}/sessions/%2Fw/{sid}/updates.jsonl"), "").unwrap();
    fs::write(format!("{g}/active_sessions.json"), format!(r#"[{{"session_id":"{sid}","pid":51}}]"#)).unwrap();
    let procs = vec![proc_(50, 1, Utc::now(), &format!("grok GROK_HOME={g} TERM=x")), proc_(51, 50, Utc::now(), "grok-worker")];
    let mut r = resolver(|_| {});
    let got = r.resolve_with(&[Need::new("hc:t1", "grok", "/w", vec![50])], &set(&[]), &HashMap::new(), Some(&procs));
    assert_eq!(got.get("hc:t1"), Some(&format!("{g}/sessions/%2Fw/{sid}/updates.jsonl")));
}

// MARK: checksAgyResolver (ChecksAgy)

#[test]
fn agy_resolver() {
    let (_t, root) = tmp();
    let id = "00000000-0000-4000-8000-000000000001";
    let id2 = "00000000-0000-4000-8000-000000000002";
    let logs = format!("{root}/brain/{id}/.system_generated/logs");
    fs::create_dir_all(&logs).unwrap();
    fs::write(format!("{logs}/transcript_full.jsonl"), "").unwrap();
    let lock = format!("{root}/presence/{id}.lock");
    let lock2 = format!("{root}/presence/{id2}.lock");
    let open = vec![format!("{root}/log/cli-20260101_000000.log"), format!("{root}/settings.json"), format!("{root}/brain/{id}"), lock.clone()];
    assert_eq!(agy_log_path(&open, &root), Some(format!("{logs}/transcript_full.jsonl")), "open presence lock names the transcript");
    let mut both = open.clone();
    both.push(lock2.clone());
    assert_eq!(agy_log_path(&both, &root), None, "two locks: no guess");
    assert_eq!(agy_log_path(std::slice::from_ref(&lock2), &root), None, "no transcript yet: nil");
    assert_eq!(agy_log_path(&open[..3], &root), None, "no lock (no conversation yet): nil");
    assert_eq!(agy_log_path(&[format!("{root}/presence/../x.lock"), format!("{root}/presence/not-a-uuid.lock")], &root), None, "odd lock names ignored");
    assert_eq!(
        agy_log_path(&[format!("{root}/presence/{}.lock", id.to_uppercase())], &root),
        Some(format!("{logs}/transcript_full.jsonl")),
        "an upper-case id is lower-cased"
    );

    let procs = parse_ps("  10     1 Thu Jan  1 00:00:00 2026     -zsh TERM=x\n  11    10 Thu Jan  1 00:00:20 2026     agy TERM=x\n");
    assert_eq!(hc_agent_process(&[10], "agy", &procs).map(|p| p.pid), Some(11), "hc finds the agy process under the shell");

    let (_t2, dir) = tmp();
    let transcript = |cid: &str| {
        let l = format!("{dir}/brain/{cid}/.system_generated/logs");
        fs::create_dir_all(&l).unwrap();
        fs::write(format!("{l}/transcript_full.jsonl"), "").unwrap();
        format!("{l}/transcript_full.jsonl")
    };
    let path_a = transcript(id);
    let path_b = transcript(id2);
    let held = Rc::new(RefCell::new(vec![format!("{dir}/presence/{id}.lock")]));
    let h2 = held.clone();
    let mut res = resolver(|r| {
        r.agy_dir = dir.clone();
        r.recheck = 0.0;
        r.open_files = Box::new(move |_| h2.borrow().clone());
    });
    let need = Need::new("hc:t1", "agy", "/tmp", vec![10]);
    assert_eq!(res.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs)).get("hc:t1"), Some(&path_a), "follows the open lock");
    *held.borrow_mut() = vec![format!("{dir}/presence/{id2}.lock")];
    assert_eq!(
        res.resolve_with(&[need], &set(&[]), &HashMap::new(), Some(&procs)).get("hc:t1"),
        Some(&path_b),
        "same pid, new lock id: the recheck switches to the new transcript"
    );
}

// MARK: checksHCSessions (ChecksHC), checksHCEdgeClaudeClear (ChecksHCEdges), checksHCInputStamp (ChecksHCInput)

fn write_pid_file(dir: &str, pid: i32, sid: &str, start: Date) -> String {
    let p = format!("{dir}/{pid}.json");
    let o = serde_json::json!({"pid": pid, "sessionId": sid, "procStart": lstart_utc(start), "cwd": "/tmp"});
    fs::write(&p, serde_json::to_vec(&o).unwrap()).unwrap();
    p
}

#[test]
fn hc_sessions() {
    let procs = parse_ps(
        "  500     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t1 TERM=xterm
  501   500 Sat Oct 31 10:01:00 2026     claude HC_TERMINAL_ID=t1
  502   501 Sat Oct 31 10:02:00 2026     /bin/zsh -c ls
  600     1 Sat Oct 31 10:00:00 2026     node /opt/bin/codex HC_TERMINAL_ID=t2
  601   600 Sat Oct 31 10:05:00 2026     rg foo
  700     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t3
  701   700 Sat Oct 31 10:09:00 2026     claude HC_TERMINAL_ID=t3",
    );
    let (_t, dir) = tmp();
    let start501 = procs.iter().find(|p| p.pid == 501).unwrap().start;
    write_pid_file(&dir, 501, "sid-501", start501);
    write_pid_file(&dir, 701, "sid-stale", start501); // pid reused: procStart does not match 701's start
    assert_eq!(claude_session(&[501, 500], &procs, &dir), Some("sid-501".into()), "<pid>.json of the foreground process");
    assert_eq!(claude_session(&[500], &procs, &dir), Some("sid-501".into()), "found in the terminal's process tree");
    assert_eq!(claude_session(&[701, 700], &procs, &dir), None, "stale <pid>.json (other process start) is ignored");
    assert!(claude_session(&[], &procs, &dir).is_none() && claude_session(&[999], &procs, &dir).is_none(), "no pid, no guess");
    assert_eq!(hc_agent_process(&[600], "codex", &procs).map(|p| p.pid), Some(600), "the terminal process runs codex itself");
    assert_eq!(hc_agent_process(&[500], "claude", &procs).map(|p| p.pid), Some(501), "newest descendant running the agent");
    assert_eq!(hc_agent_process(&[601, 600], "codex", &procs).map(|p| p.pid), Some(600), "a tool child in the foreground still maps to codex");

    let need = Need::new("hc:t1", "claude", "/tmp", vec![501, 500]);
    let mut res = resolver(|r| r.claude_dir = dir.clone());
    assert_eq!(res.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs)).get("hc:t1"), Some(&"sid-501".to_string()), "hc need resolved by pid");
    let mut res2 = resolver(|r| r.claude_dir = dir.clone());
    assert!(!res2.resolve_with(&[need], &set(&["sid-501"]), &HashMap::new(), Some(&procs)).contains_key("hc:t1"), "a claimed session is not handed out twice");
}

#[test]
fn hc_claude_config_dir() {
    let (_t, cfg) = tmp();
    let (_d, default_dir) = tmp();
    fs::create_dir_all(format!("{cfg}/sessions")).unwrap();
    fs::create_dir_all(format!("{cfg}/projects/-w")).unwrap();
    let log = format!("{cfg}/projects/-w/sid-cfg.jsonl");
    fs::write(&log, "{}\n").unwrap();
    let procs = parse_ps(&format!(
        "  500     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t1\n  501   500 Sat Oct 31 10:01:00 2026     claude CLAUDE_CONFIG_DIR={cfg} HC_TERMINAL_ID=t1\n"
    ));
    let start = procs.iter().find(|p| p.pid == 501).unwrap().start;
    write_pid_file(&format!("{cfg}/sessions"), 501, "sid-cfg", start);
    let mut res = resolver(|r| r.claude_dir = default_dir.clone());
    let need = Need::new("hc:t1", "claude", "/w", vec![501, 500]);
    assert_eq!(
        res.resolve_with(&[need], &set(&[]), &HashMap::new(), Some(&procs)).get("hc:t1"),
        Some(&log),
        "CLAUDE_CONFIG_DIR of the agent: its pid file and its log"
    );
}

#[test]
fn hc_claude_config_dir_log_later() {
    let (_t, cfg) = tmp();
    let (_d, default_dir) = tmp();
    fs::create_dir_all(format!("{cfg}/sessions")).unwrap();
    let procs = parse_ps(&format!(
        "  500     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t1\n  501   500 Sat Oct 31 10:01:00 2026     claude CLAUDE_CONFIG_DIR={cfg} HC_TERMINAL_ID=t1\n"
    ));
    let start = procs.iter().find(|p| p.pid == 501).unwrap().start;
    write_pid_file(&format!("{cfg}/sessions"), 501, "sid-new", start);
    let mut res = resolver(|r| {
        r.claude_dir = default_dir.clone();
        r.retry = 0.0;
    });
    let need = Need::new("hc:t1", "claude", "/w", vec![501, 500]);
    let get = |res: &mut SessionResolver| res.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs)).get("hc:t1").cloned();
    assert_eq!(get(&mut res), Some("sid-new".into()), "fresh Claude, no turn yet: the bare session id, no log yet");
    fs::create_dir_all(format!("{cfg}/projects/-w")).unwrap();
    let log = format!("{cfg}/projects/-w/sid-new.jsonl");
    fs::write(&log, "{}\n").unwrap();
    assert_eq!(get(&mut res), Some(log), "the first turn writes the log under the config dir: the same pid file now gives its path");
}

#[test]
fn hc_edge_claude_clear() {
    let procs = parse_ps("  500     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t1\n  501   500 Sat Oct 31 10:01:00 2026     claude HC_TERMINAL_ID=t1\n");
    let (_t, dir) = tmp();
    let start = procs.iter().find(|p| p.pid == 501).unwrap().start;
    let write = |sid: &str, m: Date| {
        let p = write_pid_file(&dir, 501, sid, start);
        set_mtime(&p, m);
    };
    write("sid-a", Utc::now() - Duration::seconds(60));
    let mut res = resolver(|r| r.claude_dir = dir.clone());
    let need = Need::new("hc:t1", "claude", "/tmp", vec![501, 500]);
    let get = |res: &mut SessionResolver| res.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs)).get("hc:t1").cloned();
    assert_eq!(get(&mut res), Some("sid-a".into()), "#6: first session");
    write("sid-b", Utc::now());
    assert_eq!(get(&mut res), Some("sid-b".into()), "#6: /clear in the same process: the new sessionId is picked up at once");
    assert_eq!(get(&mut res), Some("sid-b".into()), "#6: unchanged pid file keeps the cached session");
}

#[test]
fn hc_input_stamp() {
    let procs = parse_ps(
        "  800     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t1
  801   800 Sat Oct 31 10:01:00 2026     claude HC_TERMINAL_ID=t1
  802   801 Sat Oct 31 10:01:30 2026     node /opt/claude/cli.js HC_TERMINAL_ID=t1",
    );
    let (_t, dir) = tmp();
    let start = procs.iter().find(|p| p.pid == 802).unwrap().start;
    let write = |sid: &str, m: Date| {
        let p = write_pid_file(&dir, 802, sid, start);
        set_mtime(&p, m);
    };
    write("sid-a", Utc::now() - Duration::seconds(100));
    let mut res = resolver(|r| {
        r.claude_dir = dir.clone();
        r.retry = 3600.0;
        r.recheck = 3600.0;
    });
    let need = Need::new("hc:t1", "claude", "/tmp", vec![800]);
    let first = res.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs));
    assert_eq!(first.get("hc:t1"), Some(&"sid-a".to_string()), "session found in a child pid's file");
    write("sid-b", Utc::now());
    let second = res.resolve_with(&[need], &set(&[]), &HashMap::new(), Some(&procs));
    assert_eq!(second.get("hc:t1"), Some(&"sid-b".to_string()), "a rewrite of the child pid's file is picked up before the recheck");
}

// MARK: checksHCRemoteDayFolders (ChecksHCRemoteLogs)

#[test]
fn codex_day_folder_slack() {
    let now = Utc::now();
    let today = local_naive(now).date();
    let f = codex_day_folders("/s", now, now, 1);
    let want: Vec<String> = [today.pred_opt().unwrap(), today, today.succ_opt().unwrap()].into_iter().map(|d| day_folder("/s", d)).collect();
    // `after - 2 s` may fall on yesterday just after midnight; then that day is in too (as in Swift).
    if local_naive(now - Duration::seconds(2)).date() == today {
        assert_eq!(f, want, "day folders a day before and after here");
        assert_eq!(codex_day_folders("/s", now, now, 0), vec![day_folder("/s", today)], "no slack, today only");
    }
}

// MARK: checksRemoteHost (hostSetup), ChecksRemoteLog (resolver off remotely)

#[test]
fn host_setup_and_remote() {
    assert_eq!(host_setup("claude", None, true), Some("herdr integration install claude".into()));
    assert_eq!(host_setup("codex", None, true), Some("herdr integration install codex".into()));
    assert_eq!(host_setup("claude", Some("s1"), true), None);
    assert_eq!(host_setup("claude", None, false), None);
    assert_eq!(host_setup("grok", None, true), Some("herdr integration install grok".into()));
    assert_eq!(host_setup("pi", None, true), None);

    let mut res = resolver(|r| r.remote = true);
    let found = res.resolve(&[Need::new("w1:p1", "codex", "/", vec![])], &set(&[]));
    let pass = res.leads(&[LeadAsk::new("w1:p2", "s-lead")], &set(&[]));
    assert!(found.is_empty() && pass.swaps.is_empty() && pass.leads == set(&["s-lead"]), "resolver off remotely, herdr's session kept");
}

// MARK: TUI panes end to end (HERDR_PANE_ID + an open rollout)

#[test]
fn tui_pane_open_log() {
    let h = home_dir();
    let ro = format!("{h}/.codex/sessions/2026/10/10/rollout-2026-10-10T13-55-04-abc.jsonl");
    let procs = parse_ps(
        "  100     1 Sat Oct 10 13:50:00 2026 -zsh HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH=/s.sock\n  101   100 Sat Oct 10 13:55:02 2026 codex HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH=/s.sock\n",
    );
    let ro2 = ro.clone();
    let mut r = SessionResolver::new("/s.sock");
    r.open_files = Box::new(move |pids| if pids.contains(&101) { vec![ro2.clone()] } else { vec![] });
    let need = Need::new("w1:p1", "codex", "/w", vec![]);
    assert_eq!(r.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs)).get("w1:p1"), Some(&ro));
    let mut r2 = SessionResolver::new("/other.sock");
    r2.open_files = Box::new(|_| vec![]);
    assert!(r2.resolve_with(&[need], &set(&[]), &HashMap::new(), Some(&procs)).is_empty(), "another herdr session's socket: no match");
}

// MARK: the platform process listing

#[cfg(target_os = "linux")]
#[test]
fn proc_fs_lists_this_process() {
    let me = std::process::id() as i32;
    let ps = processes();
    let mine = ps.iter().find(|p| p.pid == me).expect("this process is listed");
    // SAFETY: getppid cannot fail.
    assert_eq!(mine.ppid, unsafe { libc::getppid() });
    assert!(mine.start <= Utc::now() && mine.start > Utc::now() - Duration::days(1));
    assert!(!mine.line.is_empty());
    let (_t, dir) = tmp();
    let path = format!("{dir}/held.jsonl");
    let _f = fs::File::create(&path).unwrap();
    assert!(open_files(&[me]).contains(&path), "an open file is listed");
    assert!(open_files(&[]).is_empty());
    assert_eq!(linux::parse_stat("12 (a) b) S 7 12 12 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 4242 0"), Some((7, 4242, "a) b".into())));
}

/// Claude Code on Linux writes `procStart` as `/proc/<pid>/stat` starttime (clock ticks after boot), not `ps lstart`.
#[cfg(target_os = "linux")]
#[test]
fn claude_pid_file_with_linux_tick_proc_start() {
    let pid = std::process::id() as i32;
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    let (_, ticks) = linux::parse_linux_stat(&stat).unwrap();
    let procs = linux::LinuxProcSource::new().processes();
    let file = |start: u64| serde_json::json!({"pid": pid, "sessionId": "sid-linux", "procStart": start.to_string()}).to_string();
    let ok = file(ticks);
    assert_eq!(claude_session_source_with(&[pid], &procs, |_| Some(ok.clone().into_bytes())), Some(("sid-linux".to_string(), pid)));
    let reused = file(ticks + 1_000_000); // pid reused: another start
    assert_eq!(claude_session_source_with(&[pid], &procs, |_| Some(reused.clone().into_bytes())), None);
}

/// Codex writes a rollout for each subagent it starts (the approvals reviewer "guardian", `spawn_agent` helpers)
/// in the same cwd, with `source: {"subagent": ...}`. They are not a pane's conversation.
#[test]
fn codex_subagent_rollouts_are_not_candidates() {
    let (_t, root) = tmp();
    let start = Utc::now() - Duration::seconds(120);
    let day = day_folder(&root, local_naive(start).date());
    fs::create_dir_all(&day).unwrap();
    let write = |dt: i64, id: &str, source: &str| {
        let t = start + Duration::seconds(dt);
        let p = format!("{day}/{}", rollout_name(t, id));
        let meta = format!(
            r#"{{"timestamp":"x","type":"session_meta","payload":{{"id":"{id}","timestamp":"{}","cwd":"/w/lead","source":{source}}}}}"#,
            iso(t)
        ) + "\n";
        fs::write(&p, meta).unwrap();
        p
    };
    let lead = write(2, "lead", r#""cli""#);
    let guardian = write(40, "guardian", r#"{"subagent":{"other":"guardian"}}"#);
    assert_eq!(codex_meta(&guardian), None, "a subagent rollout has no pane meta");
    assert!(codex_subagent_rollout(&guardian) && !codex_subagent_rollout(&lead), "the open-file filter tells them apart");
    assert_eq!(codex_meta(&lead).map(|m| m.0), Some("/w/lead".into()), "a cli rollout still reads");
    assert_eq!(codex_candidates(&root, "/w/lead", start, Utc::now()), vec![lead.clone()], "the subagent rollout is skipped");

    let procs = vec![proc_(900_002, 1, start, "node /opt/bin/codex HC_TERMINAL_ID=t4")];
    let need = Need::new("hc:t4", "codex", "/w/lead", vec![900_002]);
    let mut r = resolver(|r| r.codex_dir = root.clone());
    let got = r.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs)).get("hc:t4").cloned();
    assert_eq!(got, Some(lead), "the lead's rollout resolves although its guardian wrote one in the same cwd");
}

/// The open-file route: Codex keeps its subagents' rollouts open too (the guardian's, in the same cwd). The pane
/// resolves to the lead's rollout from its open files alone (no rollout folder to fall back on).
#[test]
fn codex_open_subagent_rollout_is_not_the_pane() {
    let (_t, home) = tmp();
    let start = Utc::now() - Duration::seconds(120);
    let day = day_folder(&format!("{home}/.codex/sessions"), local_naive(start).date());
    fs::create_dir_all(&day).unwrap();
    let write = |dt: i64, id: &str, source: &str| {
        let t = start + Duration::seconds(dt);
        let p = format!("{day}/{}", rollout_name(t, id));
        let meta = format!(
            r#"{{"timestamp":"x","type":"session_meta","payload":{{"id":"{id}","timestamp":"{}","cwd":"/w/lead","source":{source}}}}}"#,
            iso(t)
        ) + "\n";
        fs::write(&p, meta).unwrap();
        p
    };
    let lead = write(2, "lead", r#""cli""#);
    let guardian = write(40, "guardian", r#"{"subagent":{"other":"guardian"}}"#);

    let procs = vec![proc_(900_003, 1, start, "node /opt/bin/codex HC_TERMINAL_ID=t5")];
    let need = Need::new("hc:t5", "codex", "/w/lead", vec![900_003]);
    let open = vec![lead.clone(), guardian.clone()];
    let mut r = resolver(|r| {
        r.home = home.clone();
        r.codex_dir = format!("{home}/no-rollout-folder");
        r.open_files = Box::new(move |pids| if pids.contains(&900_003) { open.clone() } else { vec![] });
    });
    let got = r.resolve_with(std::slice::from_ref(&need), &set(&[]), &HashMap::new(), Some(&procs)).get("hc:t5").cloned();
    assert_eq!(got, Some(lead), "the lead's open rollout, not its guardian's");
}
