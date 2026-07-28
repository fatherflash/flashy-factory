use std::fmt;

use anyhow::{Result, bail};

const SUPPORTED_REMOTE_HELP: &str = "supported repository remotes are \
git@github.com:owner/repository.git, https://github.com/owner/repository.git, \
ssh://git@github.com/owner/repository.git, \
git@gitlab.com:group/repository.git, https://gitlab.com/group/repository.git, or \
ssh://git@gitlab.com/group/repository.git";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryProvider {
    GitHub,
    GitLab,
}

impl fmt::Display for RepositoryProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRef {
    pub provider: RepositoryProvider,
    pub host: String,
    pub namespace: String,
    pub name: String,
}

impl RepositoryRef {
    pub fn parse(remote: &str) -> Result<Self> {
        parse_repository_ref(remote)
    }

    pub fn identity(&self) -> String {
        match self.provider {
            RepositoryProvider::GitHub => {
                format!("{}/{}", self.namespace, self.name).to_ascii_lowercase()
            }
            RepositoryProvider::GitLab => {
                format!("{}/{}/{}", self.host, self.namespace, self.name).to_ascii_lowercase()
            }
        }
    }
}

pub fn parse_repository_ref(remote: &str) -> Result<RepositoryRef> {
    if remote.is_empty() || remote.contains(['?', '#']) {
        return malformed_remote();
    }

    let (transport_host, path) = if let Some(remainder) = remote.strip_prefix("https://") {
        parse_url_remote(remainder, None)?
    } else if let Some(remainder) = remote.strip_prefix("ssh://") {
        parse_url_remote(remainder, Some("git"))?
    } else {
        parse_scp_remote(remote)?
    };

    let (provider, host) = match transport_host.to_ascii_lowercase().as_str() {
        "github.com" => (RepositoryProvider::GitHub, "github.com"),
        "ssh.github.com" => (RepositoryProvider::GitHub, "github.com"),
        "gitlab.com" => (RepositoryProvider::GitLab, "gitlab.com"),
        _ => return unsupported_host(transport_host),
    };

    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.iter().any(|segment| {
        segment.is_empty() || *segment == "." || *segment == ".." || segment.ends_with(".git")
    }) {
        return malformed_remote();
    }
    if provider == RepositoryProvider::GitLab && segments.contains(&"-") {
        return malformed_remote();
    }

    let minimum_segments = 2;
    if segments.len() < minimum_segments
        || (provider == RepositoryProvider::GitHub && segments.len() != minimum_segments)
    {
        return malformed_remote();
    }

    let (name, namespace) = segments
        .split_last()
        .expect("repository paths have at least two segments");
    Ok(RepositoryRef {
        provider,
        host: host.to_owned(),
        namespace: namespace.join("/"),
        name: (*name).to_owned(),
    })
}

fn parse_url_remote<'a>(
    remote: &'a str,
    required_user: Option<&str>,
) -> Result<(&'a str, &'a str)> {
    let (authority, path) = remote.split_once('/').ok_or_else(malformed_error)?;
    if authority.is_empty() || path.is_empty() {
        return malformed_remote();
    }

    let (user_info, host_and_port) = authority
        .rsplit_once('@')
        .map_or((None, authority), |(user, host)| (Some(user), host));
    if required_user.is_some_and(|required| user_info != Some(required)) {
        return malformed_remote();
    }

    let (host, port) = host_and_port
        .rsplit_once(':')
        .map_or((host_and_port, None), |(host, port)| (host, Some(port)));
    if host.is_empty()
        || port.is_some_and(|port| {
            port.is_empty() || !port.chars().all(|character| character.is_ascii_digit())
        })
    {
        return malformed_remote();
    }
    if host.eq_ignore_ascii_case("ssh.github.com") && port.is_some_and(|port| port != "443") {
        return unsupported_host(host);
    }
    if required_user.is_none() && host.eq_ignore_ascii_case("ssh.github.com") {
        return unsupported_host(host);
    }

    Ok((host, path))
}

fn parse_scp_remote(remote: &str) -> Result<(&str, &str)> {
    let (authority, path) = remote.split_once(':').ok_or_else(malformed_error)?;
    let (user, host) = authority.split_once('@').ok_or_else(malformed_error)?;
    if user != "git" || host.is_empty() || path.is_empty() {
        return malformed_remote();
    }
    if host.eq_ignore_ascii_case("ssh.github.com") {
        return unsupported_host(host);
    }
    Ok((host, path))
}

fn malformed_error() -> anyhow::Error {
    anyhow::anyhow!("invalid repository remote; {SUPPORTED_REMOTE_HELP}")
}

fn malformed_remote<T>() -> Result<T> {
    Err(malformed_error())
}

fn unsupported_host<T>(host: &str) -> Result<T> {
    let sanitized_host = host
        .rsplit_once('@')
        .map_or(host, |(_, sanitized)| sanitized);
    bail!(
        "unsupported repository host {sanitized_host:?}; supported hosts are github.com and gitlab.com; {SUPPORTED_REMOTE_HELP}"
    )
}

#[cfg(test)]
mod tests {
    use super::{RepositoryProvider, RepositoryRef};

    #[test]
    fn parses_supported_github_remotes_without_changing_identity() {
        for remote in [
            "git@github.com:Owner/Repository.git",
            "https://token@GitHub.COM:443/Owner/Repository.git",
            "ssh://git@github.com:22/Owner/Repository.git",
            "ssh://git@ssh.github.com:443/Owner/Repository.git",
        ] {
            let repository = RepositoryRef::parse(remote).unwrap();
            assert_eq!(repository.provider, RepositoryProvider::GitHub);
            assert_eq!(repository.host, "github.com");
            assert_eq!(repository.namespace, "Owner");
            assert_eq!(repository.name, "Repository");
            assert_eq!(repository.identity(), "owner/repository");
        }
    }

    #[test]
    fn parses_gitlab_subgroups_and_host_qualified_identity() {
        for remote in [
            "git@gitlab.com:DuskLabs/Carini/Polaris.git",
            "https://GitLab.COM/DuskLabs/Carini/Polaris.git",
            "ssh://git@gitlab.com/DuskLabs/Carini/Polaris.git",
        ] {
            let repository = RepositoryRef::parse(remote).unwrap();
            assert_eq!(repository.provider, RepositoryProvider::GitLab);
            assert_eq!(repository.host, "gitlab.com");
            assert_eq!(repository.namespace, "DuskLabs/Carini");
            assert_eq!(repository.name, "Polaris");
            assert_eq!(repository.identity(), "gitlab.com/dusklabs/carini/polaris");
        }
    }

    #[test]
    fn rejects_malformed_or_ambiguous_remotes() {
        for remote in [
            "",
            "git@github.com:owner.git",
            "git@github.com:owner/repository/extra.git",
            "git@gitlab.com:repository.git",
            "git@gitlab.com:group//repository.git",
            "https://github.com/owner/repository.git?token=secret",
            "https://gitlab.com/group/repository.git#readme",
            "https://gitlab.com/group/repository/-/tree/main",
            "https://gitlab.com/group/repository/-/merge_requests/42",
            "ssh://root@gitlab.com/group/repository.git",
            "ssh://git@ssh.github.com:22/owner/repository.git",
        ] {
            assert!(RepositoryRef::parse(remote).is_err(), "{remote}");
        }
    }

    #[test]
    fn unsupported_host_errors_are_sanitized_and_actionable() {
        let error = RepositoryRef::parse("https://secret@example.github.com/owner/repository.git")
            .unwrap_err()
            .to_string();
        assert!(error.contains("example.github.com"));
        assert!(error.contains("github.com and gitlab.com"));
        assert!(!error.contains("secret"));
    }
}
