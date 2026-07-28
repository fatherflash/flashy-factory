use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::github::GitHubClient;
use crate::repository::{RepositoryProvider, RepositoryRef};

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

    async fn default_branch(
        &self,
        repository: &Path,
        cancellation: &CancellationToken,
    ) -> Result<String>;

    fn clone_repository(&self, identity: &str, destination: &Path, token: &str) -> Result<()>;

    fn git_credentials(&self) -> ForgeGitCredentials;

    async fn validate_worker_token_env(
        &self,
        token_env: &str,
        cancellation: &CancellationToken,
    ) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForgeGitCredentials {
    pub token_variable: &'static str,
    pub credential_key: &'static str,
    pub credential_helper: &'static str,
}

pub fn forge_for(provider: RepositoryProvider) -> Arc<dyn Forge> {
    forge_for_with_github(provider, GitHubClient::default())
}

pub(crate) fn forge_for_with_github(
    provider: RepositoryProvider,
    github: GitHubClient,
) -> Arc<dyn Forge> {
    match provider {
        RepositoryProvider::GitHub => Arc::new(GitHubForge {
            client: github,
            executable: PathBuf::from("gh"),
        }),
        RepositoryProvider::GitLab => Arc::new(GitLabForge::default()),
    }
}

#[cfg(test)]
pub(crate) fn forge_for_with_executable(
    provider: RepositoryProvider,
    executable: PathBuf,
) -> Arc<dyn Forge> {
    match provider {
        RepositoryProvider::GitHub => Arc::new(GitHubForge {
            client: GitHubClient::default(),
            executable,
        }),
        RepositoryProvider::GitLab => Arc::new(GitLabForge {
            executable,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }),
    }
}

struct GitHubForge {
    client: GitHubClient,
    executable: PathBuf,
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

    async fn default_branch(
        &self,
        repository: &Path,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        self.client
            .repository_default_branch(repository, cancellation)
            .await
            .map_err(|_| anyhow::anyhow!("GitHub CLI could not resolve the default branch"))
    }

    fn clone_repository(&self, identity: &str, destination: &Path, token: &str) -> Result<()> {
        let project = validate_identity(RepositoryProvider::GitHub, identity)?;
        let clone_url = format!("https://github.com/{project}.git");
        run_clone(
            &self.executable,
            &["repo", "clone"],
            &clone_url,
            destination,
            "GH_TOKEN",
            token,
        )
        .context("GitHub CLI could not clone the configured repository")
    }

    fn git_credentials(&self) -> ForgeGitCredentials {
        ForgeGitCredentials {
            token_variable: "GH_TOKEN",
            credential_key: "credential.https://github.com.helper",
            credential_helper: "!gh auth git-credential",
        }
    }

    async fn validate_worker_token_env(
        &self,
        token_env: &str,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        self.client
            .validate_token_env(token_env, cancellation)
            .await
            .map(|_| ())
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

    async fn default_branch(
        &self,
        repository: &Path,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        let branch = self
            .run(
                Some(repository),
                &[
                    "repo",
                    "view",
                    "--output",
                    "json",
                    "--jq",
                    ".default_branch",
                ],
                cancellation,
            )
            .await
            .context("GitLab CLI could not resolve the default branch")?;
        validate_branch("GitLab", branch.trim())
    }

    fn clone_repository(&self, identity: &str, destination: &Path, token: &str) -> Result<()> {
        let project = validate_identity(RepositoryProvider::GitLab, identity)?;
        let clone_url = format!("https://gitlab.com/{project}.git");
        run_clone(
            &self.executable,
            &["repo", "clone"],
            &clone_url,
            destination,
            "GITLAB_TOKEN",
            token,
        )
        .context("GitLab CLI could not clone the configured repository")
    }

    fn git_credentials(&self) -> ForgeGitCredentials {
        ForgeGitCredentials {
            token_variable: "GITLAB_TOKEN",
            credential_key: "credential.https://gitlab.com.helper",
            credential_helper: "!glab auth git-credential",
        }
    }

    async fn validate_worker_token_env(
        &self,
        token_env: &str,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let token = std::env::var(token_env)
            .with_context(|| format!("GitLab token environment {token_env:?} is missing"))?;
        if token.trim().is_empty() {
            bail!("GitLab token environment {token_env:?} is empty");
        }
        self.run_with_token(
            None,
            &["api", "user", "--hostname", "gitlab.com"],
            Some(&token),
            cancellation,
        )
        .await
        .map(|_| ())
        .with_context(|| format!("GitLab token environment {token_env:?} was rejected"))
    }
}

impl GitLabForge {
    async fn run(
        &self,
        repository: Option<&Path>,
        arguments: &[&str],
        cancellation: &CancellationToken,
    ) -> Result<String> {
        self.run_with_token(repository, arguments, None, cancellation)
            .await
    }

    async fn run_with_token(
        &self,
        repository: Option<&Path>,
        arguments: &[&str],
        token: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        let mut command = Command::new(&self.executable);
        command
            .args(arguments)
            .env_remove("GITLAB_HOST")
            .env_remove("GL_HOST")
            .env_remove("GLAB_REPO")
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(token) = token {
            command.env("GITLAB_TOKEN", token);
        }
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

fn validate_branch(provider: &str, branch: &str) -> Result<String> {
    if branch.is_empty()
        || branch.starts_with('-')
        || branch
            .chars()
            .any(|character| matches!(character, '\0' | '\n' | '\r'))
    {
        bail!("{provider} returned an invalid default branch");
    }
    Ok(branch.to_owned())
}

fn validate_identity(provider: RepositoryProvider, identity: &str) -> Result<&str> {
    let remote = match provider {
        RepositoryProvider::GitHub => format!("https://github.com/{identity}.git"),
        RepositoryProvider::GitLab => {
            let project = identity
                .strip_prefix("gitlab.com/")
                .context("invalid GitLab repository identity")?;
            format!("https://gitlab.com/{project}.git")
        }
    };
    let parsed = RepositoryRef::parse(&remote).context("invalid repository identity")?;
    if parsed.provider != provider || parsed.identity() != identity {
        bail!("repository identity does not match selected provider");
    }
    Ok(match provider {
        RepositoryProvider::GitHub => identity,
        RepositoryProvider::GitLab => identity
            .strip_prefix("gitlab.com/")
            .expect("validated GitLab identities are host-qualified"),
    })
}

fn run_clone(
    executable: impl AsRef<std::ffi::OsStr>,
    prefix: &[&str],
    project: &str,
    destination: &Path,
    token_variable: &str,
    token: &str,
) -> Result<()> {
    let status = ProcessCommand::new(executable)
        .args(prefix)
        .arg(project)
        .arg(destination)
        .args(["--", "--no-checkout", "--no-tags"])
        .env(token_variable, token)
        .env("GLAB_NO_PROMPT", "1")
        .env_remove("GITLAB_HOST")
        .env_remove("GL_HOST")
        .env_remove("GLAB_REPO")
        .env_remove(if token_variable == "GH_TOKEN" {
            "GITLAB_TOKEN"
        } else {
            "GH_TOKEN"
        })
        .env_remove(if token_variable == "GH_TOKEN" {
            "GLAB_TOKEN"
        } else {
            "GITHUB_TOKEN"
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to start repository CLI clone")?;
    if !status.success() {
        bail!("repository CLI clone failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn validates_gitlab_project_paths_without_accepting_urls_or_credentials() {
        assert!(valid_gitlab_project_path("group/subgroup/repository"));
        assert!(!valid_gitlab_project_path("group/repository/-/tree/main"));
        assert!(!valid_gitlab_project_path(
            "https://token@gitlab.com/group/repository"
        ));
        assert!(!valid_gitlab_project_path("group//repository"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn gitlab_default_branch_and_worker_token_use_glab_without_leaking_the_token() {
        let _environment = crate::TEST_ENV_LOCK.lock().await;
        let _gh = crate::TestEnvGuard::set("GH_TOKEN", "unrelated-github-secret");
        let _github = crate::TestEnvGuard::set("GITHUB_TOKEN", "unrelated-github-secret");
        let _token = crate::TestEnvGuard::set("FACTORY_FORGE_TEST_GITLAB_TOKEN", "worker-secret");
        let temp = tempfile::tempdir().unwrap();
        let glab = temp.path().join("glab");
        let log = temp.path().join("commands.log");
        fs::write(
            &glab,
            format!(
                r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> '{}'
if test "$1 $2" = 'repo view'; then
  test -z "${{GH_TOKEN:-}}"
  test -z "${{GITHUB_TOKEN:-}}"
  printf '%s\n' 'release/main'
  exit 0
fi
if test "$1 $2" = 'api user'; then
  test "$3 $4" = '--hostname gitlab.com'
  test "$GITLAB_TOKEN" = 'worker-secret'
  test -z "${{GH_TOKEN:-}}"
  test -z "${{GITHUB_TOKEN:-}}"
  exit 0
fi
exit 64
"#,
                log.display()
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&glab).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&glab, permissions).unwrap();
        let forge = forge_for_with_executable(RepositoryProvider::GitLab, glab);
        let cancellation = CancellationToken::new();

        assert_eq!(
            forge
                .default_branch(temp.path(), &cancellation)
                .await
                .unwrap(),
            "release/main"
        );
        forge
            .validate_worker_token_env("FACTORY_FORGE_TEST_GITLAB_TOKEN", &cancellation)
            .await
            .unwrap();

        let commands = fs::read_to_string(log).unwrap();
        assert!(commands.contains("repo view --output json --jq .default_branch"));
        assert!(commands.contains("api user --hostname gitlab.com"));
        assert!(!commands.contains("worker-secret"));
    }
}
