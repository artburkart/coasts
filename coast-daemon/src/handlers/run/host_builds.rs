use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::info;

use coast_core::coastfile::Coastfile;
use coast_core::protocol::BuildProgressEvent;
use coast_docker::compose_build::ComposeBuildDirective;

use super::emit;

/// Build per-instance Docker images on the HOST daemon for services with `build:` directives.
pub(super) async fn build_per_instance_images_on_host(
    coastfile_path: &Path,
    project: &str,
    instance_name: &str,
    build_env: &HashMap<String, String>,
    progress: &tokio::sync::mpsc::Sender<BuildProgressEvent>,
) -> Vec<(String, String)> {
    let mut image_tags = Vec::new();
    let Some((compose_dir, compose_content, directives)) =
        load_host_build_context(coastfile_path, project)
    else {
        return image_tags;
    };

    for directive in directives {
        if let Some(image_tag) = build_image_on_host(
            &directive,
            &compose_content,
            &compose_dir,
            project,
            instance_name,
            build_env,
            progress,
        )
        .await
        {
            image_tags.push(image_tag);
        }
    }

    image_tags
}

fn load_host_build_context(
    coastfile_path: &Path,
    project: &str,
) -> Option<(PathBuf, String, Vec<ComposeBuildDirective>)> {
    let coastfile = Coastfile::from_file(coastfile_path).ok()?;
    let compose_dir = coastfile.compose_dir()?.to_path_buf();
    let compose_content = load_compose_content(&coastfile).ok()?;
    let directives = coast_docker::compose_build::parse_compose_file(&compose_content, project)
        .ok()?
        .build_directives;
    Some((compose_dir, compose_content, directives))
}

fn load_compose_content(coastfile: &Coastfile) -> coast_core::error::Result<String> {
    match coastfile.compose_files() {
        [] => Ok(String::new()),
        [single] => std::fs::read_to_string(single).map_err(|e| {
            coast_core::error::CoastError::coastfile(format!(
                "failed to read compose file '{}': {e}",
                single.display()
            ))
        }),
        many => {
            let first_path = many.first().ok_or_else(|| {
                coast_core::error::CoastError::coastfile("no compose files configured")
            })?;
            let project_dir = first_path.parent().ok_or_else(|| {
                coast_core::error::CoastError::coastfile(format!(
                    "compose path '{}' has no parent directory",
                    first_path.display()
                ))
            })?;

            let mut cmd = Command::new("docker");
            cmd.arg("compose");
            for path in many {
                cmd.arg("-f").arg(path);
            }
            cmd.arg("--project-directory")
                .arg(project_dir)
                .arg("config");
            let output = cmd.output().map_err(|error| {
                coast_core::error::CoastError::coastfile(format!(
                    "failed to run 'docker compose config' for layered compose files: {error}"
                ))
            })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(coast_core::error::CoastError::coastfile(format!(
                    "docker compose config failed for layered compose files: {}",
                    stderr.trim()
                )));
            }
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        }
    }
}

async fn build_image_on_host(
    directive: &ComposeBuildDirective,
    compose_content: &str,
    compose_dir: &Path,
    project: &str,
    instance_name: &str,
    build_env: &HashMap<String, String>,
    progress: &tokio::sync::mpsc::Sender<BuildProgressEvent>,
) -> Option<(String, String)> {
    let instance_tag = coast_docker::compose_build::coast_built_instance_image_tag(
        project,
        &directive.service_name,
        instance_name,
    );
    let mut build_directive = directive.clone();
    build_directive.coast_image_tag = instance_tag.clone();

    info!(
        service = %directive.service_name,
        tag = %instance_tag,
        "building per-instance image on HOST"
    );

    let build_result = coast_docker::compose_build::build_image_on_host(
        &build_directive,
        compose_content,
        compose_dir,
        build_env,
    )
    .await;

    handle_host_build_result(
        &directive.service_name,
        instance_tag,
        build_result,
        progress,
    )
}

fn handle_host_build_result(
    service_name: &str,
    instance_tag: String,
    build_result: coast_core::error::Result<()>,
    progress: &tokio::sync::mpsc::Sender<BuildProgressEvent>,
) -> Option<(String, String)> {
    match build_result {
        Ok(()) => {
            emit(
                progress,
                BuildProgressEvent::item("Building images", service_name, "ok"),
            );
            info!(service = %service_name, "per-instance image built on HOST");
            Some((service_name.to_string(), instance_tag))
        }
        Err(error) => {
            emit_host_build_warning(progress, service_name, error.to_string());
            tracing::warn!(
                service = %service_name,
                error = %error,
                "failed to build per-instance image on HOST, inner compose will build"
            );
            None
        }
    }
}

fn emit_host_build_warning(
    progress: &tokio::sync::mpsc::Sender<BuildProgressEvent>,
    service_name: &str,
    verbose_detail: String,
) {
    emit(
        progress,
        BuildProgressEvent::item("Building images", service_name, "warn")
            .with_verbose(verbose_detail),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_host_build_context_parses_build_services() {
        let dir = tempfile::tempdir().unwrap();
        let coastfile_path = dir.path().join("Coastfile");
        std::fs::write(
            &coastfile_path,
            r#"
[coast]
name = "proj"
compose = "./docker-compose.yml"
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("docker-compose.yml"),
            r#"
services:
  web:
    build: .
  worker:
    image: busybox:latest
"#,
        )
        .unwrap();

        let (_, _, directives) = load_host_build_context(&coastfile_path, "proj").unwrap();

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].service_name, "web");
        assert_eq!(directives[0].context, ".");
        assert!(directives[0].dockerfile.is_none());
    }
}
