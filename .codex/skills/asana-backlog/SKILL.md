---
name: asana-backlog
description: Create one or more Asana backlog tasks only after collecting and applying the project, priority, story points, work type, and manual-or-autonomous authorization tag, with verified native dependencies.
---

# Asana Backlog

Create tasks only after the required routing, estimation, classification,
authorization, and dependency details are known. Support one task or a batch.
Do not show a preview unless the user explicitly asks for one.

## Gather only what is missing

Extract task names and descriptions from the request. For every task, require
these values before preparing a manifest or making an Asana mutation:

- **Project**: resolve a named project and its exact `Backlog` section. Do not
  guess or create a missing section.
- **Priority**: High, Medium, or Low.
- **Story Points**: 1, 3, or 5.
- **Work Type**: bug, enhancement, or documentation.
- **Authorization Tag**: `factory:manual` for manual work or
  `factory:auto-to-pr` for autonomous delivery to pull-request review.

The project and authorization tag may be shared by the batch. Priority, story
points, and work type may be shared only when the user explicitly applies the
same value to every task; otherwise collect them per task. Resolve the project,
Backlog section, project custom-field GIDs, enum-option GIDs, and tag GID from
live Asana metadata. Never invent IDs.

Ask one compact question containing only the missing required values. Do not
create a task with a missing value, substitute a default, or leave a required
field or tag unset. Do not ask about owner, dates, or extra metadata unless
requested.

## Resolve delivery policy

Every invocation must resolve the authorization tag to one of:

- `backlog_only`: select when the user asks to add, capture, or backlog work
  without authorizing implementation.
- `autonomous_to_pr`: select when the user explicitly asks to implement,
  deliver, or run autonomously to pull-request review.

When the request is unclear, ask once whether the work is manual or
autonomous. Do not default to `backlog_only`, and never infer authorization
merely because the tasks are detailed. Map the selected tag to the manifest
policy: `factory:manual` -> `backlog_only`; `factory:auto-to-pr` ->
`autonomous_to_pr`.

## Resolve dependencies

For a single task, use `independent`. For two or more tasks, resolve one of:

- `independent`: no task blocks another.
- `listed_chain`: each task after the first depends on the task immediately
  before it.
- `explicit_edges`: a custom graph whose edges use `dependent -> blocker`.

Infer explicit edges from unambiguous wording such as “after,” “requires,” or
“blocked by.” Otherwise ask one compact batch-level question. Do not predict
dependencies by inspecting repository code.

Before any Asana mutation, reject:

- more than 25 tasks;
- duplicate or unknown task references;
- self-dependencies, duplicate edges, or cycles; and
- a task with more than 30 dependencies and dependents combined.

## Create and verify the batch

Use the repository-owned `.flashy-factory/clients/asana batch-create` command.
Before invoking it, require a short-lived OAuth access token from the operator's
external token manager. The process environment must set `ASANA_AUTH_MODE` to
`oauth`, `ASANA_OAUTH_CLIENT_ID` to the app identity, and
`ASANA_OAUTH_ACCESS_TOKEN` to the token. Never request, accept, persist, or log
an OAuth client secret or refresh token. Do not place any credential in a
manifest or command argument.

Choose a stable, non-secret `batch_creation_id` that is unique across projects
for the configured Asana app, and reuse it unchanged for every retry of the
logical batch. Write a temporary JSON manifest only after every task has a
project-backed priority, story-points, and work-type assignment and the
selected authorization tag. Include the three custom-field assignments in each
task so the create operation applies them with the task's project membership:

```json
{
  "batch_creation_id": "2026-08-04-api-schema-v1",
  "delivery_policy": "autonomous_to_pr",
  "dependencies": {
    "mode": "explicit_edges",
    "edges": [
      {"dependent": "api", "blocker": "schema"}
    ]
  },
  "tasks": [
    {
      "ref": "schema",
      "name": "Add the schema",
      "notes": "Outcome and acceptance criteria",
      "custom_fields": {
        "priority-field-gid": "high-option-gid",
        "points-field-gid": "three-option-gid",
        "work-type-field-gid": "enhancement-option-gid"
      }
    },
    {
      "ref": "api",
      "name": "Use the schema",
      "custom_fields": {
        "priority-field-gid": "high-option-gid",
        "points-field-gid": "three-option-gid",
        "work-type-field-gid": "enhancement-option-gid"
      }
    }
  ]
}
```

For `independent` or `listed_chain`, the `dependencies` value may be that exact
string. Task references are local manifest identifiers, not predicted Asana
GIDs.

Before running the command, confirm its batch manifest supports setting and
reading back the three `custom_fields` values. If it does not, stop without
creating a task and report that the batch client must be updated; never create
a partially classified task. Run with the selected project explicitly:

```sh
.flashy-factory/clients/asana batch-create --project <selected-project-gid> --input /path/to/batch.json
```

The command owns the mutation sequence:

1. verify the OAuth token's app identity, remaining lifetime, and the available
   `tasks:read`, `tasks:write`, `projects:read`, `tags:read`, and
   `custom_fields:read` scopes before
   project access; do not request Full Permissions;
2. read the configured `ASANA_BACKLOG_SECTION_WITNESS_TASK_GID` and require an
   exact membership pairing `ASANA_PROJECT_GID` with
   `ASANA_BACKLOG_SECTION_GID`; for autonomous delivery do the same for the
   Ready For Spec pair. Missing, inaccessible, cross-project, or wrong-section
   witnesses fail before mutation. Do not use project-section discovery here;
3. resolve the selected authorization tag;
4. derive and preflight a deterministic external identity for every task,
   reserving a batch-level identity on the canonical first task, binding the
   canonical policy/task/dependency definition, and reusing only exact
   project/content matches;
5. create every missing task in the selected project's `Backlog`, with its
   priority, story-points, and work-type custom-field values, and capture
   returned GIDs; if a create response is lost, use bounded, backoff-aware
   external-identity lookups without retrying the create request;
6. only when every task exists, add native Asana dependency edges;
7. read dependent tasks back and verify every requested edge;
8. remove the opposite authorization tag and apply the selected existing tag
   to every successfully verified connected component;
9. move verified autonomous components to `Ready For Spec` while leaving
   manual components in `Backlog`; and
10. read every component back and require exactly the selected authorization
   tag, expected section, and requested priority, story-points, and work-type
   field values before reporting success.

The exact tags are `factory:auto-to-pr` and `factory:manual`. The command must
resolve them from the configured workspace and fail if the selected tag is
missing or duplicated. Never create an authorization tag.

## Failure behavior

Treat a nonzero command status as a partial or failed batch, even when stdout
contains a JSON report.

- If task creation is partial, create no dependency edge or autonomous
  authorization. Classify every created task as manual in `Backlog`, verify
  that fallback, delete nothing, and report every created GID and failed task.
- If an edge write or read-back fails, leave that whole connected component in
  `Backlog` with the manual tag. Other fully verified components may proceed.
- If authorization cannot be verified, downgrade the whole component to the
  manual tag in `Backlog` and verify that safe state. Report exact unsafe task
  GIDs when the downgrade cannot be confirmed.
- Report every requested edge that was not verified, using
  `dependent -> blocker`, plus returned task GIDs and operation failures.
- Never delete successfully created tasks to hide a partial result.
- Never retry an ambiguous task create. Look up its external identity and
  continue only after verifying its exact batch definition, task reference,
  project, Backlog membership, name, notes, and custom fields. If bounded
  lookup recovery is exhausted, report the exact unreconciled external
  identity.
- Reject reuse of a `batch_creation_id` when any external identity resolves to
  mismatched content, policy, dependency graph, task set, another project, or
  more than one total project membership.

Report created tasks, their GIDs, project, priority, story points, work type,
authorization tag, delivery policy, verified edges, exact missing edges,
component status, and failures. Do not claim the autonomous batch is ready
when the command returns nonzero.
