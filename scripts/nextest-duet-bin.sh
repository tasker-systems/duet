#!/usr/bin/env bash
# nextest setup script for duet-e2e (.config/nextest.toml): build the working tree's `duet`
# binary once for the run, and give every test its path as DUET_E2E_BIN. The build and the way
# its executable is found are the same as crates/duet-e2e/src/lib.rs's `build()`, which still
# serves `cargo test`.
set -euo pipefail

# A binary the developer chose, such as one built from `main`, is used as it is.
# crates/duet-e2e/src/lib.rs checks that it is a file.
if [ -n "${DUET_E2E_BIN:-}" ]; then
  exit 0
fi

bin="$(
  "${CARGO:-cargo}" build --locked -p duet-cli --bin duet --message-format=json-render-diagnostics |
    python3 -I -c '
import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m["target"]["name"] == "duet" and m.get("executable"):
        print(m["executable"])
'
)"

if [ -z "$bin" ] || [ ! -f "$bin" ]; then
  echo "cargo build -p duet-cli --bin duet reported no executable" >&2
  exit 1
fi

echo "DUET_E2E_BIN=$bin" >> "$NEXTEST_ENV"
