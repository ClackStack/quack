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
            let plucked = quack_config::is_plucked_defaults(config.dirs.defaults.as_deref());
            println!("{}", how_to_update(exe.ok().as_deref(), url, plucked));
        }
    }
    exit::OK
}

/// What to do for the way this binary was installed, going by where it lives.
fn how_to_update(exe: Option<&Path>, url: &str, plucked: bool) -> String {
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
    if url.strip_suffix("/latest").is_some() {
        return script_command(url, plucked);
    }
    if exe.starts_with("/usr/bin") {
        return format!("Update it with your package manager, or from {releases}");
    }
    format!("Get it from {url}")
}

fn script_command(url: &str, plucked: bool) -> String {
    if cfg!(windows) {
        if plucked {
            format!("  & ([scriptblock]::Create((irm {url}/download/install.ps1))) -Plucked")
        } else {
            format!("  irm {url}/download/install.ps1 | iex")
        }
    } else if plucked {
        format!("  curl -fsSL {url}/download/install.sh | sh -s -- --plucked")
    } else {
        format!("  curl -fsSL {url}/download/install.sh | sh")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://example.org/o/r/releases/latest";

    fn how(exe: &str) -> String {
        how_to_update(Some(Path::new(exe)), URL, false)
    }

    #[test]
    fn goes_by_where_the_binary_lives() {
        let brew = "/opt/homebrew/Cellar/quack/0.1.0/bin/quack";
        assert_eq!(how(brew), "  brew upgrade quack");
        assert!(how("/home/a/src/quack/target/release/quack").contains("source"));
        assert!(how("/home/a/.cargo/bin/quack").contains("source"));
        assert!(how_to_update(None, URL, false).contains("o/r/releases"));
        assert!(
            how_to_update(
                Some(Path::new("/usr/bin/quack")),
                "https://corp.example/v.txt",
                false
            )
            .contains("package manager")
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_install_script_install_runs_the_script_again() {
        let got = how("/home/a/.local/bin/quack");
        assert_eq!(got, format!("  curl -fsSL {URL}/download/install.sh | sh"));
        assert_eq!(
            how("/usr/bin/quack"),
            format!("  curl -fsSL {URL}/download/install.sh | sh")
        );
        assert_eq!(
            how_to_update(Some(Path::new("/usr/bin/quack")), URL, true),
            format!("  curl -fsSL {URL}/download/install.sh | sh -s -- --plucked")
        );
        let mirror = how_to_update(
            Some(Path::new("/opt/quack")),
            "https://corp.example/v.txt",
            false,
        );
        assert_eq!(mirror, "Get it from https://corp.example/v.txt");
    }

    #[test]
    fn detects_plucked_defaults_bundle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("defaults");
        std::fs::create_dir_all(dir.join("providers.d")).expect("mkdir");
        std::fs::create_dir_all(dir.join("profiles.d")).expect("mkdir");
        std::fs::write(
            dir.join("providers.d").join("ollama.toml"),
            "base_url = \"http://127.0.0.1:11434\"\n",
        )
        .expect("write");
        assert!(quack_config::is_plucked_defaults(Some(&dir)));

        // Adding a hosted provider makes it un-plucked.
        std::fs::write(
            dir.join("providers.d").join("openai.toml"),
            "base_url = \"https://api.openai.com/v1\"\n",
        )
        .expect("write");
        assert!(!quack_config::is_plucked_defaults(Some(&dir)));

        // Removing the hosted provider but adding config.toml is also un-plucked.
        std::fs::remove_file(dir.join("providers.d").join("openai.toml")).expect("rm");
        std::fs::write(dir.join("config.toml"), "[updates]\n").expect("write");
        assert!(!quack_config::is_plucked_defaults(Some(&dir)));

        // Missing or empty defaults dir is not treated as plucked.
        assert!(!quack_config::is_plucked_defaults(None));
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).expect("mkdir");
        assert!(!quack_config::is_plucked_defaults(Some(&empty)));
    }
}
