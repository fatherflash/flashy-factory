# Use Asana as the source of truth

Flashy Factory can use an Asana project as its software-development control
plane. Project sections are workflow states, Asana tags are optional trigger
labels, and each task GID is the durable source identity.

The integration has two repository-owned layers:

- `.flashy-factory/clients/asana` is the authenticated API boundary used by agents to
  read, create, update, comment on, move, and tag tasks.
- `.flashy-factory/sources/asana` is the polling adapter. It delegates to the client
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

You can choose different names, but update `.flashy-factory/config.toml` and the
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
and expose it only to the Flashy Factory process. The personal token supports
polling and the single-task client commands; verified batch creation has a
separate OAuth requirement described below.

```sh
export ASANA_ACCESS_TOKEN="..."
```

The client sends the token only in the `Authorization` header. It accepts only
Asana's official HTTPS API endpoint, refuses HTTP redirects, redacts the token
from diagnostics, and never stores it in configuration, task payloads, logs, or
command-line arguments. Do not put the token in `.env`,
`.flashy-factory/config.toml`, legacy `.factory/config.toml`, workflow files,
task descriptions, or shell history. Prefer an OS secret manager or service
manager environment configuration for long-running use.

The token inherits the permissions of its Asana user. Use a dedicated user with
access only to the intended workspace and project when practical. The current
worktree worker inherits host environment variables. A Docker worker needs an
explicit credential injection policy; this repository does not provide one.

## Configure polling

The repository is preconfigured with:

```toml
[source]
command = [".flashy-factory/sources/asana", "--max-results", "200"]

[trigger.triage]
type = "source"
state = "Ready For Spec"
workflow = ".flashy-factory/workflows/triage.md"

[trigger.implement]
type = "source"
state = "Ready To Implement"
workflow = ".flashy-factory/workflows/implement.md"
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
.flashy-factory/clients/asana list --state "Ready To Implement"
.flashy-factory/clients/asana get TASK_GID

# Create and refine
.flashy-factory/clients/asana create \
  --name "Fix retry accounting" \
  --section "Ready For Spec" \
  --notes-file /tmp/task.md
.flashy-factory/clients/asana update TASK_GID --notes-file /tmp/task.md
.flashy-factory/clients/asana comment TASK_GID --text-file /tmp/comment.md

# Move and classify
.flashy-factory/clients/asana move TASK_GID --section "Reviewing"
.flashy-factory/clients/asana add-tag TASK_GID --tag "bug"
.flashy-factory/clients/asana remove-tag TASK_GID --tag "enhancement"
```

`get` returns the task plus up to 200 human comment stories. Every get, update,
comment, move, and tag operation first verifies that the task belongs to
`ASANA_PROJECT_GID`; a supplied GID cannot mutate other projects visible to the
token. `create` and `list` use `ASANA_PROJECT_GID` unless `--project` is
provided. Tag operations resolve existing tags in `ASANA_WORKSPACE_GID`.
Missing or duplicate section/tag names are hard failures; the client does not
guess.

## Create a verified batch

The repository includes an `asana-backlog` skill contract at
`.codex/skills/asana-backlog/SKILL.md`. Personal Codex installations do not
automatically update from a repository checkout: after merging a change to the
contract, deliberately sync that file to the installed `asana-backlog` skill
before using the new workflow.

The skill sends one validated JSON manifest to the client. This example creates
an autonomous API task that is blocked by a schema task:

```json
{
  "batch_creation_id": "2026-08-04-api-schema-v1",
  "delivery_policy": "autonomous_to_pr",
  "dependencies": {
    "mode": "explicit_edges",
    "edges": [{"dependent": "api", "blocker": "schema"}]
  },
  "tasks": [
    {"ref": "schema", "name": "Add the schema"},
    {"ref": "api", "name": "Use the schema"}
  ]
}
```

Pass the manifest by file or standard input:

```sh
.flashy-factory/clients/asana batch-create --input /tmp/asana-batch.json
```

`batch-create` fails closed unless it can verify an OAuth access token issued
to the configured Asana app. An external token manager must mint or refresh the
short-lived token before each run; the client deliberately does not accept or
store an OAuth client secret or refresh token. Configure it through the process
environment:

```sh
export ASANA_AUTH_MODE="oauth"
export ASANA_OAUTH_CLIENT_ID="..."
export ASANA_OAUTH_ACCESS_TOKEN="..."
export ASANA_BACKLOG_SECTION_GID="..."
export ASANA_BACKLOG_SECTION_WITNESS_TASK_GID="..."
export ASANA_READY_FOR_SPEC_SECTION_GID="..."
export ASANA_READY_FOR_SPEC_SECTION_WITNESS_TASK_GID="..."
```

The token must be active, belong to `ASANA_OAUTH_CLIENT_ID`, have at least five
minutes remaining, and include only `tasks:read`, `tasks:write`,
`projects:read`, and `tags:read`; never enable Asana Full Permissions. During
`batch-create`, configured section GIDs are not trusted alone. The client reads
each configured witness task and requires exactly one membership pairing the
configured project and section. `backlog_only` validates the Backlog pair;
`autonomous_to_pr` validates both pairs. A missing, inaccessible, wrong-project,
or wrong-section witness stops before any task creation. The client does not
call the project-section discovery endpoint in this command. Keep
the access token in an OS secret manager or service environment. Never put an
access token, refresh token, or client secret in the manifest, command line,
repository, task notes, logs, or shell history.

`batch_creation_id` is required and caller supplied. Choose a stable,
non-secret identifier that is unique across projects for this Asana app, and
reuse it unchanged when retrying the same logical batch. Changing task names,
notes, custom fields, project, task references, delivery policy, or the
canonical dependency graph while reusing an identifier is rejected.

`delivery_policy` is exactly `backlog_only` or `autonomous_to_pr`.
`dependencies` is `independent`, `listed_chain`, or an `explicit_edges` object;
an explicit edge is always `dependent -> blocker`. A listed chain makes each
task after the first depend on the task immediately before it.

The operation accepts at most 25 tasks. It rejects unknown task references,
self-dependencies, duplicate edges, cycles, and graphs over Asana's limit of 30
combined dependencies and dependents per task before creating anything.
It also rejects `custom_fields` in a batch manifest: verifying those values on
a retry would require `custom_fields:read`, which is outside this app's fixed
narrow OAuth scope set.

All tasks are created in `Backlog` before any edge is written. The client then
adds native dependencies, reads every dependent task back, and authorizes only
fully verified connected components. Manual components receive the existing
`factory:manual` tag and stay in `Backlog`. Autonomous components receive the
existing `factory:auto-to-pr` tag and move to `Ready For Spec`. The client
never creates a missing authorization tag. It resolves both exact tags, removes
the opposite tag, and reads each task back to verify exactly the selected tag
and expected section before reporting success.

Before creating anything, the client derives one deterministic Asana
`external.gid` for each task. The lexicographically first task reserves a fixed
batch-level identity keyed only by `batch_creation_id`; the remaining identities
are namespaced by task `ref`. The client then looks up every identity. This
canonical anchor prevents the same creation ID from being reused with a
disjoint or removed task set. An exact existing task in the expected project
and section is reused only when it has exactly one total project membership;
mismatched content, another project, or any additional project membership stops
the batch before task creation. If Asana accepts a create but the response is
lost or unusable, the
client performs bounded, backoff-aware lookups by that external identity and
continues only after verifying the exact batch definition, content, and Backlog
membership. It never retries the ambiguous create request, preventing a
duplicate on rerun. If reconciliation is exhausted, the nonzero JSON report
names the exact unreconciled external identity so an operator can inspect it.

A partial task-creation failure stops before dependency or autonomous
authorization writes and safely classifies every created task as manual in
`Backlog`. A partial dependency failure does the same for the affected
component; unrelated verified components may proceed. The command exits
nonzero and prints a JSON report containing created GIDs, failed tasks,
verified edges, exact missing edges, component states, and operation failures.
If authorization fails, the client attempts a verified downgrade to the manual
tag in `Backlog`; the report names exact unsafe task GIDs when that state cannot
be confirmed. It never deletes a successfully created task.

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

The lowercase `factory` binary and Rust crate, database filename, and existing
branch/state formats are deliberately retained for compatibility with the
upstream project and existing installations. New repository files live in
`.flashy-factory`; the previous `.factory` layout and `FACTORY_DATA_HOME`
environment variable remain supported compatibility aliases. “Flashy Factory”
is the user-facing product name.
