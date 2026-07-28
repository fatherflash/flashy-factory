# Flashy Factory

[![CI](https://github.com/fatherflash/flashy-factory/actions/workflows/ci.yml/badge.svg)](https://github.com/fatherflash/flashy-factory/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-2f6feb.svg)](LICENSE)

Flashy Factory keeps coding agents working on a repository without making a
human orchestrate every step from a terminal.

It watches a trusted Asana task queue. When a configured condition matches,
Flashy Factory creates a durable local task, prepares an isolated workspace,
and gives one Markdown workflow to an agent. The agent uses the repository's
Asana client plus normal tools such as `gh` and `git`. When nothing matches,
Flashy Factory does nothing and spends no model tokens.

![Human intent enters a trusted ticket queue; Flashy Factory runs isolated work and produces evidence; human review gates the team's merge; shipped changes return signals to the queue](docs/assets/readme/factory-loop.svg)

## Why Flashy Factory exists

Coding agents can implement increasingly substantial changes, but most teams
still operate them as one-off terminal sessions. Every developer uses different
prompts, skills, checks, and handoff conventions. Humans remain responsible for
noticing ready work, starting an agent, waiting for CI, forwarding review
feedback, and remembering to try again.

Flashy Factory makes this process repeatable. It plays a similar role to CI/CD:
work enters a consistent system, receives the same checks and feedback loops,
and keeps moving until it reaches a human decision.

The goal is not to replace developers. Humans decide what matters, supply
product context, review the result, and remain accountable for what ships.
Flashy Factory removes the manual coordination between those decisions.

## The ticket is the control plane

The issue tracker is where humans and agents coordinate. A ticket records the
problem, scope, acceptance criteria, decisions, status, and evidence. Moving a
ticket into a configured state is an explicit request for an agent pass.

This makes ticket quality load-bearing. A vague ticket is not ready for either a
human or an agent. A triage workflow can inspect the codebase, reproduce the
problem, clarify scope, add testable acceptance criteria, and ask for the
smallest missing human decision. Once the ticket is clear, it becomes the spec
for implementation.

![An example ticket moves through specification, human approval, implementation, and human review, with feedback requesting another implementation pass](docs/assets/readme/ticket-workflow.svg)

The status names in this example are not built into Flashy Factory. In this
fork, they are ordinary Asana project sections and repository-owned prompts.
Moving a task into a configured section is the human authorization boundary.

## A deliberately small model

Flashy Factory has four concepts:

| Concept | Responsibility |
| --- | --- |
| Source | The ticket queue and control plane. Asana in this fork. |
| Trigger | A status, label, or schedule condition. |
| Workflow | A plain Markdown prompt describing the outcome and policy. |
| Worker | The agent runtime, sandbox, timeout, and concurrency limit. |

The boundary is intentional:

- Flashy Factory owns polling, trust checks, deduplication, durable claims,
  concurrency, timeouts, sandbox lifecycle, supervision, cancellation, history,
  and recovery.
- The workflow and agent own adaptive engineering work: reading the issue,
  inspecting code, clarifying requirements, implementing changes, using `gh`
  and `git`, opening a pull request, responding to CI and review, and updating
  the ticket.

Flashy Factory does not encode a fixed SDLC, a workflow graph, or deterministic
tracker effects. A trigger means only: **when this condition is true, run this
prompt**.

## Human review is the shipping boundary

Flashy Factory revalidates live source state immediately before execution, but
does not filter tasks by author. The trust boundary is whoever can move a task
into a configured Asana section or apply a required tag. Do not allow untrusted
people to satisfy those conditions. Task notes, comments, linked pull requests,
and attachments remain untrusted input regardless. Use narrow credentials and
protected branches that the worker cannot bypass.

Flashy Factory-created software pull requests remain for human review. Flashy
Factory and its default workflows never merge them or enable automatic merge.
The human who merges remains accountable for what ships.

For the complete trust and isolation model, read the
[operations guide](docs/operations.md) and [security policy](SECURITY.md).

## Get started

Install Python 3, Rust, Git, the GitHub CLI, and the Codex CLI. Export the Asana
project, workspace, and access-token variables described in the
[Asana guide](docs/asana.md), then authenticate the host tools and install:

```sh
gh auth login
codex login
cargo install --path . --locked
```

From the repository Flashy Factory will manage:

```sh
factory init
```

Edit the generated configuration and workflows for your repository, then
validate them and start Flashy Factory:

```sh
factory validate
factory run --once
factory run
```

The [runnable guide](docs/local-v1.md) covers the complete configuration, source
contract, first demonstration, two-repository fleet setup, and sandbox setup. The
[operations guide](docs/operations.md) covers inspection, cancellation,
recovery, and cleanup.

## V1 scope

This fork intentionally supports:

- one repository or an explicit fleet of repository-owned configurations;
- status, label, and schedule triggers;
- Codex workers in managed worktrees or Docker clones;
- explicit Markdown workflows;
- durable queueing, supervision, history, cancellation, and recovery.

Asana is the configured source of truth for this repository. The upstream
GitHub adapter and experimental Jira adapter remain as compatibility examples.
Linear, cross-repository workflows, hosted workers, and webhook wake-ups can
fit behind the same source, trigger, workflow, and worker boundaries later.

## Learn more

- [Vision and technical design](docs/design.md)
- [Labels and ticket status](docs/labels.md)
- [Setup, configuration, and first run](docs/local-v1.md)
- [Operations and recovery](docs/operations.md)
- [Asana source and agent client](docs/asana.md)
- [Jira source adapter](docs/jira.md)
- [Docker Sandbox development environment](docs/docker-sandbox-template.md)
- [Contributing](CONTRIBUTING.md)
- [Security](SECURITY.md)

## License

Flashy Factory is available under the [MIT License](LICENSE).
