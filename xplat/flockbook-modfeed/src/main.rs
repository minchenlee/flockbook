// flockbook-modfeed: the herdr-app mod's sidecar. One command per run; the answer is JSON Lines on stdout
// (contract: docs/protocol.md in the mod folder). Picks the herdr-core session as the app does (HC_SESSION, HC_SOCKET).
fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // install is for a person: plain text and an exit code, not JSON Lines.
    if argv.first().map(String::as_str) == Some("install") {
        std::process::exit(modfeedkit::install::run(&argv[1..]));
    }
    let text = modfeedkit::cli::run(&argv);
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
    std::process::exit(0);
}
