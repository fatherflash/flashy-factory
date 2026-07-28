# Asana task classification and lifecycle

Flashy Factory uses three independent dimensions in the repository's Asana
project:

- the **section** records lifecycle and authorizes workflow runs;
- one **type tag** classifies the work;
- one **priority tag** ranks accepted work.

## Lifecycle sections

```text
Ready For Spec → Creating Spec → Awaiting Approval
                                  ↓ human approval
Ready To Implement → Implementing → Reviewing → Done
```

`Ready For Spec` and `Ready To Implement` are the only polled sections in the
checked-in configuration. Moving a task into one of them is an explicit request
for an agent pass. Only trusted project members should have permission to make
those moves.

The triage workflow moves a claimed task to `Creating Spec`, then routes a
complete specification to `Awaiting Approval`. A human reviews the result and
moves it to `Ready To Implement`. The implementation workflow claims it by
moving it to `Implementing`, then moves a green pull-request handoff to
`Reviewing`. Humans remain responsible for merge and `Done`.

## Type tags

Every actionable task should have exactly one:

| Tag | Meaning |
| --- | --- |
| `bug` | Existing behavior is incorrect. |
| `enhancement` | New capability or improved behavior. |
| `documentation` | Documentation is the primary deliverable. |

Features are enhancements. Suspected vulnerabilities follow
[SECURITY.md](../SECURITY.md) and should not expose sensitive details through a
public tag or task.

## Priority tags

Every actionable task should have exactly one:

| Tag | Meaning |
| --- | --- |
| `P0` | Active incident, severe exposure, data loss, or broadly unusable product. |
| `P1` | Important correctness, security, or reliability work that should be next. |
| `P2` | Meaningful planned work. |
| `P3` | Valid low-impact or opportunistic work. |

Priority reflects impact and urgency, not implementation size. Leave it unset
for work that should be rejected.

The scheduled classifier validates existing tags, removes conflicting values,
and applies only classification changes. It does not create tags, edit task
content, move sections, or complete tasks.

## Compatibility note

`.factory/tickets.toml` is retained from the upstream GitHub Project workflow
for existing installations and examples. The Asana workflows do not read it;
the Asana project sections and tags described here are the source of truth.
