use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use tracing::info;

use coast_core::error::{CoastError, Result};
use coast_core::protocol::BuildProgressEvent;
use coast_docker::compose_build::ComposeBuildDirective;
use coast_docker::runtime::Runtime;

use super::{emit, paths};

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

    for directive in load_archive_build_directives(request.code_path, request.project) {
        if let Some(tag) = build_image_inside_dind(runtime, request, &directive).await {
            image_tags.push(tag);
        }
    }

    image_tags
}

fn load_archive_build_directives(code_path: &Path, project: &str) -> Vec<ComposeBuildDirective> {
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

async fn build_image_inside_dind(
    runtime: &coast_docker::dind::DindRuntime,
    request: &ArchiveBuildRequest<'_>,
    directive: &ComposeBuildDirective,
) -> Option<(String, String)> {
    let instance_tag = coast_docker::compose_build::coast_built_instance_image_tag(
        request.project,
        &directive.service_name,
        request.instance_name,
    );
    let build_context = archive_build_context_path(&directive.context);

    info!(
        service = %directive.service_name,
        tag = %instance_tag,
        context = %build_context,
        "building per-instance image inside DinD"
    );

    let build_result = execute_image_build_command(
        runtime,
        request.container_id,
        directive,
        &instance_tag,
        &build_context,
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
    directive: &ComposeBuildDirective,
    instance_tag: &str,
    build_context: &str,
) -> Result<coast_docker::runtime::ExecResult> {
    let _ = runtime
        .exec_in_coast(container_id, &["docker", "builder", "prune", "-af"])
        .await;

    let build_cmd =
        docker_build_command(instance_tag, build_context, directive.dockerfile.as_deref());
    let cmd_refs: Vec<&str> = build_cmd.iter().map(std::string::String::as_str).collect();
    runtime.exec_in_coast(container_id, &cmd_refs).await
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

fn archive_build_context_path(context: &str) -> String {
    if context == "." {
        "/tmp/coast-build".to_string()
    } else {
        format!("/tmp/coast-build/{context}")
    }
}

fn docker_build_command(
    instance_tag: &str,
    build_context: &str,
    dockerfile: Option<&str>,
) -> Vec<String> {
    let mut build_cmd = vec![
        "docker".to_string(),
        "build".to_string(),
        "-t".to_string(),
        instance_tag.to_string(),
    ];
    if let Some(dockerfile) = dockerfile {
        if dockerfile != "Dockerfile" {
            build_cmd.push("-f".to_string());
            build_cmd.push(format!("{build_context}/{dockerfile}"));
        }
    }
    build_cmd.push(build_context.to_string());
    build_cmd
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
        service_volumes: build_service_volume_overrides(
            compose_content.as_deref(),
            request.secret_container_paths,
            Some(paths::SHARED_CADDY_PKI_CONTAINER_PATH),
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

fn build_service_volume_overrides(
    compose_content: Option<&str>,
    secret_container_paths: &[String],
    shared_caddy_pki_container_path: Option<&str>,
) -> HashMap<String, Vec<String>> {
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
    let shared_caddy_mount =
        shared_caddy_pki_container_path.map(|path| format!("{path}:/data/caddy/pki"));

    services
        .iter()
        .filter_map(|(service_name, service_definition)| {
            let service_name = service_name.as_str()?.to_string();
            let service_definition = service_definition.as_mapping()?;
            let mut mounts = secret_mounts.clone();

            if let Some(ref caddy_mount) = shared_caddy_mount {
                let existing_pki_target = service_definition
                    .get(serde_yaml::Value::String("volumes".into()))
                    .and_then(serde_yaml::Value::as_sequence)
                    .is_some_and(|volumes| {
                        volumes.iter().any(|volume| {
                            volume_mount_target(volume)
                                .is_some_and(|target| target == "/data/caddy/pki")
                        })
                    });

                if !existing_pki_target && is_caddy_service(service_definition) {
                    mounts.push(caddy_mount.clone());
                }
            }

            (!mounts.is_empty()).then_some((service_name, mounts))
        })
        .collect()
}

fn is_caddy_service(service_definition: &serde_yaml::Mapping) -> bool {
    service_image_name(service_definition)
        .is_some_and(|image| image_repo_basename(image) == "caddy")
        || service_mounts_etc_caddy(service_definition)
}

fn service_image_name(service_definition: &serde_yaml::Mapping) -> Option<&str> {
    service_definition
        .get(serde_yaml::Value::String("image".into()))
        .and_then(serde_yaml::Value::as_str)
}

fn image_repo_basename(image: &str) -> &str {
    let without_digest = image.split('@').next().unwrap_or(image);
    let last_segment = without_digest.rsplit('/').next().unwrap_or(without_digest);
    last_segment.split(':').next().unwrap_or(last_segment)
}

fn service_mounts_etc_caddy(service_definition: &serde_yaml::Mapping) -> bool {
    service_definition
        .get(serde_yaml::Value::String("volumes".into()))
        .and_then(serde_yaml::Value::as_sequence)
        .is_some_and(|volumes| {
            volumes.iter().any(|volume| {
                volume_mount_target(volume).is_some_and(|target| {
                    target == "/etc/caddy" || target == "/etc/caddy/Caddyfile"
                })
            })
        })
}

fn volume_mount_target(volume: &serde_yaml::Value) -> Option<&str> {
    match volume {
        serde_yaml::Value::String(entry) => {
            let parts: Vec<&str> = entry.splitn(3, ':').collect();
            (parts.len() >= 2).then_some(parts[1])
        }
        serde_yaml::Value::Mapping(mapping) => mapping
            .get(serde_yaml::Value::String("target".into()))
            .or_else(|| mapping.get(serde_yaml::Value::String("destination".into())))
            .and_then(serde_yaml::Value::as_str),
        _ => None,
    }
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
    fn test_archive_build_context_path_uses_tmp_root_for_dot() {
        assert_eq!(archive_build_context_path("."), "/tmp/coast-build");
        assert_eq!(
            archive_build_context_path("apps/web"),
            "/tmp/coast-build/apps/web"
        );
    }

    #[test]
    fn test_docker_build_command_includes_non_default_dockerfile() {
        let command = docker_build_command(
            "coast-built/proj/web:dev-1",
            "/tmp/coast-build/apps/web",
            Some("Dockerfile.dev"),
        );

        assert_eq!(
            command,
            vec![
                "docker",
                "build",
                "-t",
                "coast-built/proj/web:dev-1",
                "-f",
                "/tmp/coast-build/apps/web/Dockerfile.dev",
                "/tmp/coast-build/apps/web"
            ]
        );
    }

    #[test]
    fn test_docker_build_command_skips_default_dockerfile_flag() {
        let command = docker_build_command(
            "coast-built/proj/web:dev-1",
            "/tmp/coast-build",
            Some("Dockerfile"),
        );

        assert_eq!(
            command,
            vec![
                "docker",
                "build",
                "-t",
                "coast-built/proj/web:dev-1",
                "/tmp/coast-build"
            ]
        );
    }

    #[test]
    fn test_build_service_volume_overrides_applies_secret_mounts_to_all_services() {
        let compose = r#"
services:
  web:
    image: app
  worker:
    image: jobs
"#;
        let overrides = build_service_volume_overrides(
            Some(compose),
            &["/run/secrets/api_key".to_string()],
            None,
        );

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
    fn test_build_service_volume_overrides_adds_shared_caddy_mount() {
        let compose = r#"
services:
  proxy:
    image: caddy:2
"#;
        let overrides = build_service_volume_overrides(
            Some(compose),
            &[],
            Some(paths::SHARED_CADDY_PKI_CONTAINER_PATH),
        );

        assert_eq!(
            overrides.get("proxy"),
            Some(&vec!["/coast-caddy-pki:/data/caddy/pki".to_string()])
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
