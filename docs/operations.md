# Operate Flashy Factory v1

Flashy Factory is always watching, not always spending tokens. `factory run`
polls the configured Asana project and starts a worker only when a configured
section, tag, or schedule trigger matches.

## Normal operation

Start from anywhere inside the configured repository:

```sh
factory validate
factory run
```

Startup validates the configured execution backend before claiming work.
Lifecycle events report polls, claims, worker delegation, safe Codex progress,
and terminal outcomes. Progress distinguishes work such as reasoning, commands,
file changes, web searches, and subtask coordination. Flashy Factory deliberately does
not print prompts, reasoning content, command arguments, or raw tool output.
Use the supported inspection commands instead of reading SQLite directly:

```sh
factory tasks [--json]
factory runs [WORKFLOW] [--json]
factory inspect RUN_ID [--json]
```

Each ticket trigger is durable. Repeated polls and normal restarts do not create
another task while the issue remains in the same status or label entry. Leaving
and re-entering creates one new task-scoped sandbox. The workflow tells the
agent to find and continue an existing branch or change request when appropriate.

Flashy Factory stores durable state and managed worktrees below
`~/.flashy-factory/<repository-hash>/`. Set `FLASHY_FACTORY_DATA_HOME` to
override that root. The older `FACTORY_DATA_HOME` name remains an alias; if both
variables are set they must resolve to the same directory.

On upgrade, a repository that only has `.factory/config.toml` continues to use
that configuration and its `.factory/workflows`, clients, and sources in place.
`factory init` does not create a competing branded directory. New repositories
use `.flashy-factory`. If both config files exist, Flashy Factory stops and asks
you to keep one, rather than guessing.

The same no-data-loss rule applies to durable state. When the new default has no
ledger but `~/.factory/<repository-hash>/factory.sqlite3` exists, Flashy Factory
continues using the legacy repository directory. If ledgers exist under both
roots, it stops to prevent split durable work. Flashy Factory also refuses to
select a new default while a still older platform-data ledger remains, and
reports the `FLASHY_FACTORY_DATA_HOME` value that keeps using it.

The unscoped ledgers `~/.flashy-factory/factory.sqlite3` and legacy
`~/.factory/factory.sqlite3` both block startup because queued or running work
owned by an old process could overlap. These guards run when `factory run`
starts; inspection and cleanup commands remain available.

## Persistent Linux service with systemd

Running `factory run` in an SSH terminal is temporary. It ends when the process
receives a signal, the terminal closes, or the session is otherwise terminated.
For one repository that should remain supervised after logout and reboot, use a
dedicated systemd system service.

This is a deployment layer around Flashy Factory. `factory init` does not
install it. Replace every angle-bracket placeholder below, use a short
filesystem-safe service name, and repeat the setup for each independently
managed repository.

### 1. Verify the service account

Authenticate the repository host CLI and Codex as the same unprivileged Unix
account named by `User=` in the service:

```sh
gh auth status       # GitHub repository
glab auth status     # GitLab.com repository
codex login status
```

Use absolute executable and repository paths in the service. Confirm the
primary checkout is clean and that its `origin` still matches the identity in
`.flashy-factory/config.toml`.

### 2. Encrypt the Asana PAT

Create the system credential directory once, then encrypt a dedicated PAT. The
credential name passed to `systemd-creds` must match `LoadCredentialEncrypted`
in the unit:

```bash
sudo install -d -m 0700 /etc/credstore.encrypted
read -rsp "Asana PAT: " FACTORY_ASANA_PAT
printf '\n'
printf '%s' "$FACTORY_ASANA_PAT" | sudo systemd-creds encrypt \
  --name=asana_pat \
  - \
  /etc/credstore.encrypted/factory-<service-name>-asana-pat.cred
unset FACTORY_ASANA_PAT
```

The silent prompt keeps the PAT out of the command and shell history. On a host
without encrypted storage, systemd may warn that its host
credential key is not on encrypted media. Host-key encryption still avoids a
plaintext credential file, but full-disk encryption provides stronger
protection against offline physical access.

### 3. Install a credential-loading wrapper

`EnvironmentFile=` is not a secret store. Use a small root-owned wrapper to
read systemd's per-service credential file and export it only to Factory:

```sh
#!/bin/sh
set -eu

credential="${CREDENTIALS_DIRECTORY:?}/asana_pat"
ASANA_ACCESS_TOKEN="$(tr -d '\r\n' <"$credential")"
export ASANA_ACCESS_TOKEN

exec /absolute/path/to/factory run \
  --config /absolute/path/to/repository/.flashy-factory/config.toml
```

Install it as `/usr/local/libexec/factory-<service-name>-run`, owned by root and
mode `0755`. It must contain no token value. The PAT will be decrypted into a
private, read-only credential directory only for the service invocation.

### 4. Install the service unit

Create `/etc/systemd/system/factory-<service-name>.service`:

```ini
[Unit]
Description=Flashy Factory for <project-name>
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
User=<unix-user>
Group=<unix-group>
WorkingDirectory=/absolute/path/to/repository
Environment=PATH=/absolute/path/to/factory-bin:/absolute/path/to/provider-cli-bin:/absolute/path/to/codex-bin:/usr/local/bin:/usr/bin:/bin
Environment=ASANA_PROJECT_GID=<project-gid>
Environment=ASANA_WORKSPACE_GID=<workspace-gid>
LoadCredentialEncrypted=asana_pat:/etc/credstore.encrypted/factory-<service-name>-asana-pat.cred
ExecStart=/usr/local/libexec/factory-<service-name>-run
Restart=on-failure
RestartSec=10s
TimeoutStopSec=30s
NoNewPrivileges=yes
PrivateTmp=yes

[Install]
WantedBy=multi-user.target
```

Project and workspace GIDs are identifiers, not secrets. Keep the PAT in the
encrypted credential, never in `Environment=`. If the CLI installations live
under a version manager, pin their current absolute directories in `PATH` and
update the unit when those paths change.

### 5. Validate once, then enable continuous operation

First run `factory validate` and `factory run --once` with the same repository,
IDs, PATH, and PAT. The one-shot poll records eligible work but launches no
worker. Then install and start the unit:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now factory-<service-name>.service
systemctl is-enabled factory-<service-name>.service
systemctl is-active factory-<service-name>.service
journalctl -u factory-<service-name>.service -f
```

`enabled` means the service is configured to start at boot. `active` means it
is running now. Once both are true, normal SSH logins do not require `factory
init`, `factory run`, or a reset. Do not also start an interactive daemon for
the same repository.

### 6. Operate and maintain it

```sh
# Stop until manually started or the machine reboots:
sudo systemctl stop factory-<service-name>.service

# Start or restart it:
sudo systemctl start factory-<service-name>.service
sudo systemctl restart factory-<service-name>.service

# Stop now and disable start at boot:
sudo systemctl disable --now factory-<service-name>.service

# Restore start at boot:
sudo systemctl enable --now factory-<service-name>.service
```

Restart after editing repository configuration, workflows, the unit, wrapper,
or environment because the daemon loads its configuration at startup. Run
`systemctl daemon-reload` first after changing the unit itself.

The service survives SSH logout, starts after reboot, and restarts ordinary
process failures. It cannot run while the host is powered off, suspended, or
offline. Inspect it occasionally and after upgrades. Revoked credentials,
expired provider or Codex sessions, moved checkouts, changed remote identities,
Asana board changes, missing version-managed binaries, and repeated startup
failures still require operator attention. A systemd restart preserves the
durable ledger, task history, branches, and managed workspaces; it is not a
Factory reset.

Keep OAuth refresh credentials for verified backlog creation in a separate
oneshot service or external token manager. Do not load the OAuth client secret
or refresh token into the Factory daemon. See the
[Asana guide](asana.md#use-narrow-oauth-for-verified-backlog-batches).

For several repositories backed by distinct Asana projects, install one clearly
named service per repository by default. The generated Asana sources read
project and workspace GIDs from the supervisor's process environment, while a
fleet has no per-repository environment mapping. Use one
[fleet](#fleet-operation) only when its repositories intentionally share that
Asana environment or each repository has a custom source wrapper that injects
its own IDs and credentials. Never run a per-repository service and a fleet for
the same checkout.

## Repository worker concurrency

New repository-owned configurations start two workers at a time:

```toml
[worker]
max_concurrent = 2
```

Every ticket worker still receives a separate Flashy Factory-owned worktree.
FIFO keeps a third eligible task queued until a slot opens, and source
reauthorization still happens immediately before each worker starts. Autonomous
dependency work is released only after its predecessor's pull request is
human-merged; the dependent worktree is then prepared from a fresh fetch of the
default branch, including that merge. Flashy Factory never merges a pull
request or enables auto-merge.

For a safe rollout, change only `worker.max_concurrent` to `2`, restart the
daemon, and inspect `factory tasks`, `factory runs`, and `factory workspaces`
while two independent tickets run. At the ten retained-delivery-worktree
capacity, new delivery tasks remain queued, while recovery and reconciliation
for a task that already owns a worktree can continue. Inspect a retained
workspace before using `factory cleanup RUN_ID --confirm`.

To roll back, stop the daemon, set `worker.max_concurrent = 1`, and restart.
Active workers finish or are cancelled through the normal durable shutdown
path; do not delete worktrees or reset the ledger to reduce concurrency.

## Fleet operation

Run more than one trusted primary checkout from an explicit fleet file:

```sh
factory run --fleet ~/.config/factory/fleet.toml --once
factory run --fleet ~/.config/factory/fleet.toml
```

Fleet configuration is read once at startup. Adding, removing, editing,
enabling, or disabling a repository has no effect on the running process.
Restart Flashy Factory to apply the changed file. There is no hot reload and no fleet
`version` field. While that supervisor process is live, fleet inspection and
mutation commands use its durable fleet and repository configuration startup
snapshot, even if either file has since been edited. After the process stops,
commands load the current files.

Each `repository.name` is a pinned provider identity. GitHub uses `owner/name`;
GitLab.com uses `gitlab.com/namespace/name`, including every subgroup. Flashy
Factory lowercases it for durable identity, verifies the canonical primary
checkout and its `origin`, and never substitutes another remote or checkout
for existing state. Relative repository paths resolve from the fleet file's parent
directory. A leading `~` and `$NAME` environment variables use the same
expansion rules as repository configuration. An unset variable is an error.

The initial supported envelope is 20 enabled repositories, three
source-triggered workflows per repository, and polling intervals of at least
60 seconds. `max_concurrent` limits the fleet. An optional repository
`max_concurrent` combines with its repository worker limit by taking the lower
value. Flashy Factory admits eligible repositories round-robin, runs at most two host
source queries concurrently, deterministically staggers polls, and applies a
shared fleet-wide rate-limit pause when a provider reports rate limiting.

Inspect a running or stopped fleet with:

```sh
factory status --fleet ~/.config/factory/fleet.toml
factory tasks --fleet ~/.config/factory/fleet.toml
factory tasks --fleet ~/.config/factory/fleet.toml --repository acme/payments
factory runs --fleet ~/.config/factory/fleet.toml --repository acme/payments
factory polls --fleet ~/.config/factory/fleet.toml
factory workspaces --fleet ~/.config/factory/fleet.toml
factory recovery --fleet ~/.config/factory/fleet.toml
```

All fleet rows and JSON objects include pinned repository identity. `status`
reports `loading`, `healthy`, `invalid_config`, `unavailable`, `backing_off`,
`rate_limited`, or `disabled`, plus the latest validation or runtime error,
consecutive failure count, and next retry time when one exists. Ordinary
transient failures retry after 5 seconds and double to a 15-minute cap.
Rate-limit responses use the server retry time, or 60 seconds when no usable
time is available. A locally valid repository remains `loading` until a
supervisor durably records successful runtime validation; inspection never
invents a healthy state for an unstarted fleet. `polls` reports the latest
durable results. Continuous
supervision records each staggered workflow query. A one-shot evaluation
replaces those rows with one repository aggregate named `all`; later continuous
workflow results replace that aggregate without mixing stale rows.
`workspaces` reports active retained, recovery, and cleanup-pending ownership;
add `--all` to include cleaned records.

Read-only task and run lists aggregate every configured repository unless
`--repository <pinned-identity>` filters them, such as `acme/payments` or
`gitlab.com/group/subgroup/payments`. A single `inspect RUN_ID` may omit the
selector only when that numeric ID exists in one configured repository. If the
same ID exists in more than one ledger, Flashy Factory rejects the request and names
the required selector.

Cancellation and cleanup are mutations, so fleet mode always requires the
pinned repository:

```sh
factory cancel --fleet ~/.config/factory/fleet.toml \
  --repository acme/payments RUN_ID
factory cleanup --fleet ~/.config/factory/fleet.toml \
  --repository acme/payments RUN_ID
factory cleanup --fleet ~/.config/factory/fleet.toml \
  --repository acme/payments RUN_ID --confirm
```

Flashy Factory resolves the selector before opening a ledger, verifies the run, task,
and workspace ownership, and refuses unknown or inconsistent ownership without
mutation. This is required because repository ledgers can both contain run
`42`.

Disabling a repository takes effect after restart. Ctrl-C and process shutdown
first stop new polling and claims, cancel active workers, wait for them to
finish durable finalization, and leave queued work durable. On the next startup
a disabled repository is opened only for non-destructive reconciliation:
interrupted work becomes bounded recoverable or queued work but stays dormant.
Disabled repositories do not poll, create schedules, claim tasks, launch normal
or recovery workers, or perform destructive cleanup. Their ledgers, history,
branches, and owned workspaces remain. Re-enable the entry and restart to resume
normal recovery, reconciliation, polling, and dispatch.

Removing an entry from the fleet file does not open, move, reset, clean, or
delete its data directory or workspaces. Restore the same pinned identity and
canonical checkout path to operate that state again. Automatic cloning,
checkout relocation, and state migration are not supported.

## Worker boundary

Worktree mode runs the host Codex CLI in a Flashy Factory-owned Git worktree. It is
fast and uses the host user credentials and process boundary. It is not a
security sandbox and should be used only for trusted local work.

Docker Sandbox mode gives every task a disposable microVM and private in-VM Git
clone. Its Flashy Factory-owned host source clone is read-only to the VM. The canonical
checkout, Flashy Factory database, host Docker daemon, host credentials, and unrelated
repositories are outside the VM boundary.

The worker has full privileges inside the microVM, including its own Docker
daemon, while Docker Sandboxes applies the hypervisor and network boundaries.
Codex and the configured repository-host credentials are injected by a host proxy and their raw values
do not enter the VM. Flashy Factory records the sandbox name, template, `sbx` version,
and limits before creation. Before removal, Flashy Factory snapshots tracked and
untracked changes in the VM and fetches that commit into trusted host Git
metadata. If the handoff fails, Flashy Factory stops and retains the sandbox.

Docker Sandboxes blocks network access unless policy allows it, but allowed
services and Git remotes still create external effects. Treat all
issue, comment, attachment, and review text as untrusted input. Use a dedicated
provider identity and protected branches so the worker cannot merge or bypass
review.

## Prove an idle poll

When no configured trigger matches and no scheduled workflow is due,
capture the task list before and after one poll:

```sh
factory tasks --json
factory run --once
factory tasks --json
```

The two task listings should show zero new tasks. In Docker Sandbox mode, also
run `sbx ls --quiet` and confirm that no `factory-` sandbox was created. This
proves the empty poll persisted and launched nothing.

## Cancellation and recovery

Request cancellation with:

```sh
factory cancel RUN_ID
```

Ctrl-C stops new polling and claims, cancels active workers, records terminal
outcomes, and leaves queued work durable. Worktree mode supervises the host
process group. Docker Sandbox mode also reconciles durable sandbox ownership by
its Flashy Factory instance name before stopping or removing a VM. It captures
recovered evidence, then permits bounded recovery. Repeated failure remains
inspectable and never turns into an automatic merge.

## Workspace retention and cleanup

Successful clean ticket workspaces are removed when they made no code commits,
or after their current branch is pushed. Failed, cancelled, dirty, unpublished,
or incomplete ticket workspaces are retained for recovery, up to ten.

Preview a retained clone before removal:

```sh
factory cleanup RUN_ID
factory cleanup RUN_ID --confirm
```

Confirmed cleanup removes only the recorded managed worktree or standalone
clone. The remote branch and change request remain. Proposal workspaces are
disposable and are removed at a terminal outcome.

## Reset durable state

Preview a fresh start before removing task history:

```sh
factory reset
factory reset --confirm
```

Reset includes the current repository ledger and the older global ledger when
it exists. Confirmation is refused while either ledger has queued or running
work, a live daemon lease, an unremoved managed container, or retained workspace
ownership. Stop Flashy Factory and use `factory cleanup RUN_ID --confirm` for retained
work before retrying.

Reset removes only SQLite state. Repository configuration, workflows, branches,
and worktree directories are preserved.

## Troubleshooting

- `factory init --check` reports missing repository assets without writing.
- `factory validate` reports invalid triggers, trusted users, host Codex
  availability in worktree mode, `sbx` and secret prerequisites in Docker
  Sandbox mode, and data-path permissions.
- `factory run --once` proves polling without launching a model or worker.
- `factory inspect RUN_ID` shows bounded task, workspace, optional sandbox,
  branch, change-request, and error evidence.
- `factory status --fleet FLEET` reports each configured repository even when a
  peer is invalid or unavailable. Use its error, failure count, and retry time
  before changing configuration.
- An `invalid_config` repository is not retried until restart. `unavailable`,
  `backing_off`, and `rate_limited` repositories remain retryable and never
  become permanently disabled because of repeated transient failures.

Scheduled workflows use the same configured worker as ticket workflows.
