#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
asana_client="${FLASHY_FACTORY_ASANA_CLIENT:-$script_dir/../.factory/clients/asana}"
ready_section="Ready For Spec"

usage() {
  echo "Usage: $0 <idea-title> [idea-description]" >&2
  echo "Creates a real demo task in ASANA_PROJECT_GID under ${ready_section}." >&2
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 2
fi

title=$1
body=${2:-"This is an early idea. Investigate the repository and turn it into a clear, bounded task with testable acceptance criteria."}

if [[ -z ${title//[[:space:]]/} ]]; then
  echo "The idea title must not be empty." >&2
  exit 2
fi

if [[ ! -x "$asana_client" ]]; then
  echo "Asana client is missing or not executable: $asana_client" >&2
  exit 1
fi

result=$(printf '%s\n' "$body" | "$asana_client" create \
  --name "$title" \
  --section "$ready_section" \
  --notes-file -)

task=$(printf '%s' "$result" | python3 -c '
import json
import sys
data = json.load(sys.stdin)["data"]
print(data.get("permalink_url") or data["gid"])
')

echo "Demo task: ${task}"
echo "Section: ${ready_section}"
echo
echo "Next:"
echo "  1. Run: cargo run -- run"
echo "  2. Wait for the agent to refine the task and move it to Creating Spec."
echo "  3. Review the task and move it to Ready To Implement."
echo "  4. Watch the implementation agent open a PR."
