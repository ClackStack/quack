//! `quack update`: says whether a new version is out and how to get it for this install.
//! It changes nothing: quack has no self-updater (docs/plan/14-packaging-and-distribution.md).

use std::path::{Component, Path};

use quack_config::Loaded;
use quack_core::checks::{self, CURRENT};
use quack_exec::exit;

pub(crate) async fn run(config: &Loaded) -> i32 {
    let latest = match checks::latest_version(config).await {
        Ok(latest) => latest,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::AGENT_ERROR;
        }
    };
    let url = &config.settings.updates.url;
    match latest {
        None => println!("quack {CURRENT}; {url} has no release yet."),
        Some(l) if !checks::is_newer(&l, CURRENT) => println!("quack {CURRENT} is up to date."),
        Some(l) => {
            println!("quack {l} is available (you have {CURRENT}).\n");
            let exe = std::env::current_exe().and_then(std::fs::canonicalize);
            println!("{}", how_to_update(exe.ok().as_deref(), url));
        }
    }
    exit::OK
}

/// What to do for the way this binary was installed, going by where it lives.
fn how_to_update(exe: Option<&Path>, url: &str) -> String {
    let releases = url.strip_suffix("/latest").unwrap_or(url);
    let Some(exe) = exe else {
        return format!("Get it from {releases}");
    };
    let parts: Vec<&str> = exe
        .components()
        .filter_map(|c| match c {
            Component::Normal(p) => p.to_str(),
            _ => None,
        })
        .collect();
    if let Some(i) = parts.iter().position(|p| *p == "Cellar")
        && let Some(formula) = parts.get(i + 1)
    {
        return format!("  brew upgrade {formula}");
    }
    if parts.contains(&"target") || parts.contains(&".cargo") {
        return "This binary was built from source: pull and build again.".to_owned();
    }
    if exe.starts_with("/usr/bin") {
        return format!("Update it with your package manager, or from {releases}");
    }
    match url.strip_suffix("/latest") {
        Some(_) if cfg!(windows) => format!("  irm {url}/download/install.ps1 | iex"),
        Some(_) => format!("  curl -fsSL {url}/download/install.sh | sh"),
        None => format!("Get it from {url}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://example.org/o/r/releases/latest";

    fn how(exe: &str) -> String {
        how_to_update(Some(Path::new(exe)), URL)
    }

    #[test]
    fn goes_by_where_the_binary_lives() {
        let brew = "/opt/homebrew/Cellar/quack-plucked/0.1.0/bin/quack";
        assert_eq!(how(brew), "  brew upgrade quack-plucked");
        assert!(how("/usr/bin/quack").contains("package manager"));
        assert!(how("/home/a/src/quack/target/release/quack").contains("source"));
        assert!(how("/home/a/.cargo/bin/quack").contains("source"));
        assert!(how_to_update(None, URL).contains("o/r/releases"));
    }

    #[cfg(unix)]
    #[test]
    fn an_install_script_install_runs_the_script_again() {
        let got = how("/home/a/.local/bin/quack");
        assert_eq!(got, format!("  curl -fsSL {URL}/download/install.sh | sh"));
        let mirror = how_to_update(Some(Path::new("/opt/quack")), "https://corp.example/v.txt");
        assert_eq!(mirror, "Get it from https://corp.example/v.txt");
    }
}
