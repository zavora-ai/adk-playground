//! Configuration module for the sandbox workspace agent example.
//!
//! Provides [`build_manifest`] for constructing the workspace manifest
//! and [`build_sandbox_config`] for constructing the full [`SandboxConfig`]
//! based on CLI arguments.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use adk_sandbox::workspace::{Capability, Manifest, ManifestEntry, SandboxClient, SandboxConfig};

use crate::CliArgs;

/// Constructs the workspace manifest for the Rust hello-world project.
///
/// Creates a manifest with directory entries for the project root and
/// its `src/` subdirectory. The agent will populate these directories
/// with `Cargo.toml` and `src/main.rs` during execution.
pub fn build_manifest() -> Manifest {
    Manifest::new(vec![
        ManifestEntry::Directory { path: "hello-world".to_string() },
        ManifestEntry::Directory { path: "hello-world/src".to_string() },
    ])
}

/// Constructs the [`SandboxConfig`] based on CLI arguments.
///
/// - Default: `LocalUnixClient` with `/tmp/adk-sandbox-snapshots` snapshot directory
/// - `--docker`: `DockerClient` with `rust:latest` base image (requires `workspace-docker` feature)
/// - `--snapshot`: enables `snapshot_on_stop`
/// - Session timeout: 120 seconds
/// - Command timeout: 60 seconds
/// - Capabilities: `Shell` + `Filesystem`
///
/// # Errors
///
/// Returns an error if `--docker` is specified but the `workspace-docker` feature
/// is not enabled, or if Docker is unavailable.
pub async fn build_sandbox_config(args: &CliArgs) -> anyhow::Result<SandboxConfig> {
    let manifest = build_manifest();

    let capabilities = HashSet::from([Capability::Shell, Capability::Filesystem]);

    let client: Arc<dyn SandboxClient> = if args.docker {
        #[cfg(feature = "workspace-docker")]
        {
            use adk_sandbox::workspace::DockerClient;

            let docker_client = DockerClient::with_image("rust:latest").await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to connect to Docker: {e}\n\
                     Ensure Docker is installed and running:\n  \
                     Install: https://docs.docker.com/get-docker/\n  \
                     Start:   docker info"
                )
            })?;
            Arc::new(docker_client)
        }
        #[cfg(not(feature = "workspace-docker"))]
        {
            anyhow::bail!(
                "Docker support requires the 'workspace-docker' feature.\n\
                 Rebuild with:\n  \
                 cargo run -p sandbox-workspace-agent-example \
                 --features workspace-docker -- --docker"
            );
        }
    } else {
        let snapshot_dir = PathBuf::from("/tmp/adk-sandbox-snapshots");
        Arc::new(adk_sandbox::workspace::LocalUnixClient::new(None, snapshot_dir))
    };

    let config = SandboxConfig::new(client, manifest, capabilities)
        .with_session_timeout(Duration::from_secs(120))
        .with_command_timeout(Duration::from_secs(60))
        .with_snapshot_on_stop(args.snapshot);

    Ok(config)
}
