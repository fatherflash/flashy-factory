use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
use factory::config::Config;
use predicates::prelude::*;
use sha2::{Digest, Sha256};

fn pin_repository(path: &std::path::Path, provider: &str, identity: &str) {
    let contents = fs::read_to_string(path).unwrap();
    let contents = contents.replace(
        "poll_every = \"30s\"\n",
        &format!(
            "poll_every = \"30s\"\n\n[repository]\nprovider = {provider:?}\nidentity = {identity:?}\n"
        ),
    );
    fs::write(path, contents).unwrap();
}

fn create_workspace_for_identity(
    repository: &std::path::Path,
    data_home: &std::path::Path,
    identity: &str,
) {
    let mut hasher = Sha256::new();
    hasher.update(identity.as_bytes());
    hasher.update(b"\0");
    hasher.update(
        repository
            .canonicalize()
            .unwrap()
            .as_os_str()
            .as_encoded_bytes(),
    );
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::create_dir_all(data_home.join(&digest[..20]).join("worktrees")).unwrap();
}

fn valid_config() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repository");
    let data_home = temp.path().join("data");
    fs::create_dir_all(repository.join(".flashy-factory")).unwrap();
    assert!(
        ProcessCommand::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        ProcessCommand::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "git@github.com:example/repository.git"
            ])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    Command::cargo_bin("factory")
        .unwrap()
        .current_dir(&repository)
        .env("FACTORY_DATA_HOME", &data_home)
        .arg("init")
        .assert()
        .success();
    let path = repository.join(".flashy-factory/config.toml");
    fs::write(
        &path,
        r#"version = 1
poll_every = "30s"

[worker]
runtime = "codex"
sandbox = "worktree"
timeout = "2h"
maximum_timeout = "8h"
max_concurrent = 2

[source]
type = "github"
project_owner = "example"
project_number = 16
status_field = "Status"
trusted_users = ["example"]

[trigger.implement]
type = "label"
label = "agent:ready"
workflow = ".flashy-factory/workflows/implement.md"
"#,
    )
    .unwrap();
    (temp, path, repository, data_home)
}

#[cfg(unix)]
fn command_with_healthy_codex(temp: &tempfile::TempDir) -> Command {
    let bin = temp.path().join("healthy-bin");
    fs::create_dir_all(&bin).unwrap();
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'codex 1.0.0'; exit 0; fi\nif [ \"$1 $2\" = \"login status\" ]; then echo 'Logged in using ChatGPT'; exit 0; fi\nexit 64\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&codex).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(codex, permissions).unwrap();
    let gh = bin.join("gh");
    fs::write(
        &gh,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'gh version 2.80.0'; exit 0; fi\nif [ \"$1\" = \"auth\" ]; then exit 0; fi\nif [ \"$1\" = \"repo\" ]; then echo 'example/repository'; exit 0; fi\nif [ \"$1 $2\" = \"api user\" ]; then echo '{\"id\":2,\"login\":\"factory-bot\"}'; exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"users/example\" ]; then echo '{\"id\":1,\"login\":\"example\",\"node_id\":\"U_1\"}'; exit 0; fi\nexit 64\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&gh).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(gh, permissions).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::cargo_bin("factory").unwrap();
    command.env("PATH", path);
    command
}

#[cfg(unix)]
fn command_with_gitlab_forge(temp: &tempfile::TempDir, log: &std::path::Path) -> Command {
    let bin = temp.path().join("gitlab-bin");
    fs::create_dir_all(&bin).unwrap();
    for (name, contents) in [
        (
            "codex",
            "#!/bin/sh\nprintf 'codex %s\\n' \"$*\" >> \"$FACTORY_TEST_LOG\"\nif [ \"$1\" = \"--version\" ]; then echo 'codex 1.0.0'; exit 0; fi\nif [ \"$1 $2\" = \"login status\" ]; then echo 'Logged in using ChatGPT'; exit 0; fi\nexit 64\n",
        ),
        (
            "glab",
            "#!/bin/sh\nprintf 'glab %s\\n' \"$*\" >> \"$FACTORY_TEST_LOG\"\nif [ \"$1\" = \"--version\" ]; then echo 'glab 1.80.0'; exit 0; fi\nif [ \"$1 $2\" = \"auth status\" ]; then exit 0; fi\nif [ \"$1 $2\" = \"repo view\" ]; then echo 'example/subgroup/repository'; exit 0; fi\nexit 64\n",
        ),
        (
            "gh",
            "#!/bin/sh\nprintf 'gh %s\\n' \"$*\" >> \"$FACTORY_TEST_LOG\"\nif [ \"$1\" = \"--version\" ]; then echo 'gh version 2.80.0'; exit 0; fi\nif [ \"$1\" = \"auth\" ]; then exit 0; fi\nif [ \"$1\" = \"repo\" ]; then exit 99; fi\nif [ \"$1 $2\" = \"api user\" ]; then echo '{\"id\":2,\"login\":\"factory-bot\"}'; exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"users/example\" ]; then echo '{\"id\":1,\"login\":\"example\",\"node_id\":\"U_1\"}'; exit 0; fi\nexit 64\n",
        ),
    ] {
        let executable = bin.join(name);
        fs::write(&executable, contents).unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(executable, permissions).unwrap();
    }
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::cargo_bin("factory").unwrap();
    command.env("PATH", path).env("FACTORY_TEST_LOG", log);
    command
}

#[cfg(unix)]
#[test]
fn validates_explicit_config() {
    let (temp, path, _repository, data_home) = valid_config();

    command_with_healthy_codex(&temp)
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", data_home)
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration is valid."))
        .stdout(predicate::str::contains("worker.runtime: codex"));
}

#[cfg(unix)]
#[test]
fn validates_gitlab_repository_with_glab_and_keeps_github_source_separate() {
    let (temp, path, repository, data_home) = valid_config();
    assert!(
        ProcessCommand::new("git")
            .args([
                "remote",
                "set-url",
                "origin",
                "git@gitlab.com:example/subgroup/repository.git",
            ])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    pin_repository(&path, "gitlab", "gitlab.com/example/subgroup/repository");
    create_workspace_for_identity(
        &repository,
        &data_home,
        "gitlab.com/example/subgroup/repository",
    );
    let log = temp.path().join("commands.log");

    command_with_gitlab_forge(&temp, &log)
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", data_home)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "repository.identity: gitlab.com/example/subgroup/repository",
        ));

    let commands = fs::read_to_string(log).unwrap();
    assert!(commands.contains("glab auth status --hostname gitlab.com"));
    assert!(commands.contains("glab repo view --output json --jq .path_with_namespace"));
    assert!(commands.contains("gh api"));
    assert!(!commands.contains("gh repo"));
}

#[cfg(unix)]
#[test]
fn gitlab_run_stops_after_forge_validation_without_creating_a_ledger() {
    let (temp, path, repository, data_home) = valid_config();
    assert!(
        ProcessCommand::new("git")
            .args([
                "remote",
                "set-url",
                "origin",
                "git@gitlab.com:example/subgroup/repository.git",
            ])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    pin_repository(&path, "gitlab", "gitlab.com/example/subgroup/repository");
    create_workspace_for_identity(
        &repository,
        &data_home,
        "gitlab.com/example/subgroup/repository",
    );
    let log = temp.path().join("run-commands.log");

    command_with_gitlab_forge(&temp, &log)
        .current_dir(&repository)
        .args(["run", "--once", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", &data_home)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "GitLab repository validation succeeded, but task execution requires provider-aware workspace operations",
        ));

    let state_directory = fs::read_dir(&data_home)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|directory| directory.join("worktrees").exists())
        .unwrap();
    assert!(!state_directory.join("factory.sqlite3").exists());
    let commands = fs::read_to_string(log).unwrap();
    assert!(commands.contains("glab repo view"));
    assert!(!commands.contains("gh "));
}

#[cfg(unix)]
#[test]
fn gitlab_docker_validation_stops_before_github_worker_credentials() {
    let (temp, path, repository, data_home) = valid_config();
    assert!(
        ProcessCommand::new("git")
            .args([
                "remote",
                "set-url",
                "origin",
                "git@gitlab.com:example/subgroup/repository.git",
            ])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    pin_repository(&path, "gitlab", "gitlab.com/example/subgroup/repository");
    let contents = fs::read_to_string(&path)
        .unwrap()
        .replace(
            "type = \"github\"\nproject_owner = \"example\"\nproject_number = 16\nstatus_field = \"Status\"\ntrusted_users = [\"example\"]",
            "command = [\"./.flashy-factory/source.sh\"]",
        )
        .replace(
            "type = \"label\"\nlabel = \"agent:ready\"",
            "type = \"source\"\nstate = \"open\"\nlabels = [\"agent:ready\"]",
        )
        .replace("sandbox = \"worktree\"", "sandbox = \"docker_sandbox\"")
        .replace(
            "max_concurrent = 2",
            "max_concurrent = 2\ntemplate = \"docker/sandbox-templates:codex\"\nmemory = \"8g\"\ncpus = 4\ngithub_token_env = \"FACTORY_GITHUB_TOKEN\"",
        );
    fs::write(&path, contents).unwrap();
    let source = repository.join(".flashy-factory/source.sh");
    fs::write(&source, "#!/bin/sh\nprintf '%s\\n' '{\"issues\":[]}'\n").unwrap();
    let mut permissions = fs::metadata(&source).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(source, permissions).unwrap();
    create_workspace_for_identity(
        &repository,
        &data_home,
        "gitlab.com/example/subgroup/repository",
    );
    let log = temp.path().join("docker-commands.log");

    command_with_gitlab_forge(&temp, &log)
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", &data_home)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "GitLab Docker Sandbox credentials require provider-aware worker credentials",
        ));

    let commands = fs::read_to_string(log).unwrap();
    assert!(commands.contains("glab repo view"));
    assert!(!commands.contains("gh "));
}

#[test]
fn legacy_and_pinned_github_configs_use_the_same_durable_directory() {
    let (_temp, path, _repository, data_home) = valid_config();
    let legacy = Config::load_with_data_home(&path, &data_home).unwrap();

    pin_repository(&path, "github", "example/repository");
    let pinned = Config::load_with_data_home(&path, &data_home).unwrap();

    assert_eq!(legacy.repository, pinned.repository);
    assert_eq!(legacy.data_directory, pinned.data_directory);
}

#[test]
fn rejects_a_changed_repository_identity_before_selecting_durable_state() {
    let (_temp, path, _repository, data_home) = valid_config();
    pin_repository(&path, "github", "example/expected");
    let unused_data_home = data_home.with_file_name("unused-state");

    let error = Config::load_with_data_home(&path, &unused_data_home).unwrap_err();

    assert!(
        format!("{error:#}").contains(
            "configured repository github identity example/expected does not match origin github identity example/repository"
        )
    );
    assert!(!unused_data_home.exists());
}

#[test]
fn rejects_a_changed_repository_provider_before_selecting_durable_state() {
    let (_temp, path, _repository, data_home) = valid_config();
    pin_repository(&path, "gitlab", "gitlab.com/example/repository");
    let unused_data_home = data_home.with_file_name("unused-provider-state");

    let error = Config::load_with_data_home(&path, &unused_data_home).unwrap_err();

    assert!(
        format!("{error:#}").contains(
            "configured repository gitlab identity gitlab.com/example/repository does not match origin github identity example/repository"
        )
    );
    assert!(!unused_data_home.exists());
}

#[test]
fn loads_a_pinned_gitlab_identity_without_creating_durable_state() {
    let (_temp, path, repository, data_home) = valid_config();
    assert!(
        ProcessCommand::new("git")
            .args([
                "remote",
                "set-url",
                "origin",
                "git@gitlab.com:example/subgroup/repository.git",
            ])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    pin_repository(&path, "gitlab", "gitlab.com/example/subgroup/repository");
    create_workspace_for_identity(
        &repository,
        &data_home,
        "gitlab.com/example/subgroup/repository",
    );
    let config = Config::load_with_data_home(&path, &data_home).unwrap();

    assert_eq!(config.repository.provider.to_string(), "gitlab");
    assert_eq!(
        config.repository.identity,
        "gitlab.com/example/subgroup/repository"
    );
}

#[test]
fn rejects_a_linked_worktree_before_selecting_durable_state() {
    let (temp, _path, repository, data_home) = valid_config();
    assert!(
        ProcessCommand::new("git")
            .args(["add", "."])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        ProcessCommand::new("git")
            .args([
                "-c",
                "user.name=Factory Test",
                "-c",
                "user.email=factory@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    let linked = temp.path().join("linked");
    assert!(
        ProcessCommand::new("git")
            .args(["worktree", "add", "--quiet", "-b", "linked"])
            .arg(&linked)
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    let unused_data_home = data_home.with_file_name("unused-linked-state");

    let error = Config::load_with_data_home(
        &linked.join(".flashy-factory/config.toml"),
        &unused_data_home,
    )
    .unwrap_err();

    assert!(
        format!("{error:#}").contains(
            "Flashy Factory must run from the primary checkout, not a linked Git worktree"
        )
    );
    assert!(!unused_data_home.exists());
}

#[cfg(unix)]
#[test]
fn docker_sandbox_validation_requires_cli_credentials_and_host_clone_token() {
    let (temp, path, _repository, data_home) = valid_config();
    let contents = fs::read_to_string(&path)
        .unwrap()
        .replace("sandbox = \"worktree\"", "sandbox = \"docker_sandbox\"")
        .replace(
            "max_concurrent = 2",
            "max_concurrent = 2\ntemplate = \"docker/sandbox-templates:codex\"\nmemory = \"8g\"\ncpus = 4\ngithub_token_env = \"FACTORY_GITHUB_TOKEN\"",
        );
    fs::write(&path, contents).unwrap();
    let bin = temp.path().join("healthy-bin");
    fs::create_dir_all(&bin).unwrap();
    let sbx = bin.join("sbx");
    fs::write(
        &sbx,
        "#!/bin/sh\nif [ \"$1\" = version ]; then echo 'sbx version 0.35.0'; exit 0; fi\nif [ \"$1 $2 $3 $4\" = 'secret ls --global --service' ]; then echo \"global service $5\"; exit 0; fi\nexit 64\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&sbx).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(sbx, permissions).unwrap();

    command_with_healthy_codex(&temp)
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", data_home)
        .env("FACTORY_GITHUB_TOKEN", "dedicated-test-token")
        .assert()
        .success()
        .stdout(predicate::str::contains("worker.sandbox: docker_sandbox"))
        .stdout(predicate::str::contains(
            "worker.template: docker/sandbox-templates:codex",
        ));
}

#[cfg(unix)]
#[test]
fn worktree_validation_requires_a_healthy_host_codex_cli() {
    let (temp, path, _repository, data_home) = valid_config();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let codex = bin.join("codex");
    fs::write(&codex, "#!/bin/sh\necho broken codex >&2\nexit 64\n").unwrap();
    let mut permissions = fs::metadata(&codex).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex, permissions).unwrap();
    let gh = bin.join("gh");
    fs::write(
        &gh,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'gh version 2.80.0'; exit 0; fi\nif [ \"$1\" = \"auth\" ]; then exit 0; fi\nif [ \"$1\" = \"repo\" ]; then echo 'example/repository'; exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"users/example\" ]; then echo '{\"id\":1,\"login\":\"example\",\"node_id\":\"U_1\"}'; exit 0; fi\nexit 64\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&gh).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(gh, permissions).unwrap();
    let path_value = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    command_with_healthy_codex(&temp)
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", data_home)
        .env("PATH", path_value)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Codex CLI health check failed"));
}

#[cfg(unix)]
#[test]
fn rejects_an_existing_database_that_is_not_writable() {
    let (temp, path, _repository, data_home) = valid_config();
    command_with_healthy_codex(&temp)
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", &data_home)
        .assert()
        .success();
    let state_directory = fs::read_dir(&data_home)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let database = state_directory.join("factory.sqlite3");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE proof (id INTEGER);")
        .unwrap();
    let mut permissions = fs::metadata(&database).unwrap().permissions();
    permissions.set_mode(0o400);
    fs::set_permissions(&database, permissions).unwrap();

    command_with_healthy_codex(&temp)
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", &data_home)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Flashy Factory database is read-only",
        ));

    let mut permissions = fs::metadata(&database).unwrap().permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(database, permissions).unwrap();
}

#[test]
fn validates_a_configurable_source_label_trigger() {
    let (temp, path, repository, data_home) = valid_config();
    let contents =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/config.toml")).unwrap();
    fs::write(
        &path,
        contents
            .replace(
                "labels = [\"factory:ready-for-spec\"]",
                "labels = [\"factory:custom-stage\"]",
            )
            .replace(
                "command = [\".flashy-factory/sources/github\"]",
                "command = [\".flashy-factory/source.sh\"]",
            ),
    )
    .unwrap();
    let source = repository.join(".flashy-factory/source.sh");
    fs::write(&source, "#!/bin/sh\nprintf '%s\\n' '{\"issues\":[]}'\n").unwrap();
    let mut permissions = fs::metadata(&source).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&source, permissions).unwrap();
    fs::create_dir_all(repository.join(".flashy-factory/workflows")).unwrap();
    fs::write(
        repository.join(".flashy-factory/workflows/triage.md"),
        "Triage.\n",
    )
    .unwrap();
    fs::write(
        repository.join(".flashy-factory/workflows/implement.md"),
        "Implement.\n",
    )
    .unwrap();
    fs::write(
        repository.join(".flashy-factory/workflows/bug-finder.md"),
        "Find bugs.\n",
    )
    .unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let gh = bin.join("gh");
    fs::write(
        &gh,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "gh version 2.80.0"; exit 0; fi
if [ "$1" = "auth" ]; then exit 0; fi
if [ "$1" = "repo" ]; then echo "example/repository"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then echo '{"issues":[]}'; exit 0; fi
exit 64
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&gh).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh, permissions).unwrap();
    let codex = bin.join("codex");
    fs::write(
        &codex,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "codex 1.0.0"; exit 0; fi
if [ "$1" = "login" ] && [ "$2" = "status" ]; then echo "Logged in using ChatGPT"; exit 0; fi
exit 64
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&codex).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex, permissions).unwrap();
    let path_value = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    Command::cargo_bin("factory")
        .unwrap()
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", data_home)
        .env("PATH", path_value)
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration is valid."));
}

#[test]
fn rejects_an_empty_source_command() {
    let (_temp, path, _repository, data_home) = valid_config();
    let contents =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/config.toml")).unwrap();
    fs::write(
        &path,
        contents.replace(
            r#"command = [".flashy-factory/sources/github"]"#,
            "command = []",
        ),
    )
    .unwrap();

    Command::cargo_bin("factory")
        .unwrap()
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", data_home)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "source.command must contain an executable",
        ));
}

#[test]
fn reports_specific_validation_failures() {
    let (_temp, path, _repository, data_home) = valid_config();
    let contents = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        contents.replace("max_concurrent = 2", "max_concurrent = 0"),
    )
    .unwrap();

    Command::cargo_bin("factory")
        .unwrap()
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", data_home)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "worker.max_concurrent must be greater than zero",
        ));
}

#[cfg(unix)]
#[test]
fn uses_default_config_path() {
    let (temp, _path, repository, data_home) = valid_config();
    fs::create_dir_all(repository.join("nested/directory")).unwrap();

    command_with_healthy_codex(&temp)
        .arg("validate")
        .current_dir(repository.join("nested/directory"))
        .env("FACTORY_DATA_HOME", data_home)
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration is valid."));
}

#[cfg(unix)]
#[test]
fn resolves_relative_paths_from_config_directory() {
    let (temp, path, repository, data_home) = valid_config();
    let launch_dir = repository.join("nested");
    fs::create_dir(&launch_dir).unwrap();

    command_with_healthy_codex(&temp)
        .current_dir(launch_dir)
        .args(["validate", "--config", path.to_str().unwrap()])
        .env("FACTORY_DATA_HOME", &data_home)
        .assert()
        .success()
        .stdout(predicate::str::contains(repository.to_str().unwrap()))
        .stdout(predicate::str::contains(data_home.to_str().unwrap()));
}
