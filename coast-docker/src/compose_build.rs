/// Compose file parsing for `build:` and `image:` directives.
///
/// During `coast build`, this module detects services with `build:` directives,
/// builds those images on the host, caches them as tarballs, and rewrites the
/// compose file to reference the pre-built images instead.
use std::path::{Path, PathBuf};

use tracing::info;

use coast_core::error::{CoastError, Result};

/// A single `--build-arg` entry for `docker build`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeBuildArg {
    pub key: String,
    pub value: Option<String>,
}

/// A build secret reference declared under compose `build.secrets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeBuildSecretRef {
    /// Name of the Coast build secret to resolve.
    pub source: String,
    /// BuildKit secret id exposed to the Dockerfile.
    pub target: String,
}

/// A resolved build secret file to pass to `docker build --secret`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerBuildSecret {
    pub id: String,
    pub src: PathBuf,
}

/// A `build:` directive found in a compose service.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposeBuildDirective {
    /// The compose service name (e.g., "app").
    pub service_name: String,
    /// Build context path, relative to the compose file directory.
    pub context: String,
    /// Optional Dockerfile path (relative to context).
    pub dockerfile: Option<String>,
    /// Optional build target stage.
    pub target: Option<String>,
    /// Compose build args to pass through to docker build.
    pub args: Vec<ComposeBuildArg>,
    /// Build secrets referenced by this service.
    pub secrets: Vec<ComposeBuildSecretRef>,
    /// The coast-built image tag (e.g., "coast-built/my-project/app:latest").
    pub coast_image_tag: String,
}

/// Result of parsing a compose file for image references.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposeParseResult {
    /// Services with `build:` directives.
    pub build_directives: Vec<ComposeBuildDirective>,
    /// Image references from services with `image:` directives.
    pub image_refs: Vec<String>,
}

impl ComposeParseResult {
    /// Return a new result with omitted services filtered out (build directives only).
    ///
    /// **Prefer [`parse_compose_file_filtered`] instead** — it filters both build
    /// directives and image refs at parse time.
    pub fn without_services(&self, omit: &[String]) -> Self {
        if omit.is_empty() {
            return self.clone();
        }
        let omit_set: std::collections::HashSet<&str> =
            omit.iter().map(std::string::String::as_str).collect();

        ComposeParseResult {
            build_directives: self
                .build_directives
                .iter()
                .filter(|d| !omit_set.contains(d.service_name.as_str()))
                .cloned()
                .collect(),
            image_refs: self.image_refs.clone(),
        }
    }
}

/// Generate a deterministic image tag for a coast-built image.
///
/// Format: `coast-built/{project}/{service}:latest`
pub fn coast_built_image_tag(project: &str, service: &str) -> String {
    format!("coast-built/{project}/{service}:latest")
}

/// Parse a compose file, skipping services listed in `omit_services`.
///
/// Both `build:` directives and `image:` refs from omitted services are excluded
/// from the result, so their images are neither built nor pulled during `coast build`.
pub fn parse_compose_file_filtered(
    content: &str,
    project: &str,
    omit_services: &[String],
) -> Result<ComposeParseResult> {
    let omit_set: std::collections::HashSet<&str> = omit_services
        .iter()
        .map(std::string::String::as_str)
        .collect();
    parse_compose_file_inner(content, project, &omit_set)
}

/// Parse a compose file's YAML content to find `build:` and `image:` directives.
///
/// Handles both short-form (`build: .`) and long-form (`build: { context: ..., dockerfile: ... }`)
/// build directives. Services with both `build:` and `image:` are treated as build directives
/// (the existing `image:` is overridden).
pub fn parse_compose_file(content: &str, project: &str) -> Result<ComposeParseResult> {
    parse_compose_file_inner(content, project, &std::collections::HashSet::new())
}

fn parse_compose_file_inner(
    content: &str,
    project: &str,
    omit_services: &std::collections::HashSet<&str>,
) -> Result<ComposeParseResult> {
    let doc: serde_yaml::Value = serde_yaml::from_str(content)
        .map_err(|e| CoastError::coastfile(format!("failed to parse compose YAML: {e}")))?;

    let services = match doc.get("services") {
        Some(serde_yaml::Value::Mapping(m)) => m,
        Some(_) => {
            return Err(CoastError::coastfile(
                "compose file 'services' key is not a mapping",
            ));
        }
        None => {
            return Ok(ComposeParseResult {
                build_directives: Vec::new(),
                image_refs: Vec::new(),
            })
        }
    };

    let mut build_directives = Vec::new();
    let mut image_refs = Vec::new();

    for (key, value) in services {
        let service_name = key.as_str().unwrap_or_default().to_string();
        if service_name.is_empty() {
            continue;
        }

        if omit_services.contains(service_name.as_str()) {
            continue;
        }

        let has_build = value.get("build").is_some();
        let has_image = value.get("image").is_some();

        if has_build {
            let build_val = value.get("build").unwrap();
            let (context, dockerfile, target, args, secrets) = match build_val {
                serde_yaml::Value::String(s) => (s.clone(), None, None, Vec::new(), Vec::new()),
                serde_yaml::Value::Mapping(m) => {
                    let ctx = m
                        .get(serde_yaml::Value::String("context".to_string()))
                        .and_then(|v| v.as_str())
                        .unwrap_or(".")
                        .to_string();
                    let df = m
                        .get(serde_yaml::Value::String("dockerfile".to_string()))
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string);
                    let target = m
                        .get(serde_yaml::Value::String("target".to_string()))
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string);
                    let args =
                        parse_build_args(m.get(serde_yaml::Value::String("args".to_string())));
                    let secrets = parse_build_secrets(
                        m.get(serde_yaml::Value::String("secrets".to_string())),
                    );
                    (ctx, df, target, args, secrets)
                }
                _ => (".".to_string(), None, None, Vec::new(), Vec::new()),
            };

            build_directives.push(ComposeBuildDirective {
                service_name: service_name.clone(),
                context,
                dockerfile,
                target,
                args,
                secrets,
                coast_image_tag: coast_built_image_tag(project, &service_name),
            });
        } else if has_image {
            if let Some(img) = value.get("image").and_then(|v| v.as_str()) {
                if !img.is_empty() {
                    image_refs.push(img.to_string());
                }
            }
        }
    }

    Ok(ComposeParseResult {
        build_directives,
        image_refs,
    })
}

fn parse_build_args(value: Option<&serde_yaml::Value>) -> Vec<ComposeBuildArg> {
    match value {
        Some(serde_yaml::Value::Mapping(map)) => map
            .iter()
            .filter_map(|(key, value)| {
                let key = key.as_str()?.to_string();
                let value = match value {
                    serde_yaml::Value::Null => None,
                    serde_yaml::Value::String(s) => Some(s.clone()),
                    other => Some(match serde_yaml::to_string(other) {
                        Ok(rendered) => rendered.trim().to_string(),
                        Err(_) => other.as_str().unwrap_or_default().to_string(),
                    }),
                };
                Some(ComposeBuildArg { key, value })
            })
            .collect(),
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(serde_yaml::Value::as_str)
            .filter(|entry| !entry.trim().is_empty())
            .map(|entry| {
                if let Some((key, value)) = entry.split_once('=') {
                    ComposeBuildArg {
                        key: key.to_string(),
                        value: Some(value.to_string()),
                    }
                } else {
                    ComposeBuildArg {
                        key: entry.to_string(),
                        value: None,
                    }
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_build_secrets(value: Option<&serde_yaml::Value>) -> Vec<ComposeBuildSecretRef> {
    let Some(serde_yaml::Value::Sequence(seq)) = value else {
        return Vec::new();
    };

    seq.iter()
        .filter_map(|entry| match entry {
            serde_yaml::Value::String(name) if !name.trim().is_empty() => {
                Some(ComposeBuildSecretRef {
                    source: name.clone(),
                    target: name.clone(),
                })
            }
            serde_yaml::Value::Mapping(map) => {
                let source = map
                    .get(serde_yaml::Value::String("source".to_string()))
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.trim().is_empty())?
                    .to_string();
                let target = map
                    .get(serde_yaml::Value::String("target".to_string()))
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.trim().is_empty())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_else(|| source.clone());
                Some(ComposeBuildSecretRef { source, target })
            }
            _ => None,
        })
        .collect()
}

/// Rewrite a compose file to replace `build:` directives with `image:` references.
///
/// For each service with a `build:` key, removes the `build` key and sets `image`
/// to the coast-built tag. Services that already use `image:` are left unchanged.
///
/// Returns the modified YAML as a string. Note: YAML comments are lost since
/// the artifact compose is machine-consumed.
pub fn rewrite_compose_for_artifact(content: &str, project: &str) -> Result<String> {
    let mut doc: serde_yaml::Value = serde_yaml::from_str(content)
        .map_err(|e| CoastError::coastfile(format!("failed to parse compose YAML: {e}")))?;

    let Some(serde_yaml::Value::Mapping(services)) = doc.get_mut("services") else {
        return Ok(content.to_string());
    };

    for (key, value) in services.iter_mut() {
        let service_name = key.as_str().unwrap_or_default();
        if service_name.is_empty() {
            continue;
        }

        if let serde_yaml::Value::Mapping(svc) = value {
            let build_key = serde_yaml::Value::String("build".to_string());
            if svc.contains_key(&build_key) {
                svc.remove(&build_key);
                let image_key = serde_yaml::Value::String("image".to_string());
                let tag = coast_built_image_tag(project, service_name);
                svc.insert(image_key, serde_yaml::Value::String(tag));
            }
        }
    }

    serde_yaml::to_string(&doc).map_err(|e| {
        CoastError::coastfile(format!("failed to serialize rewritten compose YAML: {e}"))
    })
}

/// Generate a per-instance image tag for a coast-built image.
///
/// Format: `coast-built/{project}/{service}:{instance_name}`
pub fn coast_built_instance_image_tag(project: &str, service: &str, instance: &str) -> String {
    format!("coast-built/{project}/{service}:{instance}")
}

/// Parse a Dockerfile to extract base image references from `FROM` lines.
///
/// Handles:
/// - `FROM image`
/// - `FROM image AS stage`
/// - `FROM --platform=linux/amd64 image`
/// - `FROM --platform=linux/amd64 image AS stage`
/// - Multi-stage builds (multiple FROM lines)
///
/// Skips:
/// - `FROM scratch` (special Docker keyword, not a real image)
/// - `FROM $VARIABLE` or `FROM ${VARIABLE}` (build arg references)
/// - References to earlier build stages (e.g., `FROM builder`)
///
/// Returns a deduplicated list of base image references.
pub fn parse_dockerfile_base_images(dockerfile_content: &str) -> Vec<String> {
    let mut images = Vec::new();
    let mut stage_names: Vec<String> = Vec::new();

    for line in dockerfile_content.lines() {
        let trimmed = line.trim();
        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Match FROM lines (case-insensitive)
        let upper = trimmed.to_uppercase();
        if !upper.starts_with("FROM ") {
            continue;
        }

        // Parse the FROM line: FROM [--platform=...] image [AS name]
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        // Skip "FROM", then skip any --flags
        let mut idx = 1;
        while idx < parts.len() && parts[idx].starts_with("--") {
            idx += 1;
        }
        if idx >= parts.len() {
            continue;
        }

        let image_ref = parts[idx];

        // Track AS stage names
        if idx + 2 < parts.len() && parts[idx + 1].eq_ignore_ascii_case("AS") {
            stage_names.push(parts[idx + 2].to_lowercase());
        }

        // Skip scratch
        if image_ref.eq_ignore_ascii_case("scratch") {
            continue;
        }

        // Skip variable references ($VAR, ${VAR})
        if image_ref.starts_with('$') {
            continue;
        }

        // Skip references to earlier build stages
        if stage_names.iter().any(|s| s == &image_ref.to_lowercase()) {
            // This FROM references an earlier stage, not an external image
            continue;
        }

        // Deduplicate
        let image_str = image_ref.to_string();
        if !images.contains(&image_str) {
            images.push(image_str);
        }
    }

    images
}

/// Construct the `docker build` command for a build directive.
///
/// Returns the command as a vector of strings suitable for `tokio::process::Command`.
pub fn docker_build_cmd(
    directive: &ComposeBuildDirective,
    compose_dir: &Path,
    build_secrets: &[DockerBuildSecret],
) -> Vec<String> {
    let mut cmd = vec![
        "docker".to_string(),
        "build".to_string(),
        "-t".to_string(),
        directive.coast_image_tag.clone(),
    ];

    if let Some(ref df) = directive.dockerfile {
        cmd.push("-f".to_string());
        cmd.push(
            compose_dir
                .join(&directive.context)
                .join(df)
                .display()
                .to_string(),
        );
    }

    if let Some(ref target) = directive.target {
        cmd.push("--target".to_string());
        cmd.push(target.clone());
    }

    for arg in &directive.args {
        cmd.push("--build-arg".to_string());
        cmd.push(match &arg.value {
            Some(value) => format!("{}={value}", arg.key),
            None => arg.key.clone(),
        });
    }

    for secret in build_secrets {
        cmd.push("--secret".to_string());
        cmd.push(format!("id={},src={}", secret.id, secret.src.display()));
    }

    cmd.push(compose_dir.join(&directive.context).display().to_string());

    cmd
}

/// Build an image on the host and save it as a tarball in the cache directory.
///
/// Runs `docker build` followed by `docker save` using `tokio::process::Command`.
/// Returns the path to the saved tarball.
pub async fn build_and_cache_image(
    directive: &ComposeBuildDirective,
    compose_dir: &Path,
    cache_dir: &Path,
    build_secrets: &[DockerBuildSecret],
) -> Result<PathBuf> {
    let cmd_args = docker_build_cmd(directive, compose_dir, build_secrets);
    info!(
        service = %directive.service_name,
        tag = %directive.coast_image_tag,
        "building image from compose build: directive"
    );

    // Run docker build
    let output = tokio::process::Command::new(&cmd_args[0])
        .args(&cmd_args[1..])
        .output()
        .await
        .map_err(|e| {
            CoastError::docker(format!(
                "failed to run docker build for service '{}': {}. \
                 Ensure Docker is running and the build context exists at '{}'.",
                directive.service_name,
                e,
                compose_dir.join(&directive.context).display()
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CoastError::docker(format!(
            "docker build failed for service '{}' (exit code {}):\n{}",
            directive.service_name,
            output.status.code().unwrap_or(-1),
            stderr
        )));
    }

    info!(
        service = %directive.service_name,
        tag = %directive.coast_image_tag,
        "image built successfully, saving tarball"
    );

    // Save as tarball using the image_cache naming convention
    let tarball_filename = crate::image_cache::tarball_filename(&directive.coast_image_tag);
    let tarball_path = cache_dir.join(&tarball_filename);

    let save_output = tokio::process::Command::new("docker")
        .args([
            "save",
            "-o",
            &tarball_path.display().to_string(),
            &directive.coast_image_tag,
        ])
        .output()
        .await
        .map_err(|e| {
            CoastError::docker(format!(
                "failed to run docker save for image '{}': {}",
                directive.coast_image_tag, e
            ))
        })?;

    if !save_output.status.success() {
        let stderr = String::from_utf8_lossy(&save_output.stderr);
        return Err(CoastError::docker(format!(
            "docker save failed for image '{}' (exit code {}):\n{}",
            directive.coast_image_tag,
            save_output.status.code().unwrap_or(-1),
            stderr
        )));
    }

    info!(
        service = %directive.service_name,
        tarball = %tarball_path.display(),
        "image tarball cached"
    );

    Ok(tarball_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coast_built_image_tag() {
        assert_eq!(
            coast_built_image_tag("my-app", "web"),
            "coast-built/my-app/web:latest"
        );
    }

    #[test]
    fn test_coast_built_image_tag_special_chars() {
        assert_eq!(
            coast_built_image_tag("my-project", "api-server"),
            "coast-built/my-project/api-server:latest"
        );
    }

    #[test]
    fn test_parse_simple_build_string() {
        let yaml = r#"
services:
  app:
    build: .
    ports:
      - "3000:3000"
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert_eq!(result.build_directives.len(), 1);
        assert_eq!(result.build_directives[0].service_name, "app");
        assert_eq!(result.build_directives[0].context, ".");
        assert!(result.build_directives[0].dockerfile.is_none());
        assert_eq!(
            result.build_directives[0].coast_image_tag,
            "coast-built/proj/app:latest"
        );
        assert!(result.image_refs.is_empty());
    }

    #[test]
    fn test_parse_build_object() {
        let yaml = r#"
services:
  app:
    build:
      context: ./docker
      dockerfile: Dockerfile.prod
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert_eq!(result.build_directives.len(), 1);
        assert_eq!(result.build_directives[0].context, "./docker");
        assert_eq!(
            result.build_directives[0].dockerfile,
            Some("Dockerfile.prod".to_string())
        );
    }

    #[test]
    fn test_parse_build_object_context_only() {
        let yaml = r#"
services:
  app:
    build:
      context: ./src
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert_eq!(result.build_directives.len(), 1);
        assert_eq!(result.build_directives[0].context, "./src");
        assert!(result.build_directives[0].dockerfile.is_none());
    }

    #[test]
    fn test_parse_image_only() {
        let yaml = r#"
services:
  db:
    image: postgres:16
  cache:
    image: redis:7
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert!(result.build_directives.is_empty());
        assert_eq!(result.image_refs.len(), 2);
        assert!(result.image_refs.contains(&"postgres:16".to_string()));
        assert!(result.image_refs.contains(&"redis:7".to_string()));
    }

    #[test]
    fn test_parse_mixed_build_and_image() {
        let yaml = r#"
services:
  app:
    build: .
  db:
    image: postgres:16
  worker:
    build:
      context: ./worker
      dockerfile: Dockerfile
"#;
        let result = parse_compose_file(yaml, "my-app").unwrap();
        assert_eq!(result.build_directives.len(), 2);
        assert_eq!(result.image_refs.len(), 1);
        assert_eq!(result.image_refs[0], "postgres:16");

        let app_dir = result
            .build_directives
            .iter()
            .find(|d| d.service_name == "app")
            .unwrap();
        assert_eq!(app_dir.context, ".");

        let worker_dir = result
            .build_directives
            .iter()
            .find(|d| d.service_name == "worker")
            .unwrap();
        assert_eq!(worker_dir.context, "./worker");
        assert_eq!(worker_dir.dockerfile, Some("Dockerfile".to_string()));
    }

    #[test]
    fn test_parse_empty_services() {
        let yaml = r#"
services: {}
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert!(result.build_directives.is_empty());
        assert!(result.image_refs.is_empty());
    }

    #[test]
    fn test_parse_no_services_key() {
        let yaml = r#"
version: '3'
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert!(result.build_directives.is_empty());
        assert!(result.image_refs.is_empty());
    }

    #[test]
    fn test_parse_service_with_both_build_and_image() {
        // When a service has both build: and image:, build: takes precedence
        let yaml = r#"
services:
  app:
    build: .
    image: myapp:latest
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert_eq!(result.build_directives.len(), 1);
        assert!(result.image_refs.is_empty());
    }

    #[test]
    fn test_parse_invalid_yaml() {
        let yaml = "not: valid: yaml: [";
        let result = parse_compose_file(yaml, "proj");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_services_not_mapping() {
        let yaml = r#"
services: "not a mapping"
"#;
        let result = parse_compose_file(yaml, "proj");
        assert!(result.is_err());
    }

    #[test]
    fn test_rewrite_removes_build_adds_image() {
        let yaml = r#"
services:
  app:
    build: .
    ports:
      - "3000:3000"
  db:
    image: postgres:16
"#;
        let rewritten = rewrite_compose_for_artifact(yaml, "my-proj").unwrap();

        // Parse the rewritten YAML to verify structure
        let doc: serde_yaml::Value = serde_yaml::from_str(&rewritten).unwrap();
        let services = doc.get("services").unwrap();

        // app should have image and no build
        let app = services.get("app").unwrap();
        assert!(app.get("build").is_none());
        assert_eq!(
            app.get("image").unwrap().as_str().unwrap(),
            "coast-built/my-proj/app:latest"
        );
        // ports should be preserved
        assert!(app.get("ports").is_some());

        // db should be unchanged
        let db = services.get("db").unwrap();
        assert_eq!(db.get("image").unwrap().as_str().unwrap(), "postgres:16");
    }

    #[test]
    fn test_rewrite_no_build_directives() {
        let yaml = r#"
services:
  db:
    image: postgres:16
"#;
        let rewritten = rewrite_compose_for_artifact(yaml, "proj").unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&rewritten).unwrap();
        let db = doc.get("services").unwrap().get("db").unwrap();
        assert_eq!(db.get("image").unwrap().as_str().unwrap(), "postgres:16");
    }

    #[test]
    fn test_rewrite_invalid_yaml() {
        let result = rewrite_compose_for_artifact("not: valid: [", "proj");
        assert!(result.is_err());
    }

    #[test]
    fn test_rewrite_no_services() {
        let yaml = "version: '3'\n";
        let rewritten = rewrite_compose_for_artifact(yaml, "proj").unwrap();
        // Should pass through without error
        assert!(!rewritten.is_empty());
    }

    #[test]
    fn test_docker_build_cmd_simple() {
        let directive = ComposeBuildDirective {
            service_name: "app".to_string(),
            context: ".".to_string(),
            dockerfile: None,
            target: None,
            args: Vec::new(),
            secrets: Vec::new(),
            coast_image_tag: "coast-built/proj/app:latest".to_string(),
        };
        let cmd = docker_build_cmd(&directive, Path::new("/home/user/project"), &[]);
        assert_eq!(cmd[0], "docker");
        assert_eq!(cmd[1], "build");
        assert_eq!(cmd[2], "-t");
        assert_eq!(cmd[3], "coast-built/proj/app:latest");
        assert_eq!(cmd[4], "/home/user/project/.");
    }

    #[test]
    fn test_docker_build_cmd_with_dockerfile() {
        let directive = ComposeBuildDirective {
            service_name: "app".to_string(),
            context: "./docker".to_string(),
            dockerfile: Some("Dockerfile.prod".to_string()),
            target: None,
            args: Vec::new(),
            secrets: Vec::new(),
            coast_image_tag: "coast-built/proj/app:latest".to_string(),
        };
        let cmd = docker_build_cmd(&directive, Path::new("/project"), &[]);
        assert_eq!(cmd[0], "docker");
        assert_eq!(cmd[1], "build");
        assert_eq!(cmd[2], "-t");
        assert_eq!(cmd[3], "coast-built/proj/app:latest");
        assert_eq!(cmd[4], "-f");
        assert_eq!(cmd[5], "/project/./docker/Dockerfile.prod");
        assert_eq!(cmd[6], "/project/./docker");
    }

    #[test]
    fn test_docker_build_cmd_subdir_context() {
        let directive = ComposeBuildDirective {
            service_name: "worker".to_string(),
            context: "./services/worker".to_string(),
            dockerfile: None,
            target: None,
            args: Vec::new(),
            secrets: Vec::new(),
            coast_image_tag: "coast-built/proj/worker:latest".to_string(),
        };
        let cmd = docker_build_cmd(&directive, Path::new("/app"), &[]);
        assert_eq!(cmd.last().unwrap(), "/app/./services/worker");
    }

    #[test]
    fn test_parse_build_with_extra_fields() {
        let yaml = r#"
services:
  app:
    build:
      context: .
      dockerfile: Dockerfile
      args:
        NODE_ENV: production
      target: builder
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert_eq!(result.build_directives.len(), 1);
        assert_eq!(result.build_directives[0].context, ".");
        assert_eq!(
            result.build_directives[0].dockerfile,
            Some("Dockerfile".to_string())
        );
        assert_eq!(
            result.build_directives[0].target.as_deref(),
            Some("builder")
        );
        assert_eq!(
            result.build_directives[0].args,
            vec![ComposeBuildArg {
                key: "NODE_ENV".to_string(),
                value: Some("production".to_string())
            }]
        );
    }

    #[test]
    fn test_parse_build_secrets_short_and_long_form() {
        let yaml = r#"
services:
  app:
    build:
      context: .
      secrets:
        - npm_token
        - source: github_token
          target: gh_token
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        assert_eq!(
            result.build_directives[0].secrets,
            vec![
                ComposeBuildSecretRef {
                    source: "npm_token".to_string(),
                    target: "npm_token".to_string(),
                },
                ComposeBuildSecretRef {
                    source: "github_token".to_string(),
                    target: "gh_token".to_string(),
                }
            ]
        );
    }

    #[test]
    fn test_docker_build_cmd_includes_target_args_and_secrets() {
        let directive = ComposeBuildDirective {
            service_name: "app".to_string(),
            context: ".".to_string(),
            dockerfile: Some("Dockerfile".to_string()),
            target: Some("runtime".to_string()),
            args: vec![ComposeBuildArg {
                key: "NODE_ENV".to_string(),
                value: Some("production".to_string()),
            }],
            secrets: vec![ComposeBuildSecretRef {
                source: "npm_token".to_string(),
                target: "npm_token".to_string(),
            }],
            coast_image_tag: "coast-built/proj/app:latest".to_string(),
        };
        let cmd = docker_build_cmd(
            &directive,
            Path::new("/project"),
            &[DockerBuildSecret {
                id: "npm_token".to_string(),
                src: PathBuf::from("/tmp/npm_token"),
            }],
        );
        assert!(cmd.contains(&"--target".to_string()));
        assert!(cmd.contains(&"runtime".to_string()));
        assert!(cmd.contains(&"--build-arg".to_string()));
        assert!(cmd.contains(&"NODE_ENV=production".to_string()));
        assert!(cmd.contains(&"--secret".to_string()));
        assert!(cmd.contains(&"id=npm_token,src=/tmp/npm_token".to_string()));
    }

    #[test]
    fn test_rewrite_preserves_other_service_config() {
        let yaml = r#"
services:
  app:
    build: .
    ports:
      - "3000:3000"
    environment:
      - NODE_ENV=production
    volumes:
      - ./data:/data
"#;
        let rewritten = rewrite_compose_for_artifact(yaml, "proj").unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&rewritten).unwrap();
        let app = doc.get("services").unwrap().get("app").unwrap();

        // build should be gone, image should be added
        assert!(app.get("build").is_none());
        assert!(app.get("image").is_some());

        // Other config should be preserved
        assert!(app.get("ports").is_some());
        assert!(app.get("environment").is_some());
        assert!(app.get("volumes").is_some());
    }

    #[test]
    fn test_rewrite_multiple_build_services() {
        let yaml = r#"
services:
  app:
    build: .
  worker:
    build:
      context: ./worker
"#;
        let rewritten = rewrite_compose_for_artifact(yaml, "proj").unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&rewritten).unwrap();
        let services = doc.get("services").unwrap();

        let app = services.get("app").unwrap();
        assert!(app.get("build").is_none());
        assert_eq!(
            app.get("image").unwrap().as_str().unwrap(),
            "coast-built/proj/app:latest"
        );

        let worker = services.get("worker").unwrap();
        assert!(worker.get("build").is_none());
        assert_eq!(
            worker.get("image").unwrap().as_str().unwrap(),
            "coast-built/proj/worker:latest"
        );
    }

    // -------------------------------------------------------
    // coast_built_instance_image_tag tests
    // -------------------------------------------------------

    #[test]
    fn test_coast_built_instance_image_tag() {
        assert_eq!(
            coast_built_instance_image_tag("my-app", "web", "feature-01"),
            "coast-built/my-app/web:feature-01"
        );
    }

    #[test]
    fn test_coast_built_instance_image_tag_main() {
        assert_eq!(
            coast_built_instance_image_tag("proj", "app", "main"),
            "coast-built/proj/app:main"
        );
    }

    // -------------------------------------------------------
    // parse_dockerfile_base_images tests
    // -------------------------------------------------------

    #[test]
    fn test_parse_dockerfile_simple_from() {
        let dockerfile = "FROM node:20-alpine\nRUN npm install\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert_eq!(images, vec!["node:20-alpine"]);
    }

    #[test]
    fn test_parse_dockerfile_from_with_as() {
        let dockerfile = "FROM node:20-alpine AS builder\nRUN npm install\nFROM nginx:alpine\nCOPY --from=builder /app /app\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert_eq!(images, vec!["node:20-alpine", "nginx:alpine"]);
    }

    #[test]
    fn test_parse_dockerfile_from_with_platform() {
        let dockerfile = "FROM --platform=linux/amd64 node:20-alpine\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert_eq!(images, vec!["node:20-alpine"]);
    }

    #[test]
    fn test_parse_dockerfile_from_platform_and_as() {
        let dockerfile = "FROM --platform=linux/amd64 node:20-alpine AS builder\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert_eq!(images, vec!["node:20-alpine"]);
    }

    #[test]
    fn test_parse_dockerfile_skip_scratch() {
        let dockerfile = "FROM scratch\nCOPY binary /\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert!(images.is_empty());
    }

    #[test]
    fn test_parse_dockerfile_skip_variable() {
        let dockerfile = "ARG BASE=node:20\nFROM $BASE\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert!(images.is_empty());
    }

    #[test]
    fn test_parse_dockerfile_skip_variable_braces() {
        let dockerfile = "ARG BASE=node:20\nFROM ${BASE}\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert!(images.is_empty());
    }

    #[test]
    fn test_parse_dockerfile_skip_stage_reference() {
        let dockerfile = "FROM node:20 AS builder\nRUN npm build\nFROM builder\nRUN something\n";
        let images = parse_dockerfile_base_images(dockerfile);
        // Only node:20, not "builder" (which is a stage name)
        assert_eq!(images, vec!["node:20"]);
    }

    #[test]
    fn test_parse_dockerfile_multistage_dedup() {
        let dockerfile =
            "FROM node:20 AS deps\nRUN npm ci\nFROM node:20 AS builder\nRUN npm build\n";
        let images = parse_dockerfile_base_images(dockerfile);
        // node:20 should appear only once
        assert_eq!(images, vec!["node:20"]);
    }

    #[test]
    fn test_parse_dockerfile_comments_and_empty_lines() {
        let dockerfile =
            "# This is a comment\n\nFROM node:20-alpine\n# Another comment\nRUN echo hi\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert_eq!(images, vec!["node:20-alpine"]);
    }

    #[test]
    fn test_parse_dockerfile_case_insensitive() {
        let dockerfile = "from node:20-alpine\n";
        let images = parse_dockerfile_base_images(dockerfile);
        assert_eq!(images, vec!["node:20-alpine"]);
    }

    #[test]
    fn test_parse_dockerfile_empty() {
        let images = parse_dockerfile_base_images("");
        assert!(images.is_empty());
    }

    #[test]
    fn test_parse_dockerfile_no_from() {
        let images = parse_dockerfile_base_images("RUN echo hello\nCOPY . .\n");
        assert!(images.is_empty());
    }

    #[test]
    fn test_parse_dockerfile_complex_multistage() {
        let dockerfile = "\
FROM node:20-alpine AS deps
RUN npm ci

FROM node:20-alpine AS builder
COPY --from=deps /app/node_modules ./node_modules
RUN npm run build

FROM --platform=linux/amd64 nginx:1.25-alpine
COPY --from=builder /app/dist /usr/share/nginx/html
";
        let images = parse_dockerfile_base_images(dockerfile);
        assert_eq!(images, vec!["node:20-alpine", "nginx:1.25-alpine"]);
    }

    // -------------------------------------------------------
    // without_services (omit) tests
    // -------------------------------------------------------

    #[test]
    fn test_without_services_empty_omit() {
        let yaml = r#"
services:
  app:
    build: .
  db:
    image: postgres:16
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        let filtered = result.without_services(&[]);
        assert_eq!(filtered.build_directives.len(), 1);
        assert_eq!(filtered.image_refs.len(), 1);
    }

    #[test]
    fn test_without_services_removes_build_directive() {
        let yaml = r#"
services:
  app:
    build: .
  worker:
    build: ./worker
  db:
    image: postgres:16
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        let filtered = result.without_services(&["worker".to_string()]);
        assert_eq!(filtered.build_directives.len(), 1);
        assert_eq!(filtered.build_directives[0].service_name, "app");
        assert_eq!(filtered.image_refs.len(), 1);
    }

    #[test]
    fn test_without_services_removes_multiple() {
        let yaml = r#"
services:
  app:
    build: .
  keycloak:
    image: quay.io/keycloak/keycloak
  redash:
    build: ./redash
  db:
    image: postgres:16
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        let filtered = result.without_services(&["keycloak".to_string(), "redash".to_string()]);
        assert_eq!(filtered.build_directives.len(), 1);
        assert_eq!(filtered.build_directives[0].service_name, "app");
    }

    #[test]
    fn test_without_services_nonexistent_name_is_noop() {
        let yaml = r#"
services:
  app:
    build: .
"#;
        let result = parse_compose_file(yaml, "proj").unwrap();
        let filtered = result.without_services(&["nonexistent".to_string()]);
        assert_eq!(filtered.build_directives.len(), 1);
    }

    // -------------------------------------------------------
    // parse_compose_file_filtered tests
    // -------------------------------------------------------

    #[test]
    fn test_filtered_skips_image_refs_for_omitted_services() {
        let yaml = r#"
services:
  app:
    build: .
  keycloak:
    image: bitnami/keycloak:26
  redash:
    image: redash/redash:25
  redash-worker:
    image: redash/redash:25
  langfuse:
    image: langfuse/langfuse:2
  db:
    image: postgres:16
"#;
        let omit = vec![
            "keycloak".to_string(),
            "redash".to_string(),
            "redash-worker".to_string(),
            "langfuse".to_string(),
        ];
        let result = parse_compose_file_filtered(yaml, "proj", &omit).unwrap();

        assert_eq!(result.build_directives.len(), 1);
        assert_eq!(result.build_directives[0].service_name, "app");

        // Only postgres:16 should remain — keycloak, redash, langfuse images are omitted
        assert_eq!(result.image_refs.len(), 1);
        assert_eq!(result.image_refs[0], "postgres:16");
    }

    #[test]
    fn test_filtered_empty_omit_same_as_unfiltered() {
        let yaml = r#"
services:
  app:
    build: .
  db:
    image: postgres:16
  redis:
    image: redis:7
"#;
        let unfiltered = parse_compose_file(yaml, "proj").unwrap();
        let filtered = parse_compose_file_filtered(yaml, "proj", &[]).unwrap();
        assert_eq!(unfiltered, filtered);
    }

    #[test]
    fn test_filtered_omits_build_directives_and_images() {
        let yaml = r#"
services:
  app:
    build: .
  debug:
    build: ./debug
  nginx:
    image: nginx:latest
  db:
    image: postgres:16
"#;
        let omit = vec!["debug".to_string(), "nginx".to_string()];
        let result = parse_compose_file_filtered(yaml, "proj", &omit).unwrap();
        assert_eq!(result.build_directives.len(), 1);
        assert_eq!(result.build_directives[0].service_name, "app");
        assert_eq!(result.image_refs, vec!["postgres:16"]);
    }
}
