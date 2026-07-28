# Use Jira as a source

This Flashy Factory fork uses Asana by default. It retains a Jira adapter backed
by `jiractrl` and `jq` to demonstrate the provider-neutral source contract. The
adapter asks Jira for only issues matching the trigger's exact state and
labels. It does not filter by author; restrict trust to people who can label
issues in the target Jira project.

## Authenticate

Configure `jiractrl` first:

```sh
export JIRACTRL_BASE_URL="https://jira.example.com"
export JIRACTRL_TOKEN="..."
jiractrl auth check
```

Use a dedicated, revocable credential. Do not commit it to the repository or
put it in issue content, logs, or workflow files.

## Configure the adapter

Copy the Jira workflow examples into Flashy Factory's executable workflow directory:

```sh
cp examples/jira-triage.md .flashy-factory/workflows/jira-triage.md
cp examples/jira-implement.md .flashy-factory/workflows/jira-implement.md
```

Replace the source and workflow paths in `.flashy-factory/config.toml`. Adapt the
project key, state names, and label to your Jira project:

```toml
[source]
command = [
  ".flashy-factory/sources/jira",
  "--project", "SPS",
  "--max-results", "100",
]

[trigger.triage]
type = "source"
state = "Ready For Spec"
labels = ["factory-ready"]
workflow = ".flashy-factory/workflows/jira-triage.md"

[trigger.implement]
type = "source"
state = "Ready To Implement"
labels = ["factory-ready"]
workflow = ".flashy-factory/workflows/jira-implement.md"
timeout = "4h"
```

The adapter builds bounded JQL such as:

```text
project = "SPS" AND status = "Ready To Implement" AND labels = "factory-ready"
```

Flashy Factory passes only the Jira key, such as `SPS-123`, to the worker. The example
Jira workflows tell the agent to fetch, comment, update, and transition the
live ticket with `jiractrl`; `git` and `gh` remain responsible for code and pull
requests. Their source templates live under `examples/`; the default
`.flashy-factory/workflows` directory is tailored to Asana.

## Worker requirements

The included Jira example is configured with `sandbox = "worktree"`, so the
worker inherits the host's `jiractrl` binary and Jira environment variables. A
Docker worker would also need `jiractrl` in its image and an explicit Jira
credential mount or environment policy. That environment is not included in
this example.

Read the [runnable guide](local-v1.md) for the complete Flashy Factory configuration
and the [source adapter contract](local-v1.md#source-adapter-contract) that the
Jira script implements.
