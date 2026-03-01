/// `coast ui` command — open the Coast dashboard in the default browser.
///
/// If run from within a known project directory, navigates directly to
/// that project's page. Detects the project by matching the cwd against
/// `project_root` paths from existing build manifests.
use anyhow::{bail, Result};
use clap::Args;
use colored::Colorize;
use rust_i18n::t;

fn default_api_port() -> u16 {
    std::env::var("COAST_API_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(31415)
}
const RESOLVER_PATH: &str = "/etc/resolver/localcoast";

/// Arguments for `coast ui`.
#[derive(Debug, Args)]
pub struct UiArgs {
    /// Override the port (default: 31415).
    #[arg(long)]
    port: Option<u16>,
}

/// Execute the `coast ui` command.
pub async fn execute(args: &UiArgs) -> Result<()> {
    let sock = super::socket_path();
    if tokio::net::UnixStream::connect(&sock).await.is_err() {
        bail!("{}", t!("error.daemon_not_running"));
    }

    let port = args.port.unwrap_or_else(default_api_port);

    // Use localcoast hostname only when the resolver exists AND we're on the
    // default port (production). Dev mode runs on a different port and its DNS
    // server isn't registered in /etc/resolver, so always use localhost there.
    let host = if port == 31415 && std::path::Path::new(RESOLVER_PATH).exists() {
        "localcoast"
    } else {
        "localhost"
    };

    let project = detect_project_from_cwd();

    let url = match project {
        Some(ref name) => format!("http://{host}:{port}/#/project/{name}"),
        None => format!("http://{host}:{port}"),
    };

    match &project {
        Some(name) => println!(
            "{} Opening {} (project: {})",
            "ok".green().bold(),
            url.bold(),
            name.bold(),
        ),
        None => println!("{} Opening {}", "ok".green().bold(), url.bold(),),
    }

    open_browser(&url)?;
    Ok(())
}

/// Scan $COAST_HOME/images/*/latest/manifest.json for project roots and find
/// which project (if any) the current working directory belongs to.
fn detect_project_from_cwd() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let images_dir = coast_core::artifact::coast_home().ok()?.join("images");
    let entries = std::fs::read_dir(&images_dir).ok()?;

    let mut projects: Vec<(String, std::path::PathBuf)> = Vec::new();

    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let manifest_path = entry.path().join("latest").join("manifest.json");
        let content = std::fs::read_to_string(&manifest_path).ok();
        let root = content
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .and_then(|v| v.get("project_root")?.as_str().map(ToString::to_string));

        if let Some(root) = root {
            projects.push((name, std::path::PathBuf::from(root)));
        }
    }

    // Sort by path length descending so deeper (more specific) roots match first
    projects.sort_by(|a, b| b.1.as_os_str().len().cmp(&a.1.as_os_str().len()));

    for (name, root) in &projects {
        if cwd.starts_with(root) {
            return Some(name.clone());
        }
    }
    None
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!("Browser opening not supported on this platform. Visit: {url}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        args: UiArgs,
    }

    #[test]
    fn test_ui_default_args() {
        let cli = TestCli::try_parse_from(["test"]).unwrap();
        assert!(cli.args.port.is_none());
    }

    #[test]
    fn test_ui_custom_port() {
        let cli = TestCli::try_parse_from(["test", "--port", "8080"]).unwrap();
        assert_eq!(cli.args.port, Some(8080));
    }

    #[tokio::test]
    async fn test_ui_fails_when_daemon_not_running() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("COAST_HOME", tmp.path());
        let args = UiArgs { port: None };
        let result = execute(&args).await;
        std::env::remove_var("COAST_HOME");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("coast daemon start"),
            "error should suggest starting the daemon, got: {err}"
        );
    }
}
