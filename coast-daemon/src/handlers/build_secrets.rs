use std::path::Path;

use coast_core::coastfile::Coastfile;
use coast_core::error::{CoastError, Result};
use coast_core::types::BuildSecretConfig;
use coast_docker::compose_build::{ComposeBuildDirective, DockerBuildSecret};

pub(crate) struct MaterializedBuildSecrets {
    pub(crate) secrets: Vec<DockerBuildSecret>,
    _tempdir: tempfile::TempDir,
}

pub(crate) fn materialize_build_secrets(
    coastfile: &Coastfile,
    directive: &ComposeBuildDirective,
) -> Result<MaterializedBuildSecrets> {
    let tempdir = tempfile::tempdir().map_err(|error| CoastError::Io {
        message: format!("failed to create temp dir for build secrets: {error}"),
        path: std::env::temp_dir(),
        source: Some(error),
    })?;

    let registry = coast_secrets::extractor::ExtractorRegistry::with_builtins();
    let mut secrets = Vec::new();

    for secret_ref in &directive.secrets {
        let config = coastfile
            .build_secrets
            .iter()
            .find(|secret| secret.name == secret_ref.source)
            .ok_or_else(|| {
                CoastError::coastfile(format!(
                    "compose build for service '{}' references unknown build secret '{}'",
                    directive.service_name, secret_ref.source
                ))
            })?;

        let path = tempdir.path().join(format!("{}.secret", secret_ref.target));
        let value = extract_build_secret_value(config, &registry, &coastfile.project_root)?;
        std::fs::write(&path, value.as_bytes()).map_err(|error| CoastError::Io {
            message: format!(
                "failed to write build secret '{}' to temp file: {error}",
                config.name
            ),
            path: path.clone(),
            source: Some(error),
        })?;

        secrets.push(DockerBuildSecret {
            id: secret_ref.target.clone(),
            src: path,
        });
    }

    Ok(MaterializedBuildSecrets {
        secrets,
        _tempdir: tempdir,
    })
}

fn extract_build_secret_value(
    config: &BuildSecretConfig,
    registry: &coast_secrets::extractor::ExtractorRegistry,
    project_root: &Path,
) -> Result<coast_secrets::extractor::SecretValue> {
    let mut resolved_params = config.params.clone();
    if let Some(path) = resolved_params.get("path").cloned() {
        let path = Path::new(&path);
        if path.is_relative() {
            let abs = project_root.join(path);
            resolved_params.insert("path".to_string(), path_to_string(&abs));
        }
    }

    registry
        .extract(&config.extractor, &resolved_params)
        .map_err(|error| {
            CoastError::secret(format!(
                "failed to extract build secret '{}' using extractor '{}': {}",
                config.name, config.extractor, error
            ))
        })
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
