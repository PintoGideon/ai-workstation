// FORK: stamp the git commit into `aiw -V`, so two builds of the same
// version number can be told apart. No dependency: `git` is run if present,
// and a source tarball without `.git` simply gets no stamp.
use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .is_some_and(|out| !out.stdout.is_empty());
    if let Some(sha) = sha {
        let stamp = if dirty { format!("{sha}-dirty") } else { sha };
        println!("cargo:rustc-env=AIW_GIT_SHA={stamp}");
    }
    // Rebuild the stamp when HEAD moves (commit, checkout, rebase).
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Ok(head) = std::fs::read_to_string(".git/HEAD")
        && let Some(reference) = head.strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed=.git/{}", reference.trim());
    }
}
