use std::path::{Path, PathBuf};

use tracing::info;

use coast_core::coastfile::Coastfile;
use coast_core::protocol::BuildProgressEvent;
use coast_docker::compose_build::ComposeBuildDirective;

use super::emit;
use crate::handlers::build_secrets;

/// Build per-instance Docker images on the HOST daemon for services with `build:` directives.
///
/// Parses the compose file to find build directives, runs `docker build` on the host for each,
/// and returns the list of (service_name, image_tag) pairs that were successfully built.
/// Uses the host's Docker layer cache from `coast build`, making rebuilds fast.
pub(super) async fn build_per_instance_images_on_host(
    code_path: &Path,
    coastfile_path: &Path,
    project: &str,
    instance_name: &str,
    progress: &tokio::sync::mpsc::Sender<BuildProgressEvent>,
) -> Vec<(String, String)> {
    let mut image_tags = Vec::new();
    let coastfile = Coastfile::from_file(coastfile_path).ok();

    for directive in load_host_build_directives(code_path, project) {
        if let Some(image_tag) = build_image_on_host(
            &directive,
            code_path,
            coastfile.as_ref(),
            project,
            instance_name,
            progress,
        )
        .await
        {
            image_tags.push(image_tag);
        }
    }

    image_tags
}

fn load_host_build_directives(code_path: &Path, project: &str) -> Vec<ComposeBuildDirective> {
    let Some(compose_path) = find_workspace_compose_path(code_path) else {
        return Vec::new();
    };
    let Ok(compose_content) = std::fs::read_to_string(compose_path) else {
        return Vec::new();
    };

    coast_docker::compose_build::parse_compose_file(&compose_content, project)
        .map(|result| result.build_directives)
        .unwrap_or_default()
}

fn find_workspace_compose_path(code_path: &Path) -> Option<PathBuf> {
    [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ]
    .iter()
    .map(|name| code_path.join(name))
    .find(|path| path.exists())
}

async fn build_image_on_host(
    directive: &ComposeBuildDirective,
    code_path: &Path,
    coastfile: Option<&Coastfile>,
    project: &str,
    instance_name: &str,
    progress: &tokio::sync::mpsc::Sender<BuildProgressEvent>,
) -> Option<(String, String)> {
    let (instance_tag, command_args, _materialized_secrets) =
        host_build_command_args(directive, code_path, coastfile, project, instance_name).ok()?;

    info!(
        service = %directive.service_name,
        tag = %instance_tag,
        "building per-instance image on HOST"
    );

    let build_result = tokio::process::Command::new(&command_args[0])
        .args(&command_args[1..])
        .output()
        .await;

    handle_host_build_result(
        &directive.service_name,
        instance_tag,
        build_result,
        progress,
    )
}

fn host_build_command_args(
    directive: &ComposeBuildDirective,
    code_path: &Path,
    coastfile: Option<&Coastfile>,
    project: &str,
    instance_name: &str,
) -> coast_core::error::Result<(
    String,
    Vec<String>,
    Option<build_secrets::MaterializedBuildSecrets>,
)> {
    let instance_tag = coast_docker::compose_build::coast_built_instance_image_tag(
        project,
        &directive.service_name,
        instance_name,
    );
    let mut build_directive = directive.clone();
    build_directive.coast_image_tag = instance_tag.clone();
    let materialized_secrets = match coastfile {
        Some(coastfile) => Some(build_secrets::materialize_build_secrets(
            coastfile,
            &build_directive,
        )?),
        None if directive.secrets.is_empty() => None,
        None => {
            return Err(coast_core::error::CoastError::coastfile(
                "build secrets were requested but no Coastfile artifact was available",
            ));
        }
    };
    let secret_refs = materialized_secrets
        .as_ref()
        .map(|materialized| materialized.secrets.as_slice())
        .unwrap_or(&[]);
    let command_args =
        coast_docker::compose_build::docker_build_cmd(&build_directive, code_path, secret_refs);
    Ok((instance_tag, command_args, materialized_secrets))
}

fn handle_host_build_result(
    service_name: &str,
    instance_tag: String,
    build_result: Result<std::process::Output, std::io::Error>,
    progress: &tokio::sync::mpsc::Sender<BuildProgressEvent>,
) -> Option<(String, String)> {
    match build_result {
        Ok(output) if output.status.success() => {
            emit(
                progress,
                BuildProgressEvent::item("Building images", service_name, "ok"),
            );
            info!(service = %service_name, "per-instance image built on HOST");
            Some((service_name.to_string(), instance_tag))
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            emit_host_build_warning(progress, service_name, stderr.trim().to_string());
            tracing::warn!(
                service = %service_name,
                stderr = %stderr,
                "failed to build per-instance image on HOST, inner compose will build"
            );
            None
        }
        Err(error) => {
            emit_host_build_warning(progress, service_name, error.to_string());
            tracing::warn!(
                service = %service_name,
                error = %error,
                "failed to run docker build on HOST, inner compose will build"
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
    fn test_find_workspace_compose_path_finds_first_existing_candidate() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("compose.yaml"), "services: {}\n").unwrap();

        let compose_path = find_workspace_compose_path(dir.path()).unwrap();
        assert_eq!(compose_path, dir.path().join("compose.yaml"));
    }

    #[test]
    fn test_load_host_build_directives_parses_build_services() {
        let dir = tempfile::tempdir().unwrap();
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

        let directives = load_host_build_directives(dir.path(), "proj");

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].service_name, "web");
        assert_eq!(directives[0].context, ".");
        assert!(directives[0].dockerfile.is_none());
    }
}
