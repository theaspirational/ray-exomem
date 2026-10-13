#!/usr/bin/env bash
# Deploy ray-exomem on the server it runs on. Run it there, from the checkout:
#
#   scripts/deploy.sh                build, gate, back up, swap, restart
#   scripts/deploy.sh --check        build and gate only; touches nothing live
#   scripts/deploy.sh --accept-diff  deploy even though the gate found a difference
#
# The gate is the point of this script. A rayforce or storage change can make a
# new binary read the same files as different strings without any error, and the
# first write after such a read makes the damage permanent. So before the live
# binary is replaced, the running binary and the new one each load a copy of the
# live data, and their full export and a Rayfall query per exom must be identical.

set -euo pipefail

UNIT="${DEPLOY_UNIT:-ray-exomem.service}"
PORT="${DEPLOY_PORT:-9780}"
DATA="${DEPLOY_DATA:-$HOME/.ray-exomem}"
BIN="${DEPLOY_BIN:-bin/ray-exomem}"
BACKUPS="${DEPLOY_BACKUPS:-$HOME/backups}"
GATE_PORT_OLD=9798
GATE_PORT_NEW=9799
NEW=target/release/ray-exomem

main() {
  local mode="${1:-}"
  case "$mode" in "" | --check | --accept-diff) ;; *) echo "unknown option: $mode" >&2; exit 2 ;; esac

  cd "$(dirname "$0")/.."
  # A non-interactive ssh shell has neither cargo nor the nvm node on PATH.
  export PATH="$HOME/.cargo/bin:$PATH"
  if ! command -v npm >/dev/null && [ -s "$HOME/.nvm/nvm.sh" ]; then
    set +u; . "$HOME/.nvm/nvm.sh"; set -u
  fi

  echo "==> git pull --ff-only"
  git pull --ff-only
  echo "==> cargo build --release --bin ray-exomem ($(git log --oneline -1))"
  cargo build --release --bin ray-exomem

  if [ -x "$BIN" ]; then
    if gate; then
      echo "==> gate passed: both binaries read the data identically"
    elif [ "$mode" = --accept-diff ]; then
      echo "==> gate found differences; continuing because of --accept-diff"
    else
      echo "!!  gate FAILED: the new binary reads the data differently (see above)." >&2
      echo "!!  Nothing live was touched. If the difference is the fix you are" >&2
      echo "!!  deploying, rerun with --accept-diff." >&2
      exit 1
    fi
  else
    echo "==> no $BIN yet, nothing to compare against; skipping the gate"
  fi
  if [ "$mode" = --check ]; then exit 0; fi

  local ts backup old
  ts=$(date -u +%Y%m%dT%H%M%SZ)
  backup="$BACKUPS/ray-exomem-pre-deploy-$ts.tar.gz"
  old="$BIN.pre-deploy.$ts"

  echo "==> stopping $UNIT"
  systemctl --user stop "$UNIT"
  mkdir -p "$BACKUPS" "$(dirname "$BIN")"
  tar -C "$(dirname "$DATA")" -czf "$backup" "$(basename "$DATA")"
  echo "==> data backup: $backup"
  if [ -x "$BIN" ]; then
    cp -p "$BIN" "$old"
    echo "==> old binary: $old"
  fi
  cp "$NEW" "$BIN.new"
  mv -f "$BIN.new" "$BIN"

  echo "==> starting $UNIT"
  systemctl --user start "$UNIT"
  if wait_http "$PORT" && [ "$(systemctl --user is-active "$UNIT")" = active ]; then
    echo "==> ok: new version is up"
    return
  fi

  echo "!!  new version did not come up; rolling back" >&2
  journalctl --user -u "$UNIT" --since "-2min" -o cat --no-pager | tail -30 >&2 || true
  systemctl --user stop "$UNIT" || true
  rm -rf "$DATA"
  tar -C "$(dirname "$DATA")" -xzf "$backup"
  if [ -e "$old" ]; then
    cp -p "$old" "$BIN"
    systemctl --user start "$UNIT"
    echo "!!  rolled back to the previous binary and data: $(systemctl --user is-active "$UNIT")" >&2
  fi
  exit 1
}

wait_http() {
  for _ in $(seq 60); do
    if curl -fs -m 2 "http://127.0.0.1:$1/auth/info" >/dev/null; then return 0; fi
    sleep 0.5
  done
  return 1
}

# Load one snapshot of the live data with the running binary and with the new
# one, on throwaway no-auth daemons, and compare what they read.
gate() {
  local tmp pids=() rc=0
  tmp=$(mktemp -d)
  cp -a "$DATA" "$tmp/snapshot"
  rm -f "$tmp/snapshot/sym.lk"
  cp -a "$tmp/snapshot" "$tmp/old"
  cp -a "$tmp/snapshot" "$tmp/new"

  "$BIN" serve --bind "127.0.0.1:$GATE_PORT_OLD" --data-dir "$tmp/old" >"$tmp/old.log" 2>&1 &
  pids+=($!)
  "$NEW" serve --bind "127.0.0.1:$GATE_PORT_NEW" --data-dir "$tmp/new" >"$tmp/new.log" 2>&1 &
  pids+=($!)

  echo "==> gate: comparing $BIN and $NEW on a copy of $DATA"
  if ! wait_http "$GATE_PORT_OLD"; then
    echo "!!  the running binary did not start on the copy:" >&2; tail -20 "$tmp/old.log" >&2; rc=1
  elif ! wait_http "$GATE_PORT_NEW"; then
    echo "!!  the new binary did not start on the copy:" >&2; tail -20 "$tmp/new.log" >&2; rc=1
  else
    python3 - "$GATE_PORT_OLD" "$GATE_PORT_NEW" <<'PY' || rc=1
import json, sys, urllib.parse, urllib.request

OLD, NEW = sys.argv[1], sys.argv[2]


def call(port, path, body=None):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=body)
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)


def exoms(node):
    here = [node["path"]] if node.get("kind") == "exom" else []
    return here + [p for child in node.get("children") or [] for p in exoms(child)]


def canon(value):
    """Order-independent form: row order may differ, row content may not."""
    if isinstance(value, dict):
        return {k: canon(v) for k, v in value.items()}
    if isinstance(value, list):
        return sorted((canon(v) for v in value), key=lambda v: json.dumps(v, sort_keys=True))
    return value


def read(port, exom):
    q = urllib.parse.quote(exom, safe="")
    export = call(port, f"/api/actions/export-json?exom={q}")
    query = f"(query {exom} (find ?id ?p ?v) (where (fact-row ?id ?p ?v)))".encode()
    try:
        rows = call(port, "/api/query", query).get("rows", "no rows in reply")
    except Exception as err:  # an engine error is a result to compare, not a crash
        rows = f"query failed: {type(err).__name__}"
    parts = {f"export.{key}": canon(value) for key, value in export.items()}
    parts["fact-row query"] = canon(rows)
    return parts


old_exoms, new_exoms = sorted(exoms(call(OLD, "/api/tree"))), sorted(exoms(call(NEW, "/api/tree")))
bad = old_exoms != new_exoms
if bad:
    print(f"  exom list differs: only old {sorted(set(old_exoms) - set(new_exoms))}, "
          f"only new {sorted(set(new_exoms) - set(old_exoms))}")
for exom in sorted(set(old_exoms) & set(new_exoms)):
    old, new = read(OLD, exom), read(NEW, exom)
    diffs = [part for part in sorted(set(old) | set(new)) if old.get(part) != new.get(part)]
    bad |= bool(diffs)
    print(f"  {'DIFF' if diffs else 'ok  '} {exom}: {len(old.get('export.facts', []))} fact rows")
    for part in diffs:
        a, b = old.get(part), new.get(part)
        print(f"       {part} differs")
        if isinstance(a, list) and isinstance(b, list):
            as_set = lambda rows: {json.dumps(r, sort_keys=True, ensure_ascii=False) for r in rows}
            for label, rows in (("only old", as_set(a) - as_set(b)), ("only new", as_set(b) - as_set(a))):
                for row in sorted(rows)[:5]:
                    print(f"         {label}: {row[:300]}")
        else:
            print(f"         old: {str(a)[:300]}\n         new: {str(b)[:300]}")
sys.exit(1 if bad else 0)
PY
  fi

  kill "${pids[@]}" 2>/dev/null || true
  wait "${pids[@]}" 2>/dev/null || true
  rm -rf "$tmp"
  return $rc
}

# Parse the whole file before running anything: `git pull` may rewrite it.
main "$@"
