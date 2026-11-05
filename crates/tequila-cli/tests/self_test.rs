//! `--self-test` as people run it: the binary, its report and its folder.
//!
//! The facts alone (`--facts-only`), which every machine has and which take
//! a second: the report is written and printed, says where to share it,
//! and names no one - the person's user name, the computer's name, the home
//! folder and the folder the report went to are nowhere in it. The renders
//! are run by CI as `--self-test --cpu-only` (`.github/workflows/ci.yml`).

use std::path::PathBuf;
use std::process::Command;

/// A folder of its own for each test: they run at once.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tequila-self-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

#[test]
fn the_report_is_written_and_names_no_one() {
    let dir = scratch("report");
    let folder = dir.join("run");
    let result = Command::new(env!("CARGO_BIN_EXE_tequila")).args(["--self-test", "--facts-only", "-o"]).arg(&folder).output().expect("the test runs");
    let said = String::from_utf8_lossy(&result.stdout).into_owned();
    assert!(result.status.success(), "{said}\n{}", String::from_utf8_lossy(&result.stderr));
    let report = std::fs::read_to_string(folder.join("report.md")).expect("report.md is written");
    assert!(report.starts_with("**Tequila self-test**"), "{report}");
    assert!(report.contains("https://discord.gg/"), "where to share it: {report}");
    assert!(said.contains(report.trim()), "the report is printed as well");
    for line in ["OS ", "CPU ", "Cycles "] {
        assert!(report.contains(line), "no {line:?} line: {report}");
    }

    let mut secrets: Vec<String> = ["USERNAME", "USER", "LOGNAME", "COMPUTERNAME", "HOSTNAME", "USERPROFILE", "HOME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .collect();
    secrets.push(folder.display().to_string());
    secrets.push(dir.display().to_string());
    for secret in secrets.iter().map(|s| s.trim()).filter(|s| s.chars().count() >= 3) {
        assert!(!report.to_lowercase().contains(&secret.to_lowercase()), "the report holds {secret:?}:\n{report}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A run that stopped is carried on from its folder, the check it stopped
/// in reported as a crash with what the application saw.
#[test]
fn a_stopped_run_is_carried_on_and_its_crash_reported() {
    let dir = scratch("resume").join("run");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("results.tsv"), "started\tfacts\noutcome\tfacts\tpass\t-\t-\t-\t-\tfacts\t\nstarted\ttequila-cpu\n").unwrap();
    // Only what is cheap runs: the scene's renders are left to CI.
    let result = Command::new(env!("CARGO_BIN_EXE_tequila"))
        .args(["--self-test", "--facts-only", "--resume", "--crashed", "exit code 0xc0000005", "-o"])
        .arg(&dir)
        .output()
        .expect("the test runs");
    // Not one of this run's checks, and still reported, with how it ended;
    // and the run fails for it.
    let report = std::fs::read_to_string(dir.join("report.md")).unwrap();
    assert!(report.contains("CRASH") && report.contains("exit code 0xc0000005"), "{report}");
    assert_eq!(result.status.code(), Some(1), "a crashed check fails the run");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}
