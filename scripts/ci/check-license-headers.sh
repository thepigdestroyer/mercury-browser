#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Check that every file in the repository says which licence it is under: an
# SPDX-License-Identifier line near its top (PolyForm-Noncommercial-1.0.0 for
# Mercury's own files; MPL-2.0, Apache-2.0, MIT or BSD for adapted code), or an
# entry in REUSE.toml. Also reports REUSE.toml entries for files
# that no longer exist. Vendored code in third_party/ and the licence texts are
# skipped. See LICENSE and CONTRIBUTING.md.
#
#   scripts/ci/check-license-headers.sh
set -euo pipefail

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.."

listed="$(sed -n 's/^  "\(.*\)",$/\1/p' REUSE.toml | sort -u)"
failed=0

while IFS= read -r file; do
    [ -f "$file" ] || continue
    id="$(head -n 30 -- "$file" | LC_ALL=C sed -n 's/.*SPDX-License-Identifier: *\([A-Za-z0-9.+-]*\).*/\1/p' | head -n 1)"
    case "$id" in
        PolyForm-Noncommercial-1.0.0 | MPL-2.0 | Apache-2.0 | MIT | BSD-2-Clause | BSD-3-Clause) continue ;;
        "") grep -q -x -F -- "$file" <<<"$listed" && continue
            printf 'no licence: %s (add an SPDX header, or list it in REUSE.toml)\n' "$file" ;;
        *) printf 'unexpected licence %s: %s\n' "$id" "$file" ;;
    esac
    failed=1
done < <(git ls-files --cached --others --exclude-standard |
    grep -v -E '^(third_party/|LICENSES/|LICENSE$|Cargo\.lock$|REUSE\.toml$)')

while IFS= read -r file; do
    [ -e "$file" ] || { printf 'REUSE.toml lists a missing file: %s\n' "$file"; failed=1; }
done <<<"$listed"

[ "$failed" -eq 0 ] && printf '%s\n' 'licence headers: every file has one'
exit "$failed"
