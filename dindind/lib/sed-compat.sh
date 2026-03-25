#!/bin/bash
# sed compatibility shim: makes BSD `sed -i ''` work on GNU sed.
# BSD sed requires `sed -i '' 's/a/b/' file` while GNU sed uses `sed -i 's/a/b/' file`.
# This shim detects the empty-string backup suffix after -i and drops it,
# keeping the -i flag itself.

REAL_SED=/usr/bin/sed
args=()
skip_empty=false

for arg in "$@"; do
  if [ "$skip_empty" = true ]; then
    skip_empty=false
    if [ "$arg" = "" ]; then
      continue
    fi
  fi
  if [ "$arg" = "-i" ]; then
    args+=("-i")
    skip_empty=true
    continue
  fi
  args+=("$arg")
done

exec "$REAL_SED" "${args[@]}"
