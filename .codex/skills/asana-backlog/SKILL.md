---
name: asana-backlog
description: Create one or more Asana backlog tasks with required project fields, an explicit manual or autonomous delivery policy, and verified native dependencies.
---

# Asana Backlog

Create tasks once the required routing, estimation, delivery, and dependency
details are known. Support one task or a batch. Do not show a preview unless the
user explicitly asks for one.

## Gather only what is missing

Extract task names and descriptions from the request. For every task, require:

- **Project**: resolve a named project and its exact `Backlog` section. Do not
  guess or create a missing section.
- **Priority**: High, Medium, or Low.
- **Story Points**: 1, 3, or 5.
- **Work Type**: bug, enhancement, or documentation.

Resolve the project custom-field and enum-option GIDs from live Asana metadata.
Never invent IDs. Ask one compact question containing only missing choices.
Do not ask about owner, dates, or extra metadata unless requested.

## Resolve delivery policy

Every invocation must resolve one of:

- `backlog_only`: select when the user asks to add, capture, or backlog work
  without authorizing implementation.
- `autonomous_to_pr`: select when the user explicitly asks to implement,
  deliver, or run autonomously to pull-request review.

When the request is unclear, ask once. If no answer is available, use the safe
default `backlog_only`. Never infer authorization merely because the tasks are
detailed.

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
logical batch. Write a temporary JSON manifest with this shape:

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

Run:

```sh
.flashy-factory/clients/asana batch-create --input /path/to/batch.json
```

The command owns the mutation sequence:

1. verify the OAuth token's app identity, remaining lifetime, and required
   scopes before project access;
2. resolve the exact existing `Backlog` section, selected authorization tag,
   and, for autonomous delivery, `Ready For Spec` section;
3. derive and preflight a deterministic external identity for every task,
   reserving a batch-level identity on the canonical first task, binding the
   canonical policy/task/dependency definition, and reusing only exact
   project/content matches;
4. create every missing task in `Backlog` and capture returned GIDs; if a create
   response is lost, use bounded, backoff-aware external-identity lookups
   without retrying the create request;
5. only when every task exists, add native Asana dependency edges;
6. read dependent tasks back and verify every requested edge;
7. remove the opposite authorization tag and apply the selected existing tag
   to every successfully verified connected component;
8. move verified autonomous components to `Ready For Spec` while leaving
   manual components in `Backlog`; and
9. read every component back and require exactly the selected authorization
   tag and expected section before reporting success.

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

Report created tasks, their GIDs, delivery policy, verified edges, exact
missing edges, component status, and failures. Do not claim the autonomous
batch is ready when the command returns nonzero.

## Polaris Internal Portal defaults

Use these facts only for the **Polaris Internal Portal** project; re-check them
if Asana returns different project metadata.

- Section: Backlog.
- Priority options: High, Medium, Low.
- Story Points: 1 = one to three days; 3 = four to seven days; 5 = more than
  one week.
- Work Type options: bug, enhancement, documentation.
