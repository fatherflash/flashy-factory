# Use Asana as the source of truth

Flashy Factory can use an Asana project as its software-development control
plane. Project sections are workflow states, Asana tags are optional trigger
labels, and each task GID is the durable source identity.

The integration has two repository-owned layers:

- `.factory/clients/asana` is the authenticated API boundary used by agents to
  read, create, update, comment on, move, and tag tasks.
- `.factory/sources/asana` is the polling adapter. It delegates to the client
  and emits Flashy Factory's provider-neutral source JSON.

The Rust scheduler remains provider-neutral. It still owns polling, durable
claims, deduplication, reauthorization, isolation, and recovery.

## Prepare the Asana project

Create or choose one project for the repository. The checked-in workflows use
these exact, case-sensitive section names:

```text
Ready For Spec → Creating Spec → Awaiting Approval
                                  ↓ human approval
Ready To Implement → Implementing → Reviewing → Done
```

You can choose different names, but update `.factory/config.toml` and the
workflow prompts together. Only trusted project members should be able to move
tasks into `Ready For Spec` or `Ready To Implement`; those moves authorize an
agent run.

Find the project and workspace GIDs in Asana's URLs or through the API. GIDs are
identifiers, not secrets, but environment variables keep machine-specific
configuration out of the repository:

```sh
export ASANA_PROJECT_GID="..."
export ASANA_WORKSPACE_GID="..."
```

## Authenticate without committing a secret

For a personal setup, create a dedicated, revocable Asana personal access token
and expose it only to the Flashy Factory process:

```sh
export ASANA_ACCESS_TOKEN="..."
```

The client sends the token only in the `Authorization` header. It accepts only
Asana's official HTTPS API endpoint, refuses HTTP redirects, redacts the token
from diagnostics, and never stores it in configuration, task payloads, logs, or
command-line arguments. Do not put the token in `.env`,
`.factory/config.toml`, workflow files, task descriptions, or shell history.
Prefer an OS secret manager or service manager environment configuration for
long-running use.

The token inherits the permissions of its Asana user. Use a dedicated user with
access only to the intended workspace and project when practical. The current
worktree worker inherits host environment variables. A Docker worker needs an
explicit credential injection policy; this repository does not provide one.

## Configure polling

The repository is preconfigured with:

```toml
[source]
command = [".factory/sources/asana", "--max-results", "200"]

[trigger.triage]
type = "source"
state = "Ready For Spec"
workflow = ".factory/workflows/triage.md"

[trigger.implement]
type = "source"
state = "Ready To Implement"
workflow = ".factory/workflows/implement.md"
timeout = "4h"
```

`state` is an exact project section name. Each optional `label` is an exact
Asana tag name; all configured tags must be present. The adapter:

1. resolves exactly one matching section in `ASANA_PROJECT_GID`;
2. pages through its incomplete tasks with bounded results;
3. filters exact tag matches;
4. emits task name, notes, URL, creator, tags, and a stable revision;
5. fails instead of silently truncating work.

Flashy Factory reruns the same query immediately before execution. A task moved
out of the triggering section will not start.

## Agent client operations

Run these from the repository root. Substantial text uses files or standard
input so content is not mangled by shell quoting:

```sh
# Discover and read
.factory/clients/asana list --state "Ready To Implement"
.factory/clients/asana get TASK_GID

# Create and refine
.factory/clients/asana create \
  --name "Fix retry accounting" \
  --section "Ready For Spec" \
  --notes-file /tmp/task.md
.factory/clients/asana update TASK_GID --notes-file /tmp/task.md
.factory/clients/asana comment TASK_GID --text-file /tmp/comment.md

# Move and classify
.factory/clients/asana move TASK_GID --section "Reviewing"
.factory/clients/asana add-tag TASK_GID --tag "bug"
.factory/clients/asana remove-tag TASK_GID --tag "enhancement"
```

`get` returns the task plus up to 200 human comment stories. Every get, update,
comment, move, and tag operation first verifies that the task belongs to
`ASANA_PROJECT_GID`; a supplied GID cannot mutate other projects visible to the
token. `create` and `list` use `ASANA_PROJECT_GID` unless `--project` is
provided. Tag operations resolve existing tags in `ASANA_WORKSPACE_GID`.
Missing or duplicate section/tag names are hard failures; the client does not
guess.

## Validate and start

With the three environment variables available:

```sh
factory validate
factory workflows
factory run --once
factory run
```

`factory run --once` is the safest first live check: it discovers and records
eligible tasks but does not launch a worker.

The lowercase `factory` binary, Rust crate, `.factory` directory, `FACTORY_*`
environment variables, database filename, and existing branch/state formats
are deliberately retained for compatibility with the upstream project and
existing installations. “Flashy Factory” is the user-facing product name.
