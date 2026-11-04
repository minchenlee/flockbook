//! macOS (and other non-Linux unix) process source: `/bin/ps` and `/usr/sbin/lsof`, as Swift runs them: read-only,
//! C locale, minimal environment.

use super::{parse_lsof, parse_ps, Proc, ProcessSource};
use std::process::{Command, Stdio};

pub struct MacProcessSource;

fn run(tool: &str, args: &[&str]) -> String {
    let out = Command::new(tool)
        .args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

impl ProcessSource for MacProcessSource {
    fn processes(&self) -> Vec<Proc> {
        parse_ps(&run("/bin/ps", &["-xE", "-ww", "-o", "pid=,ppid=,lstart=,command="]))
    }

    fn open_files(&self, pids: &[i32]) -> Vec<String> {
        if pids.is_empty() {
            return Vec::new();
        }
        let list = pids.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
        parse_lsof(&run("/usr/sbin/lsof", &["-nP", "-Fn", "-p", &list]))
    }
}
