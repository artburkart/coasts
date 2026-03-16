use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use tracing::info;

use coast_core::error::{CoastError, Result};
use coast_core::protocol::BuildProgressEvent;
use coast_docker::compose_build::ComposeBuildDirective;
use coast_docker::runtime::Runtime;

use super::emit;

pub(super) struct ArchiveBuildRequest<'a> {
    pub container_id: &'a str,
    pub code_path: &'a Path,
    pub branch: &'a str,
    pub project: &'a str,
    pub instance_name: &'a str,
    pub artifact_dir: &'a Path,
    pub coastfile_path: &'a Path,
    pub has_volume_mounts: bool,
    pub secret_container_paths: &'a [String],
    pub build_env: &'a HashMap<String, String>,
    pub progress: &'a tokio::sync::mpsc::Sender<BuildProgressEvent>,
}

/// Pipe a branch's code into the DinD container via git archive, build per-instance images
/// inside DinD, and write a compose override with image/volume/extra_hosts overrides.
///
/// Returns the list of (service_name, image_tag) pairs built inside DinD.
pub(super) async fn run_archive_build(
    docker: &bollard::Docker,
    request: ArchiveBuildRequest<'_>,
) -> Result<Vec<(String, String)>> {
    let archive_rt = coast_docker::dind::DindRuntime::with_client(docker.clone());

    // Create temp build directory inside DinD
    let _ = archive_rt
        .exec_in_coast(request.container_id, &["mkdir", "-p", "/tmp/coast-build"])
        .await;

    pipe_git_archive_into_dind(request.code_path, request.branch, request.container_id).await?;

    // Build per-instance images INSIDE DinD from /tmp/coast-build
    let per_instance_image_tags = build_images_inside_dind(&archive_rt, &request).await;

    // Write compose override inside DinD at /tmp/coast-build/
    write_archive_compose_override(&archive_rt, &request, &per_instance_image_tags).await;

    emit(
        request.progress,
        BuildProgressEvent::done("Building images", "ok"),
    );

    Ok(per_instance_image_tags)
}

async fn pipe_git_archive_into_dind(
    code_path: &Path,
    branch: &str,
    container_id: &str,
) -> Result<()> {
    let root_owned = code_path.to_path_buf();
    let branch_owned = branch.to_string();
    let container_id_owned = container_id.to_string();
    let archive_result = tokio::task::spawn_blocking(move || {
        let mut archive = std::process::Command::new("git")
            .args(["archive", &branch_owned])
            .current_dir(&root_owned)
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let archive_stdout = archive.stdout.take().expect("archive stdout was piped");
        let extract_output = std::process::Command::new("docker")
            .args([
                "exec",
                "-i",
                &container_id_owned,
                "tar",
                "-x",
                "-C",
                "/tmp/coast-build",
            ])
            .stdin(archive_stdout)
            .output()?;
        archive.wait()?;
        Ok::<_, std::io::Error>(extract_output)
    })
    .await;

    match archive_result {
        Ok(Ok(output)) if output.status.success() => {
            info!(branch = %branch, "piped git archive into DinD at /tmp/coast-build");
            Ok(())
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(CoastError::git(format!(
                "Failed to extract git archive into DinD: {}",
                stderr.trim()
            )))
        }
        Ok(Err(e)) => Err(CoastError::git(format!(
            "Failed to run git archive for branch '{branch}': {e}"
        ))),
        Err(e) => Err(CoastError::git(format!(
            "spawn_blocking failed for git archive: {e}"
        ))),
    }
}

async fn build_images_inside_dind(
    runtime: &coast_docker::dind::DindRuntime,
    request: &ArchiveBuildRequest<'_>,
) -> Vec<(String, String)> {
    let mut image_tags = Vec::new();
    let Some((compose_content, directives)) =
        load_archive_build_directives(request.code_path, request.project)
    else {
        return image_tags;
    };

    for directive in directives {
        if let Some(tag) =
            build_image_inside_dind(runtime, request, &compose_content, &directive).await
        {
            image_tags.push(tag);
        }
    }

    image_tags
}

fn load_archive_build_directives(
    code_path: &Path,
    project: &str,
) -> Option<(String, Vec<ComposeBuildDirective>)> {
    let Some(compose_path) = find_workspace_compose_path(code_path) else {
        return None;
    };
    let Ok(compose_content) = std::fs::read_to_string(compose_path) else {
        return None;
    };

    let directives = coast_docker::compose_build::parse_compose_file(&compose_content, project)
        .ok()?
        .build_directives;
    Some((compose_content, directives))
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

async fn build_image_inside_dind(
    runtime: &coast_docker::dind::DindRuntime,
    request: &ArchiveBuildRequest<'_>,
    compose_content: &str,
    directive: &ComposeBuildDirective,
) -> Option<(String, String)> {
    let instance_tag = coast_docker::compose_build::coast_built_instance_image_tag(
        request.project,
        &directive.service_name,
        request.instance_name,
    );
    let mut image_tags = HashMap::new();
    image_tags.insert(directive.service_name.clone(), instance_tag.clone());
    let rewritten = match coast_docker::compose_build::rewrite_compose_for_build(
        compose_content,
        &image_tags,
    ) {
        Ok(content) => content,
        Err(error) => {
            emit_archive_build_warning(
                request.progress,
                &directive.service_name,
                error.to_string(),
            );
            return None;
        }
    };

    info!(
        service = %directive.service_name,
        tag = %instance_tag,
        "building per-instance image inside DinD"
    );

    let build_result = execute_image_build_command(
        runtime,
        request.container_id,
        &directive.service_name,
        &rewritten,
        request.build_env,
    )
    .await;

    handle_image_build_result(
        &directive.service_name,
        instance_tag,
        build_result,
        request.progress,
    )
}

async fn execute_image_build_command(
    runtime: &coast_docker::dind::DindRuntime,
    container_id: &str,
    service_name: &str,
    compose_content: &str,
    build_env: &HashMap<String, String>,
) -> Result<coast_docker::runtime::ExecResult> {
    let _ = runtime
        .exec_in_coast(container_id, &["docker", "builder", "prune", "-af"])
        .await;
    let write_cmd = format!(
        "printf '%s' '{}' > /tmp/coast-build/docker-compose.coast-build.yml",
        compose_content.replace('\'', "'\\''")
    );
    let _ = runtime
        .exec_in_coast(container_id, &["sh", "-lc", &write_cmd])
        .await;
    let build_cmd = [
        "docker",
        "compose",
        "-f",
        "/tmp/coast-build/docker-compose.coast-build.yml",
        "--project-directory",
        "/tmp/coast-build",
        "build",
        service_name,
    ];
    let build_script = crate::handlers::shell_command_with_env(&build_cmd, build_env);
    runtime
        .exec_in_coast(container_id, &["sh", "-lc", &build_script])
        .await
}

fn handle_image_build_result(
    service_name: &str,
    instance_tag: String,
    build_result: Result<coast_docker::runtime::ExecResult>,
    progress: &tokio::sync::mpsc::Sender<BuildProgressEvent>,
) -> Option<(String, String)> {
    match build_result {
        Ok(result) if result.success() => {
            emit(
                progress,
                BuildProgressEvent::item("Building images", service_name, "ok"),
            );
            info!(service = %service_name, "per-instance image built inside DinD");
            Some((service_name.to_string(), instance_tag))
        }
        Ok(result) => {
            emit_archive_build_warning(progress, service_name, result.stderr.clone());
            tracing::warn!(
                service = %service_name,
                stderr = %result.stderr,
                "failed to build per-instance image inside DinD"
            );
            None
        }
        Err(error) => {
            emit_archive_build_warning(progress, service_name, error.to_string());
            tracing::warn!(
                service = %service_name,
                error = %error,
                "failed to exec docker build inside DinD"
            );
            None
        }
    }
}

fn emit_archive_build_warning(
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

async fn write_archive_compose_override(
    runtime: &coast_docker::dind::DindRuntime,
    request: &ArchiveBuildRequest<'_>,
    per_instance_image_tags: &[(String, String)],
) {
    let compose_content = load_archive_compose_content(request.artifact_dir, request.code_path);
    let override_data = ArchiveComposeOverrideData {
        volume_yaml: build_volume_override_yaml(request.coastfile_path, request.has_volume_mounts),
        service_images: build_service_image_overrides(per_instance_image_tags),
        service_volumes: build_secret_volume_overrides(
            compose_content.as_deref(),
            request.secret_container_paths,
        ),
        service_extra_hosts: build_extra_host_overrides(compose_content.as_deref()),
    };

    if let Some(override_yaml) = render_archive_compose_override(&override_data) {
        write_archive_override_file(runtime, request.container_id, &override_yaml).await;
    }
}

struct ArchiveComposeOverrideData {
    volume_yaml: Option<String>,
    service_images: HashMap<String, String>,
    service_volumes: HashMap<String, Vec<String>>,
    service_extra_hosts: HashMap<String, Vec<String>>,
}

fn load_archive_compose_content(artifact_dir: &Path, code_path: &Path) -> Option<String> {
    let compose_path = artifact_dir.join("compose.yml");
    if compose_path.exists() {
        std::fs::read_to_string(compose_path).ok()
    } else {
        let workspace_compose = code_path.join("docker-compose.yml");
        std::fs::read_to_string(workspace_compose).ok()
    }
}

fn build_volume_override_yaml(coastfile_path: &Path, has_volume_mounts: bool) -> Option<String> {
    if !has_volume_mounts {
        return None;
    }

    let mut volume_yaml = String::from("volumes:\n");
    if coastfile_path.exists() {
        if let Ok(coastfile) = coast_core::coastfile::Coastfile::from_file(coastfile_path) {
            for volume in &coastfile.volumes {
                let container_mount = format!("/coast-volumes/{}", volume.name);
                volume_yaml.push_str(&format!(
                    "  {}:\n    driver: local\n    driver_opts:\n      type: none\n      device: {}\n      o: bind\n",
                    volume.name, container_mount
                ));
            }
        }
    }

    Some(volume_yaml)
}

fn build_service_image_overrides(
    per_instance_image_tags: &[(String, String)],
) -> HashMap<String, String> {
    per_instance_image_tags
        .iter()
        .map(|(service_name, tag)| (service_name.clone(), tag.clone()))
        .collect()
}

fn build_secret_volume_overrides(
    compose_content: Option<&str>,
    secret_container_paths: &[String],
) -> HashMap<String, Vec<String>> {
    if secret_container_paths.is_empty() {
        return HashMap::new();
    }

    let Some(content) = compose_content else {
        return HashMap::new();
    };
    let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return HashMap::new();
    };
    let Some(services) = yaml
        .get("services")
        .and_then(|services| services.as_mapping())
    else {
        return HashMap::new();
    };

    let secret_mounts: Vec<String> = secret_container_paths
        .iter()
        .map(|container_path| format!("{container_path}:{container_path}:ro"))
        .collect();

    services
        .keys()
        .filter_map(|service_name| service_name.as_str().map(str::to_string))
        .map(|service_name| (service_name, secret_mounts.clone()))
        .collect()
}

fn build_extra_host_overrides(compose_content: Option<&str>) -> HashMap<String, Vec<String>> {
    let Some(content) = compose_content else {
        return HashMap::new();
    };

    coast_docker::compose::extract_compose_services(content)
        .into_iter()
        .map(|service_name| {
            (
                service_name,
                vec!["host.docker.internal:host-gateway".to_string()],
            )
        })
        .collect()
}

fn render_archive_compose_override(data: &ArchiveComposeOverrideData) -> Option<String> {
    let mut needs_override = data.volume_yaml.is_some();
    let mut override_yaml = String::from("# Auto-generated by Coast - do not edit\n");

    if let Some(volume_yaml) = &data.volume_yaml {
        override_yaml.push_str(volume_yaml);
    }

    let all_services = collect_archive_override_services(data);
    if !all_services.is_empty() {
        needs_override = true;
        override_yaml.push_str("services:\n");
        for service in all_services {
            append_service_override_yaml(&mut override_yaml, service, data);
        }
    }

    needs_override.then_some(override_yaml)
}

fn collect_archive_override_services(data: &ArchiveComposeOverrideData) -> BTreeSet<&str> {
    data.service_images
        .keys()
        .map(std::string::String::as_str)
        .chain(data.service_volumes.keys().map(std::string::String::as_str))
        .chain(
            data.service_extra_hosts
                .keys()
                .map(std::string::String::as_str),
        )
        .collect()
}

fn append_service_override_yaml(
    override_yaml: &mut String,
    service: &str,
    data: &ArchiveComposeOverrideData,
) {
    override_yaml.push_str(&format!("  {}:\n", service));
    if let Some(tag) = data.service_images.get(service) {
        override_yaml.push_str(&format!("    image: {}\n", tag));
    }
    if let Some(volumes) = data.service_volumes.get(service) {
        override_yaml.push_str("    volumes:\n");
        for volume in volumes {
            override_yaml.push_str(&format!("      - {}\n", volume));
        }
    }
    if let Some(extra_hosts) = data.service_extra_hosts.get(service) {
        override_yaml.push_str("    extra_hosts:\n");
        for extra_host in extra_hosts {
            override_yaml.push_str(&format!("      - \"{}\"\n", extra_host));
        }
    }
    override_yaml.push_str("    environment:\n");
    override_yaml.push_str("      WATCHPACK_POLLING: \"true\"\n");
}

async fn write_archive_override_file(
    runtime: &coast_docker::dind::DindRuntime,
    container_id: &str,
    override_yaml: &str,
) {
    let write_cmd = format!(
        "cat > /tmp/coast-build/docker-compose.override.yml << 'COAST_EOF'\n{}\nCOAST_EOF",
        override_yaml
    );
    let _ = runtime
        .exec_in_coast(container_id, &["sh", "-c", &write_cmd])
        .await;
    info!("wrote compose override inside DinD at /tmp/coast-build/");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_archive_build_directives_reads_workspace_compose() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("docker-compose.yml"),
            r#"
services:
  web:
    build:
      context: .
      dockerfile: Dockerfile.dev
"#,
        )
        .unwrap();

        let (compose_content, directives) =
            load_archive_build_directives(dir.path(), "proj").unwrap();

        assert!(compose_content.contains("Dockerfile.dev"));
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].service_name, "web");
    }

    #[test]
    fn test_build_secret_volume_overrides_applies_to_all_services() {
        let compose = r#"
services:
  web:
    image: app
  worker:
    image: jobs
"#;
        let overrides =
            build_secret_volume_overrides(Some(compose), &["/run/secrets/api_key".to_string()]);

        assert_eq!(
            overrides.get("web"),
            Some(&vec![
                "/run/secrets/api_key:/run/secrets/api_key:ro".to_string()
            ])
        );
        assert_eq!(
            overrides.get("worker"),
            Some(&vec![
                "/run/secrets/api_key:/run/secrets/api_key:ro".to_string()
            ])
        );
    }

    #[test]
    fn test_build_volume_override_yaml_includes_declared_volumes() {
        let dir = tempfile::tempdir().unwrap();
        let coastfile_path = dir.path().join("Coastfile");
        std::fs::write(
            &coastfile_path,
            r#"
[coast]
name = "proj"
compose = "./docker-compose.yml"

[volumes.cache]
strategy = "shared"
service = "redis"
mount = "/data"
"#,
        )
        .unwrap();

        let volume_yaml = build_volume_override_yaml(&coastfile_path, true).unwrap();

        assert!(volume_yaml.contains("volumes:\n"));
        assert!(volume_yaml.contains("cache:"));
        assert!(volume_yaml.contains("device: /coast-volumes/cache"));
    }

    #[test]
    fn test_render_archive_compose_override_renders_services_and_watchpack() {
        let data = ArchiveComposeOverrideData {
            volume_yaml: None,
            service_images: HashMap::from([(
                "web".to_string(),
                "coast-built/proj/web:dev-1".to_string(),
            )]),
            service_volumes: HashMap::from([(
                "web".to_string(),
                vec!["/run/secrets/api_key:/run/secrets/api_key:ro".to_string()],
            )]),
            service_extra_hosts: HashMap::from([(
                "web".to_string(),
                vec!["host.docker.internal:host-gateway".to_string()],
            )]),
        };

        let override_yaml = render_archive_compose_override(&data).unwrap();

        assert!(override_yaml.contains("services:\n"));
        assert!(override_yaml.contains("  web:\n"));
        assert!(override_yaml.contains("    image: coast-built/proj/web:dev-1\n"));
        assert!(override_yaml.contains("      - /run/secrets/api_key:/run/secrets/api_key:ro\n"));
        assert!(override_yaml.contains("      - \"host.docker.internal:host-gateway\"\n"));
        assert!(override_yaml.contains("      WATCHPACK_POLLING: \"true\"\n"));
    }

    #[test]
    fn test_render_archive_compose_override_keeps_volume_only_override() {
        let data = ArchiveComposeOverrideData {
            volume_yaml: Some("volumes:\n  cache:\n".to_string()),
            service_images: HashMap::new(),
            service_volumes: HashMap::new(),
            service_extra_hosts: HashMap::new(),
        };

        let override_yaml = render_archive_compose_override(&data).unwrap();

        assert!(override_yaml.contains("volumes:\n  cache:\n"));
        assert!(!override_yaml.contains("services:\n"));
    }
}
