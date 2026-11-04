//! Swift: `ChecksModFeedReview.swift` (the claim race part). Two sidecars that press Allow at the same moment: only
//! one claim wins, and a claim file that another run is still writing is never removed.

use modfeedkit::answered::FeedAnswered;
use modfeedkit::protocol::fnv1a;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::{Duration, SystemTime};

const T: &str = "t1";
const ID: &str = "box-1";

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("locate/cache.json");
    let folder = dir.path().join("locate/answered").join(fnv1a(T));
    (dir, cache, folder)
}

fn age(path: &Path, by: Duration) {
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() - by).unwrap();
}

fn names(folder: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn many_runs_at_once_one_claim_wins() {
    let (_d, cache, folder) = setup();
    let n = 16;
    let gate = Arc::new(Barrier::new(n));
    let wins: usize = (0..n)
        .map(|_| {
            let (cache, gate) = (cache.clone(), gate.clone());
            std::thread::spawn(move || {
                let a = FeedAnswered::new(Some(&cache));
                gate.wait();
                a.claim(T, ID, "Bash", "rm -rf build", true, SystemTime::now()) as usize
            })
        })
        .collect::<Vec<_>>() // every thread starts before any join: they meet at the barrier
        .into_iter()
        .map(|h| h.join().unwrap())
        .sum();
    assert_eq!(wins, 1, "one claim wins of {n} at the same moment");
    assert_eq!(names(&folder), vec![fnv1a(ID)], "only the claim stays: no lock, no temp file");
}

#[test]
fn a_claim_mid_write_is_not_removed() {
    let (_d, cache, folder) = setup();
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(fnv1a(ID));
    std::fs::write(&path, b"").unwrap(); // an older sidecar made it and has not written yet
    let a = FeedAnswered::new(Some(&cache));
    assert!(!a.claim(T, ID, "Bash", "ls", true, SystemTime::now()), "a fresh empty claim counts as taken");
    assert!(path.exists(), "and it stays");
    a.seen(T, None);
    assert!(path.exists(), "seen() leaves a fresh empty claim too");

    age(&path, Duration::from_secs(3));
    assert!(a.claim(T, ID, "Bash", "ls", true, SystemTime::now()), "an old unreadable file is no claim");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("\"id\":\"box-1\""), "the new claim has its mark ({body})");
}

#[test]
fn a_lock_of_a_live_run_blocks_an_old_one_goes() {
    let (_d, cache, folder) = setup();
    std::fs::create_dir_all(&folder).unwrap();
    let lock = folder.join(".lock");
    std::fs::write(&lock, b"").unwrap();
    let a = FeedAnswered::new(Some(&cache));
    assert!(!a.claim(T, ID, "Bash", "ls", true, SystemTime::now()), "another run is claiming: no second claim");
    assert!(!folder.join(fnv1a(ID)).exists());

    age(&lock, Duration::from_secs(30));
    assert!(a.claim(T, ID, "Bash", "ls", true, SystemTime::now()), "a lock left by a dead run goes");
    assert_eq!(names(&folder), vec![fnv1a(ID)]);
}

#[test]
fn hold_and_release_leave_no_temp_files() {
    let (_d, cache, folder) = setup();
    let a = FeedAnswered::new(Some(&cache));
    assert!(a.claim(T, ID, "Bash", "ls", true, SystemTime::now()));
    a.hold(T, ID);
    assert_eq!(names(&folder), vec![fnv1a(ID)]);
    assert!(a.is_claimed(T, "other-id", "Bash", "l s", true, SystemTime::now()), "same tool and command: claimed");
    a.release(T, ID);
    assert!(names(&folder).is_empty());
}
