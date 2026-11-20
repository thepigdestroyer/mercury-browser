#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Refreshes crates/talaria-servo/resources/hsts_preload.fstmap and its manifest from Chromium's HSTS
# preload list. A release-time maintainer step (make hsts-preload-refresh): it needs the network,
# CI never runs it, and it leaves the changes uncommitted for review.
#   update-hsts-preload.sh [<chromium-ref>]                 download from Gitiles (default: main)
#   update-hsts-preload.sh --from <file> [--base64] [<ref>]  build from a local copy; no download
set -eu
LC_ALL=C; export LC_ALL

usage() {
    echo "usage: update-hsts-preload.sh [<chromium-ref>]" >&2
    echo "       update-hsts-preload.sh --from <file> [--base64] [<chromium-ref>]" >&2
    exit "${1:-2}"
}

from=; base64=
while [ $# -gt 0 ]; do
    case $1 in
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
url="https://chromium.googlesource.com/chromium/src/+/$ref/net/http/transport_security_state_static.json?format=TEXT"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT; trap 'exit 1' HUP INT TERM
tool() { cargo run --locked --quiet --release -p mercury-devtools --bin hsts-preload -- "$@"; }

if [ -n "$from" ]; then
    input=$from
else
    input=$tmp/chromium.json.base64
    base64=--base64
    curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
        --tlsv1.2 --max-time 600 --retry 2 --output "$input" "$url"
fi
if [ -f "$data" ]; then cp "$data" "$tmp/old.fstmap"; fi

tool build ${base64:+"$base64"} --input "$input" --ref "$ref" --source-url "$url" --output "$data"
tool check "$data"
if [ -f "$tmp/old.fstmap" ]; then
    tool diff "$tmp/old.fstmap" "$data" || echo "(the file it replaced could not be compared)"
fi
echo "Nothing is committed. Review with: git diff --stat -- crates/talaria-servo/resources"
echo "(later: git show HEAD:$data > old.fstmap; hsts-preload diff old.fstmap $data). Commit both files."
