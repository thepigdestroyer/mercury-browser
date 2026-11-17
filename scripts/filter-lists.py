#!/usr/bin/env python3
"""Check or install Mercury Browser's bundled filter lists (the data files the content blocker reads).

    python3 scripts/filter-lists.py check [<directory>]
    python3 scripts/filter-lists.py install <destination>

The lists and their manifest live in crates/talaria-servo/resources/adblock/ (the default
<directory>). `check` verifies that manifest.json names each list once, as a plain .txt name,
with an https source URL and the SHA-256 of the file beside it. `install` checks, then
replaces <destination> with a directory holding exactly manifest.json and the lists it names
(the macOS bundle's Contents/Resources/filter-lists, the Linux archive's filter-lists/,
target/<profile>/filter-lists for local runs). Exits non-zero, writing nothing, if a check
fails. Standard library only.
"""

import hashlib
import json
import os
import re
import shutil
import sys
import tempfile

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), os.pardir)
DEFAULT_SOURCE = os.path.join(ROOT, "crates", "talaria-servo", "resources", "adblock")
MANIFEST = "manifest.json"
SAFE_FILE = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}\.txt$")


def fail(message):
    print("filter-lists: error: " + message, file=sys.stderr)
    sys.exit(1)


def checked_files(directory):
    """The list files the manifest in `directory` names, after checking every entry."""
    try:
        with open(os.path.join(directory, MANIFEST), encoding="utf-8") as handle:
            manifest = json.load(handle)
    except (OSError, ValueError) as error:
        fail("cannot read %s in %s: %s" % (MANIFEST, directory, error))
    if manifest.get("format_version") != 1:
        fail("%s has format_version %r; expected 1" % (MANIFEST, manifest.get("format_version")))
    lists = manifest.get("lists")
    if not isinstance(lists, list) or not lists:
        fail("%s names no lists" % MANIFEST)
    files = []
    for entry in lists:
        name = entry.get("file", "")
        if not SAFE_FILE.match(name) or name in files:
            fail("unsafe or repeated file name %r" % name)
        if not str(entry.get("source_url", "")).startswith("https://"):
            fail("%s: the source URL is not https" % name)
        if not entry.get("licence"):
            fail("%s: no licence" % name)
        path = os.path.join(directory, name)
        try:
            with open(path, "rb") as handle:
                digest = hashlib.sha256(handle.read()).hexdigest()
        except OSError as error:
            fail("cannot read %s: %s" % (path, error))
        if digest != entry.get("sha256"):
            fail("%s: SHA-256 %s does not match the manifest's %s" % (name, digest, entry.get("sha256")))
        files.append(name)
    return files


def install(source, destination):
    files = checked_files(source)
    destination = os.path.abspath(destination)
    parent = os.path.dirname(destination)
    os.makedirs(parent, exist_ok=True)
    staging = tempfile.mkdtemp(prefix=".filter-lists-", dir=parent)
    try:
        for name in [MANIFEST] + files:
            shutil.copyfile(os.path.join(source, name), os.path.join(staging, name))
            os.chmod(os.path.join(staging, name), 0o644)
        os.chmod(staging, 0o755)
        checked_files(staging)
        if os.path.lexists(destination):
            if os.path.islink(destination) or not os.path.isdir(destination):
                fail("%s exists and is not a directory" % destination)
            shutil.rmtree(destination)
        os.rename(staging, destination)
    finally:
        if os.path.isdir(staging):
            shutil.rmtree(staging)
    print("filter-lists: installed %d lists in %s" % (len(files), destination))


def main(arguments):
    if len(arguments) >= 1 and arguments[0] == "check" and len(arguments) <= 2:
        directory = arguments[1] if len(arguments) == 2 else DEFAULT_SOURCE
        files = checked_files(directory)
        print("filter-lists: %d lists match %s" % (len(files), os.path.join(directory, MANIFEST)))
        return 0
    if len(arguments) == 2 and arguments[0] == "install":
        install(DEFAULT_SOURCE, arguments[1])
        return 0
    print(__doc__.strip(), file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
