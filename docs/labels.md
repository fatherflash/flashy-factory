# Asana task classification and lifecycle

Flashy Factory uses three independent dimensions in the repository's Asana
project:

- the **section** records lifecycle and authorizes workflow runs;
- one **type tag** classifies the work; and
- the Asana **Priority** custom field ranks accepted work.

## Lifecycle sections

```text
Ready For Spec → Creating Spec → Awaiting Approval
                                  ↓ human dependency approval
Ready To Implement → Implementing → Reviewing → Done
         ↑
Approved - Waiting On Dependencies

Needs Decision
```

`Ready For Spec` and `Ready To Implement` are the normal polled sections.
Moving a task into either is an explicit request for an agent pass. The
checked-in configuration polls `Approved - Waiting On Dependencies` for
autonomous dependency reconciliation. A manual dependent remains there until a
human verifies its predecessor PR was merged and moves it to `Ready To
Implement`. Only trusted project members should be able to move tasks into the
two normal work sections.

The triage workflow moves a claimed task to `Creating Spec`, then every complete
specification goes to `Awaiting Approval`. A human reviews advisory dependency
suggestions and confirms, rejects, or corrects them. Only the confirmed native
links are applied: independent work goes to `Ready To Implement`, and a task
with an incomplete confirmed blocker goes to `Approved - Waiting On
Dependencies`. Ambiguous or unsafe work goes to `Needs Decision`. Sections—not
an Epic custom field—remain the visual source of truth. The implementation workflow
claims eligible work by moving it to `Implementing`, then moves a green
pull-request handoff to `Reviewing`. Humans remain responsible for merge and
`Done`.

While an autonomous task is waiting, its dedicated reconciliation trigger
re-reads the graph on every poll. Only a changed dependency graph or completion
revision creates another reconciliation run. A final completed blocker releases
the task to `Ready To Implement`; any remaining incomplete blocker leaves it
waiting. A manual dependent uses the explicit human post-merge release instead,
so it cannot be released solely because Asana marks a predecessor completed.

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

## Priority field

Every actionable task should have a value in Asana's `Priority` custom field.
Use the project's configured values, such as High, Medium, or Low. Priority
reflects impact and urgency, not implementation size. Factory workflows
preserve that field and do not add, remove, or require `P0`, `P1`, `P2`, or
`P3` tags.

The scheduled classifier validates type tags and applies only type-tag
classification changes. It does not create tags, edit task content, move
sections, complete tasks, or change the Priority field.

## Compatibility note

`.flashy-factory/tickets.toml` is retained from the upstream GitHub Project workflow
for existing installations and examples. The Asana workflows do not read it;
the Asana project sections and tags described here are the source of truth.
