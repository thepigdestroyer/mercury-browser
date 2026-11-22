#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Refreshes crates/talaria-servo/resources/hsts_preload.fstmap and its manifest from Chromium's HSTS
# preload list. A release-time maintainer step (make hsts-preload-refresh): it needs the network,
# CI never runs it, and it leaves the changes uncommitted for review.
#   update-hsts-preload.sh [<chromium-ref>] [-- <build option>...]
#       download from Gitiles (default ref: main). The ref is resolved to a commit first, the
#       list is downloaded from that commit and the manifest records the commit.
#   update-hsts-preload.sh --from <file> [--base64] [<ref>] [-- <build option>...]
#       build from a local copy; no download
# Everything after -- goes to `hsts-preload build`, for example `-- --min-entries 40000
# --allow-shrink --allow-flag-loss` when a refresh is meant to drop many hosts.
set -eu
LC_ALL=C; export LC_ALL

usage() {
    echo "usage: update-hsts-preload.sh [<chromium-ref>] [-- <build option>...]" >&2
    echo "       update-hsts-preload.sh --from <file> [--base64] [<chromium-ref>] [-- <build option>...]" >&2
    exit "${1:-2}"
}

from=; base64=
while [ $# -gt 0 ]; do
    case $1 in
        --) shift; break ;;
        --from) [ $# -ge 2 ] || usage; from=$2; shift 2 ;;
        --base64) base64=--base64; shift ;;
        -h | --help) usage 0 ;;
        -*) usage ;;
        *) [ "${ref+set}" != set ] || usage; ref=$1; shift ;;
    esac
done
[ -z "$base64" ] || [ -n "$from" ] || usage
ref=${ref-main}
# A Gitiles branch, tag or commit: plain path characters, and no ".." or doubled, leading or
# trailing slash, so that the ref cannot leave the /+/<ref>/ part of the URL.
case $ref in
    '' | *[!A-Za-z0-9._/-]* | *..* | /* | */ | *//* | -*)
        echo "update-hsts-preload.sh: refusing the Chromium ref '$ref'" >&2; exit 2 ;;
esac

case $from in /* | '') ;; *) from=$PWD/$from ;; esac
cd "$(dirname "$0")/../.."
data=crates/talaria-servo/resources/hsts_preload.fstmap
host=https://chromium.googlesource.com/chromium/src/+
file=net/http/transport_security_state_static.json
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT; trap 'exit 1' HUP INT TERM
tool() { cargo run --locked --quiet --release -p mercury-devtools --bin hsts-preload -- "$@"; }
fetch() {
    curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
        --tlsv1.2 --max-time 600 --retry 2 --max-filesize 100000000 --output "$1" "$2"
}

if [ -n "$from" ]; then
    input=$from
else
    # Gitiles answers /+/<ref>?format=JSON with the commit ("commit": "<40 hex digits>") after a
    # one-line )]}' prefix. Downloading from the commit, not from a moving branch, means that the
    # input can be fetched again later, and the manifest records the commit.
    fetch "$tmp/commit.json" "$host/$ref?format=JSON"
    commit=$(sed -n 's/^ *"commit": *"\([0-9a-f]\{40\}\)".*$/\1/p' "$tmp/commit.json" | head -n 1)
    [ -n "$commit" ] || { echo "update-hsts-preload.sh: no commit found for '$ref'" >&2; exit 1; }
    ref=$commit
    input=$tmp/chromium.json.base64
    base64=--base64
    fetch "$input" "$host/$ref/$file?format=TEXT"
fi
url="$host/$ref/$file?format=TEXT"
if [ -f "$data" ]; then cp "$data" "$tmp/old.fstmap"; fi

tool build ${base64:+"$base64"} --input "$input" --ref "$ref" --source-url "$url" --output "$data" "$@"
tool check "$data"
if [ -f "$tmp/old.fstmap" ]; then
    tool diff "$tmp/old.fstmap" "$data" || echo "(the file it replaced could not be compared)"
fi
echo "Nothing is committed. Review with: git diff --stat -- crates/talaria-servo/resources"
echo "(later: git show HEAD:$data > old.fstmap; hsts-preload diff old.fstmap $data). Commit both files."
