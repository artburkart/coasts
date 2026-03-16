use std::path::PathBuf;
use std::process::Command;

use coast_core::coastfile::Coastfile;
use coast_core::error::{CoastError, Result};
use coast_core::protocol::BuildProgressEvent;
use coast_docker::compose_build::{self, ComposeParseResult};

pub(super) struct ComposeAnalysis {
    pub content: Option<String>,
    pub dir: Option<PathBuf>,
    pub parse_result: Option<ComposeParseResult>,
}

impl ComposeAnalysis {
    pub(super) fn from_coastfile(coastfile: &Coastfile) -> Result<Self> {
        let dir = coastfile.compose_dir().map(std::path::Path::to_path_buf);
        let content = match coastfile.compose_files() {
            [] => None,
            [single] => std::fs::read_to_string(single).ok(),
            many => Some(render_merged_compose(many)?),
        };
        let parse_result = content.as_ref().and_then(|compose_content| {
            compose_build::parse_compose_file_filtered(
                compose_content,
                &coastfile.name,
                &coastfile.omit.services,
            )
            .ok()
        });

        Ok(Self {
            content,
            dir,
            parse_result,
        })
    }

    pub(super) fn has_build_directives(&self) -> bool {
        self.parse_result
            .as_ref()
            .is_some_and(|result| !result.build_directives.is_empty())
    }

    pub(super) fn has_image_refs(&self) -> bool {
        self.parse_result
            .as_ref()
            .is_some_and(|result| !result.image_refs.is_empty())
    }
}

fn render_merged_compose(paths: &[PathBuf]) -> Result<String> {
    let first_path = paths
        .first()
        .ok_or_else(|| CoastError::coastfile("no compose files configured"))?;
    let project_dir = first_path.parent().ok_or_else(|| {
        CoastError::coastfile(format!(
            "compose path '{}' has no parent directory",
            first_path.display()
        ))
    })?;

    let mut cmd = Command::new("docker");
    cmd.arg("compose");
    for path in paths {
        cmd.arg("-f").arg(path);
    }
    cmd.arg("--project-directory")
        .arg(project_dir)
        .arg("config");

    let output = cmd.output().map_err(|error| {
        CoastError::coastfile(format!(
            "failed to run 'docker compose config' for layered compose files: {error}"
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CoastError::coastfile(format!(
            "docker compose config failed for layered compose files: {}",
            stderr.trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BuildPlan {
    steps: Vec<String>,
}

impl BuildPlan {
    pub(super) fn from_inputs(
        has_secrets: bool,
        has_build_directives: bool,
        has_image_refs: bool,
        has_setup: bool,
    ) -> Self {
        let mut steps = vec!["Parsing Coastfile".to_string()];
        if has_secrets {
            steps.push("Extracting secrets".to_string());
        }
        steps.push("Creating artifact".to_string());
        if has_build_directives {
            steps.push("Building images".to_string());
        }
        if has_image_refs {
            steps.push("Pulling images".to_string());
        }
        if has_setup {
            steps.push("Building coast image".to_string());
        }
        steps.push("Writing manifest".to_string());
        Self { steps }
    }

    #[cfg(test)]
    pub(super) fn steps(&self) -> &[String] {
        &self.steps
    }

    pub(super) fn total_steps(&self) -> u32 {
        self.steps.len() as u32
    }

    pub(super) fn step_number(&self, name: &str) -> u32 {
        self.steps
            .iter()
            .position(|step| step == name)
            .map(|idx| (idx + 1) as u32)
            .expect("step not in plan")
    }

    pub(super) fn build_plan_event(&self) -> BuildProgressEvent {
        BuildProgressEvent::build_plan(self.steps.clone())
    }

    pub(super) fn started(&self, name: &str) -> BuildProgressEvent {
        BuildProgressEvent::started(name, self.step_number(name), self.total_steps())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::*;

    struct PathEnvGuard {
        _guard: MutexGuard<'static, ()>,
        previous_path: Option<OsString>,
    }

    impl Drop for PathEnvGuard {
        fn drop(&mut self) {
            match &self.previous_path {
                Some(path) => unsafe { std::env::set_var("PATH", path) },
                None => unsafe { std::env::remove_var("PATH") },
            }
        }
    }

    fn prepend_test_path(path: &std::path::Path) -> PathEnvGuard {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let previous_path = std::env::var_os("PATH");
        let mut paths = vec![path.to_path_buf()];
        paths.extend(
            previous_path
                .as_ref()
                .map(std::env::split_paths)
                .into_iter()
                .flatten(),
        );
        let combined = std::env::join_paths(paths).unwrap();
        unsafe {
            std::env::set_var("PATH", &combined);
        }
        PathEnvGuard {
            _guard: guard,
            previous_path,
        }
    }

    #[test]
    fn test_build_plan_includes_optional_steps_in_order() {
        let plan = BuildPlan::from_inputs(true, true, true, true);
        assert_eq!(
            plan.steps(),
            &[
                "Parsing Coastfile".to_string(),
                "Extracting secrets".to_string(),
                "Creating artifact".to_string(),
                "Building images".to_string(),
                "Pulling images".to_string(),
                "Building coast image".to_string(),
                "Writing manifest".to_string(),
            ]
        );
        assert_eq!(plan.step_number("Pulling images"), 5);
        assert_eq!(plan.total_steps(), 7);
    }

    #[test]
    fn test_build_plan_minimal_shape() {
        let plan = BuildPlan::from_inputs(false, false, false, false);
        assert_eq!(
            plan.steps(),
            &[
                "Parsing Coastfile".to_string(),
                "Creating artifact".to_string(),
                "Writing manifest".to_string(),
            ]
        );
        let event = plan.started("Creating artifact");
        assert_eq!(event.step, "Creating artifact");
        assert_eq!(event.status, "started");
        assert_eq!(event.step_number, Some(2));
        assert_eq!(event.total_steps, Some(3));
    }

    #[test]
    fn test_compose_analysis_detects_builds_and_images() {
        let dir = tempfile::tempdir().unwrap();
        let compose_path = dir.path().join("docker-compose.yml");
        std::fs::write(
            &compose_path,
            r#"services:
  app:
    build: .
  db:
    image: postgres:16
"#,
        )
        .unwrap();

        let coastfile = Coastfile::parse(
            r#"
[coast]
name = "plan-test"
compose = "./docker-compose.yml"
"#,
            dir.path(),
        )
        .unwrap();

        let analysis = ComposeAnalysis::from_coastfile(&coastfile).unwrap();
        assert!(analysis.content.is_some());
        assert_eq!(analysis.dir, Some(dir.path().to_path_buf()));
        assert!(analysis.has_build_directives());
        assert!(analysis.has_image_refs());
    }

    #[test]
    fn test_compose_analysis_merges_multiple_files_via_docker_compose_config() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = tempfile::tempdir().unwrap();
        let docker_path = bin_dir.path().join("docker");
        std::fs::write(
            &docker_path,
            r#"#!/bin/sh
printf '%s\n' "services:
  app:
    build: .
  db:
    image: postgres:16"
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&docker_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&docker_path, perms).unwrap();
        }
        let _path_guard = prepend_test_path(bin_dir.path());

        let coastfile = Coastfile::parse(
            r#"
[coast]
name = "plan-test"
compose = ["./docker-compose.yml", "./docker-compose.dev.yml"]
"#,
            dir.path(),
        )
        .unwrap();

        let analysis = ComposeAnalysis::from_coastfile(&coastfile).unwrap();
        assert!(analysis.content.is_some());
        assert_eq!(analysis.dir, Some(dir.path().to_path_buf()));
        assert!(analysis.has_build_directives());
        assert!(analysis.has_image_refs());
    }
}
