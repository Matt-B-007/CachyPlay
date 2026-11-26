#!/bin/sh
# Refuse a version that would rank at or below a release that already exists, because the
# updater would never offer it to anyone.
#   printf 'v0.1.0\nv0.1.0-beta.10\n' | scripts/version-guard.sh 0.1.0-beta.11   -> fails
# Existing release tags come on stdin, one per line. A finished version ranks above its betas:
# 0.1.0-beta.11 < 0.1.0 < 0.1.1-beta.1.
set -eu
new="${1:?usage: version-guard.sh VERSION < existing-tags}"
new="${new#v}"
key() { case "$1" in *-*) printf '%s\n' "$1" ;; *) printf '%s-zzz\n' "$1" ;; esac; }
newest=""
while IFS= read -r tag; do
  tag="${tag#v}"
  [ -n "$tag" ] && [ "$tag" != "$new" ] || continue
  if [ -z "$newest" ] || [ "$(printf '%s\n%s\n' "$(key "$newest")" "$(key "$tag")" | sort -V | tail -n1)" = "$(key "$tag")" ]; then
    newest="$tag"
  fi
done
[ -n "$newest" ] || exit 0
top=$(printf '%s\n%s\n' "$(key "$newest")" "$(key "$new")" | sort -V | tail -n1)
if [ "$top" != "$(key "$new")" ]; then
  echo "Version $new ranks below the existing release $newest, so nobody would be offered it." >&2
  case "$new" in
    *-*) echo "A beta after a stable release needs the next version number, e.g. ${newest%%-*} -> bump the last number and add -beta.1." >&2 ;;
  esac
  exit 1
fi
