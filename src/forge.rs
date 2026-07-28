use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::github::GitHubClient;
use crate::repository::RepositoryProvider;

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

#[async_trait]
pub trait Forge: Send + Sync {
    fn provider(&self) -> RepositoryProvider;

    async fn validate(
        &self,
        repository: &Path,
        expected_identity: &str,
        cancellation: &CancellationToken,
    ) -> Result<()>;
}

pub fn forge_for(provider: RepositoryProvider) -> Arc<dyn Forge> {
    forge_for_with_github(provider, GitHubClient::default())
}

pub(crate) fn forge_for_with_github(
    provider: RepositoryProvider,
    github: GitHubClient,
) -> Arc<dyn Forge> {
    match provider {
        RepositoryProvider::GitHub => Arc::new(GitHubForge { client: github }),
        RepositoryProvider::GitLab => Arc::new(GitLabForge::default()),
    }
}

struct GitHubForge {
    client: GitHubClient,
}

#[async_trait]
impl Forge for GitHubForge {
    fn provider(&self) -> RepositoryProvider {
        RepositoryProvider::GitHub
    }

    async fn validate(
        &self,
        repository: &Path,
        expected_identity: &str,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        self.client
            .validate_global(cancellation)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "GitHub CLI is unavailable or unauthenticated; run gh auth login --hostname github.com"
                )
            })?;
        let discovered = self
            .client
            .validate_repository(repository, cancellation)
            .await
            .map_err(|_| {
                anyhow::anyhow!("GitHub CLI cannot read the configured GitHub repository")
            })?
            .to_ascii_lowercase();
        if discovered != expected_identity {
            bail!(
                "GitHub repository identity {discovered} does not match configured identity {expected_identity}"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct GitLabForge {
    executable: PathBuf,
    command_timeout: Duration,
}

impl Default for GitLabForge {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("glab"),
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }
}

#[async_trait]
impl Forge for GitLabForge {
    fn provider(&self) -> RepositoryProvider {
        RepositoryProvider::GitLab
    }

    async fn validate(
        &self,
        repository: &Path,
        expected_identity: &str,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        self.run(None, &["--version"], cancellation)
            .await
            .context("GitLab CLI is unavailable; install glab and ensure it is executable")?;
        self.run(
            None,
            &["auth", "status", "--hostname", "gitlab.com"],
            cancellation,
        )
        .await
        .context("GitLab CLI is not authenticated; run glab auth login --hostname gitlab.com")?;
        let path = self
            .run(
                Some(repository),
                &[
                    "repo",
                    "view",
                    "--output",
                    "json",
                    "--jq",
                    ".path_with_namespace",
                ],
                cancellation,
            )
            .await
            .context("GitLab CLI cannot read the configured GitLab project")?;
        let path = path.trim();
        if !valid_gitlab_project_path(path) {
            bail!("glab returned an invalid GitLab project identity");
        }
        let discovered = format!("gitlab.com/{}", path.to_ascii_lowercase());
        if discovered != expected_identity {
            bail!(
                "GitLab repository identity {discovered} does not match configured identity {expected_identity}"
            );
        }
        Ok(())
    }
}

impl GitLabForge {
    async fn run(
        &self,
        repository: Option<&Path>,
        arguments: &[&str],
        cancellation: &CancellationToken,
    ) -> Result<String> {
        let mut command = Command::new(&self.executable);
        command
            .args(arguments)
            .env_remove("GITLAB_HOST")
            .env_remove("GL_HOST")
            .env_remove("GLAB_REPO")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(repository) = repository {
            command.current_dir(repository);
        }
        let child = command.spawn().with_context(|| {
            format!(
                "failed to start GitLab CLI at {}",
                self.executable.display()
            )
        })?;
        let output = tokio::select! {
            _ = cancellation.cancelled() => bail!("GitLab CLI command cancelled"),
            result = tokio::time::timeout(self.command_timeout, child.wait_with_output()) => {
                match result {
                    Ok(output) => output.context("failed to wait for GitLab CLI")?,
                    Err(_) => bail!(
                        "glab command timed out after {}",
                        humantime::format_duration(self.command_timeout)
                    ),
                }
            }
        };
        if !output.status.success() {
            bail!("glab command failed with status {}", output.status);
        }
        String::from_utf8(output.stdout).context("glab output was not valid UTF-8")
    }
}

fn valid_gitlab_project_path(path: &str) -> bool {
    let segments = path.split('/').collect::<Vec<_>>();
    segments.len() >= 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && *segment != "-"
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
                })
        })
}

#[cfg(test)]
mod tests {
    use super::valid_gitlab_project_path;

    #[test]
    fn validates_gitlab_project_paths_without_accepting_urls_or_credentials() {
        assert!(valid_gitlab_project_path("group/subgroup/repository"));
        assert!(!valid_gitlab_project_path("group/repository/-/tree/main"));
        assert!(!valid_gitlab_project_path(
            "https://token@gitlab.com/group/repository"
        ));
        assert!(!valid_gitlab_project_path("group//repository"));
    }
}
