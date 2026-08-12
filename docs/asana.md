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
Backlog
Ready For Spec → Creating Spec → Awaiting Approval
                                  ↓ human dependency approval
Ready To Implement → Implementing → Reviewing → Done
         ↑
Approved - Waiting On Dependencies

Needs Decision
```

You can choose different names, but update `.flashy-factory/config.toml` and the
workflow prompts together. Only trusted project members should be able to move
tasks into `Ready For Spec` or `Ready To Implement`; those moves authorize a
normal agent run. `Approved - Waiting On Dependencies` is additionally polled
only for autonomous dependency reconciliation.

Find the project and workspace GIDs in Asana's URLs or through the API. GIDs are
identifiers, not secrets, but environment variables keep machine-specific
configuration out of the repository:

```sh
export ASANA_PROJECT_GID="..."
export ASANA_WORKSPACE_GID="..."
```

### Create the delivery tags and custom fields

Create these existing workspace tags exactly once and add them only as an
explicit delivery authorization:

```text
factory:manual
factory:auto-to-pr
```

The verified backlog workflow also requires these project custom fields and
enum options:

| Field | Options |
| --- | --- |
| Priority | High, Medium, Low |
| Story Points | 1, 3, 5 |
| Work Type | bug, enhancement, documentation |

Names are human-readable classification. The batch manifest uses each custom
field GID mapped to its selected enum-option GID, so record both kinds of GID.
Do not replace the `factory:*` authorization tags with a custom field: triggers
use exact tag names, while classification uses the custom fields.

### Create section witness tasks

Create one harmless, incomplete, untagged witness task in `Backlog` and another
in `Ready For Spec`. Keep each task in exactly one project and do not move or
complete it. Give them names that clearly state their operational purpose.

Verified batch creation reads these tasks before writing anything. The
`backlog_only` policy proves the configured Backlog GID belongs to the expected
project. The `autonomous_to_pr` policy proves both Backlog and Ready For Spec.
This prevents a copied or mistyped section GID from routing a batch into another
project visible to the OAuth identity.

### Collect and verify the non-secret GIDs

Record this per-project inventory outside the repository's committed files:

```text
ASANA_WORKSPACE_GID
ASANA_PROJECT_GID
ASANA_BACKLOG_SECTION_GID
ASANA_BACKLOG_SECTION_WITNESS_TASK_GID
ASANA_READY_FOR_SPEC_SECTION_GID
ASANA_READY_FOR_SPEC_SECTION_WITNESS_TASK_GID
Priority field GID and High/Medium/Low option GIDs
Story Points field GID and 1/3/5 option GIDs
Work Type field GID and bug/enhancement/documentation option GIDs
factory:manual tag GID
factory:auto-to-pr tag GID
```

Asana URLs expose useful object GIDs, but a board URL can contain workspace,
project, and view identifiers together. Do not identify a value only by its
position in the URL. Confirm it in Asana's UI or with authenticated reads from
the official API:

```text
GET /api/1.0/projects/{project_gid}?opt_fields=workspace.gid
GET /api/1.0/projects/{project_gid}/sections
GET /api/1.0/workspaces/{workspace_gid}/tags
GET /api/1.0/tasks/{witness_task_gid}
GET /api/1.0/projects/{project_gid}/custom_field_settings?opt_fields=custom_field.gid,custom_field.name,custom_field.enum_options.gid,custom_field.enum_options.name
```

When reading a witness task, verify its membership pairs the expected project
and section. When reading project custom-field settings, record the custom
field and enum-option GIDs rather than their display order. GIDs identify
resources but are not authentication secrets.

## Authenticate without committing a secret

For a personal setup, create a dedicated, revocable Asana personal access token
and expose it only to the Flashy Factory process. The personal token supports
polling and the single-task client commands; verified batch creation has a
separate OAuth requirement described below.

```bash
read -rsp "Asana PAT: " ASANA_ACCESS_TOKEN
printf '\n'
export ASANA_ACCESS_TOKEN
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
Do not request Asana Full Permissions: a dedicated user with access to this
project and its tasks is sufficient for the supported client operations.

### Use a PAT for the long-running daemon

The polling daemon and normal single-task client operations use
`ASANA_ACCESS_TOKEN`. Create a dedicated, revocable PAT from the Asana developer
console. The PAT inherits everything its Asana user can access; it is not
project-scoped by the environment variables. Prefer an automation user whose
workspace and project access is no broader than necessary.

Unset it after interactive validation. For continuous operation, use the
encrypted systemd credential pattern in the
[operations guide](operations.md#persistent-linux-service-with-systemd) instead
of a shell export.

### Use narrow OAuth for verified backlog batches

`batch-create` intentionally rejects PAT authentication. Create an Asana OAuth
developer app and disable Full Permissions. Allow exactly these scopes:

```text
tasks:read
tasks:write
projects:read
tags:read
custom_fields:read
```

Register a redirect URI controlled by your authorization helper. Complete
Asana's [authorization-code flow with
PKCE](https://developers.asana.com/docs/oauth), then keep the client secret and
long-lived refresh token in a secret manager. Asana access tokens normally last
one hour, so a local helper or token broker must exchange the refresh token for
a new access token before expiry and expose only that short-lived token to the
interactive Codex process.

The Factory daemon does not need the OAuth client secret, OAuth refresh token,
or OAuth access token. The backlog client does not accept or persist the client
secret or refresh token. Keeping these two credential paths separate limits
which long-running process can read each secret.

Before task creation, the environment used to launch Codex must contain:

```sh
export ASANA_AUTH_MODE="oauth"
export ASANA_OAUTH_CLIENT_ID="<oauth-client-id>"
ASANA_OAUTH_ACCESS_TOKEN="$(tr -d '\r\n' </path/to/private/runtime-access-token)"
export ASANA_OAUTH_ACCESS_TOKEN
export ASANA_PROJECT_GID="<project-gid>"
export ASANA_WORKSPACE_GID="<workspace-gid>"
export ASANA_BACKLOG_SECTION_GID="<backlog-section-gid>"
export ASANA_BACKLOG_SECTION_WITNESS_TASK_GID="<backlog-witness-task-gid>"
export ASANA_READY_FOR_SPEC_SECTION_GID="<ready-section-gid>"
export ASANA_READY_FOR_SPEC_SECTION_WITNESS_TASK_GID="<ready-witness-task-gid>"
```

Do not type real secret values into a command saved in shell history. A
project-specific launcher may read the refreshed access token from a `0600`
runtime file, export these non-secret IDs, change to the repository, and start
Codex. Use a different launcher or configuration file for each Asana project.

## Configure polling

The repository is preconfigured with:

```toml
pull_request_reconcile_every = "60s"

[source]
command = [".flashy-factory/sources/asana", "--max-results", "200"]

[trigger.triage]
type = "source"
state = "Ready For Spec"
labels = ["factory:auto-to-pr"]
workflow = ".flashy-factory/workflows/triage.md"

[trigger.triage-manual]
type = "source"
state = "Ready For Spec"
labels = ["factory:manual"]
workflow = ".flashy-factory/workflows/triage.md"

[trigger.implement]
type = "source"
state = "Ready To Implement"
labels = ["factory:auto-to-pr"]
workflow = ".flashy-factory/workflows/implement.md"
timeout = "4h"

[trigger.reconcile-dependencies]
type = "source"
state = "Approved - Waiting On Dependencies"
labels = ["factory:auto-to-pr"]
workflow = ".flashy-factory/workflows/reconcile-dependencies.md"

[trigger.implement-manual]
type = "source"
state = "Ready To Implement"
labels = ["factory:manual"]
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

`Approved - Waiting On Dependencies` has its own autonomous-only source trigger.
Each poll re-reads the live native graph. The stable observation
revision includes the direct dependency GIDs and completion states, so a graph
or completion change creates one new reconciliation pass without restarting the
daemon; an unchanged waiting observation does not repeatedly enqueue work. The
reconciliation workflow moves only a fully unblocked autonomous task to `Ready
To Implement`, leaves a blocked task waiting, and sends an unsafe graph to
`Needs Decision`. A manual dependent remains waiting until a human verifies the
predecessor PR was human-merged and explicitly moves it to `Ready To Implement`.

## Review inferred dependencies during specification approval

When a specification is ready, the triage workflow reads incomplete planned and
active project tasks and comments with advisory dependency candidates. Each
candidate includes a concise rationale and a low, medium, or high confidence.
It is never a native edge by itself. The human reviewer confirms, rejects, or
corrects every suggestion while the task remains in `Awaiting Approval`.

To approve the specification, create an approval file containing only the
blocker GIDs the human confirmed (an empty list explicitly confirms independent
work), then run:

```json
{"confirmed_dependencies":["blocker-gid"]}
```

```sh
.flashy-factory/clients/asana apply-spec-approval TASK_GID --input /tmp/asana-approval.json
```

The command checks the live task, requires the approval section and exactly one
delivery-policy tag, validates each blocker belongs to the project, writes only
the confirmed native links, and then routes the task. An independent task is
sent directly to `Ready To Implement`; an autonomous task still blocked by a
confirmed predecessor is sent to `Approved - Waiting On Dependencies` until
the predecessor's PR is human-merged.

## Trusted pull-request reconciliation

An implementation handoff records a link only when the runtime observes a
canonical GitHub pull-request URL for the configured repository. This applies
to both `factory:auto-to-pr` and `factory:manual` tasks.
Comments and task descriptions are never association inputs. Every 60 seconds,
the daemon observes those linked PRs with authenticated `gh`; this polling does
not occupy an implementation worker slot. It records a durable fingerprint of
the PR state, head SHA, and reviews, so an unchanged response is consumed once
across polls and daemon restarts.

For a merged PR, the daemon re-reads Asana, moves the linked task to `Done`,
and completes it. For an autonomous PR, it also evaluates direct native
dependents in the same pass.
An eligible autonomous dependent moves to `Ready To Implement`. A closed but
unmerged PR moves only its linked task to `Needs Decision`. Missing,
cross-project, contradictory, inaccessible, or otherwise unsafe observations
fail closed to `Needs Decision` for autonomous work. For manual work, only a
merged PR changes the task; other observations remain untouched. The
released dependent is reauthorized before execution, then its managed worktree
is created from a fresh fetch of the default branch, including the predecessor's
human-merged change rather than the pre-merge base.

Operators need both the scoped Asana token above and an authenticated `gh`
session permitted to read the repository's pull requests. This path never
merges a PR, requests auto-merge, or enables auto-merge.

For autonomous tasks, the source query additionally fetches the native Asana
dependency graph. It persists the direct dependency GIDs and a stable revision
derived from sorted GIDs and their live completion states in the durable task
context. The implementation workflow reads the graph again just before its
claim. A changed revision or incomplete blocker returns the task to
`Approved - Waiting On Dependencies`; malformed, inaccessible, cross-project,
cyclic, or otherwise unresolved graphs go to `Needs Decision`. Sections remain
the visible workflow state; do not replace them with an Epic custom field.

## Agent client operations

Run these from the repository root. Substantial text uses files or standard
input so content is not mangled by shell quoting:

```sh
# Discover and read
.flashy-factory/clients/asana list --state "Ready To Implement"
.flashy-factory/clients/asana get TASK_GID
.flashy-factory/clients/asana dependency-state TASK_GID
.flashy-factory/clients/asana dependency-review TASK_GID
.flashy-factory/clients/asana apply-spec-approval TASK_GID --input /tmp/asana-approval.json

# Create and refine
.flashy-factory/clients/asana create \
  --name "Fix retry accounting" \
  --section "Ready For Spec" \
  --notes-file /tmp/task.md
.flashy-factory/clients/asana update TASK_GID --notes-file /tmp/task.md
.flashy-factory/clients/asana comment TASK_GID --text-file /tmp/comment.md

# Move a task
.flashy-factory/clients/asana move TASK_GID --section "Reviewing"
```

`get` returns the task plus up to 200 human comment stories. Every get, update,
comment, move, and tag operation first verifies that the task belongs to
`ASANA_PROJECT_GID`; a supplied GID cannot mutate other projects visible to the
token. `create` and `list` use `ASANA_PROJECT_GID` unless `--project` is
provided. Tag operations resolve existing tags in `ASANA_WORKSPACE_GID`.
Missing or duplicate section/tag names are hard failures; the client does not
guess.

`factory:manual` and `factory:auto-to-pr` are the only tags Factory uses for
workflow authorization. Work classification belongs in the existing Asana
`Work Type` custom field; Factory workflows preserve it and never require or
manage `bug`, `enhancement`, or `documentation` tags.

## Upgrade from legacy type-tag classification

Older Factory checkouts may contain a scheduled `classify-tickets` workflow
that manages `bug`, `enhancement`, and `documentation` tags. Factory preserves
existing repository configuration and workflows during `factory init`, so it
does not remove that legacy scheduler automatically. To adopt custom-field
classification, an operator should remove the `[trigger.classify-tickets]`
block from `.flashy-factory/config.toml` and remove
`.flashy-factory/workflows/classify-tickets.md`, then run `factory validate`.
This is an opt-in configuration migration: do not delete the task's existing
Asana `Work Type` field or its `factory:*` authorization tag.

`dependency-state` follows native dependency edges, verifies that every task is
accessible and belongs only to `ASANA_PROJECT_GID`, rejects malformed or cyclic
graphs, and returns `dependencies`, `blocked`, and `dependency_revision`.
Treat any failure as a decision boundary, not permission to implement.

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
    {
      "ref": "schema",
      "name": "Add the schema",
      "custom_fields": {
        "<priority-field-gid>": "<medium-option-gid>",
        "<story-points-field-gid>": "<three-points-option-gid>",
        "<work-type-field-gid>": "<enhancement-option-gid>"
      }
    },
    {
      "ref": "api",
      "name": "Use the schema",
      "custom_fields": {
        "<priority-field-gid>": "<medium-option-gid>",
        "<story-points-field-gid>": "<three-points-option-gid>",
        "<work-type-field-gid>": "<enhancement-option-gid>"
      }
    }
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
ASANA_OAUTH_ACCESS_TOKEN="$(tr -d '\r\n' </path/to/private/runtime-access-token)"
export ASANA_OAUTH_ACCESS_TOKEN
export ASANA_BACKLOG_SECTION_GID="..."
export ASANA_BACKLOG_SECTION_WITNESS_TASK_GID="..."
export ASANA_READY_FOR_SPEC_SECTION_GID="..."
export ASANA_READY_FOR_SPEC_SECTION_WITNESS_TASK_GID="..."
```

The token must be active, belong to `ASANA_OAUTH_CLIENT_ID`, have at least five
minutes remaining, and include only `tasks:read`, `tasks:write`,
`projects:read`, `tags:read`, and `custom_fields:read`; never enable Asana Full
Permissions. During
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
The low-level client permits each task to include a `custom_fields` object, but
the `asana-backlog` skill requires Priority, Story Points, and Work Type for
every task. Map their project custom-field GIDs to enum-option GIDs as shown
above. Those assignments are included in deterministic retry
identity, written during task creation, and read back before a component can be
authorized. This requires the narrow `custom_fields:read` scope; it does not
require Full Permissions.

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
