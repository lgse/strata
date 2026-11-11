#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Stands in for Strata's sources in the test image's dependency stage.

The stage holds the workspace's manifests and nothing else, so that only third-party crates
are fetched and built there. Cargo still has to load every manifest, and it refuses one whose
targets have no file. `stub` writes an empty file for each target a manifest names or Cargo
would look for. `leftovers` lists what a build of those stubs left in a target directory,
which the stage requires to be nothing: a stub built there must never pass for the real
crate in a run.
"""

import argparse
from pathlib import Path
import re
import sys
import tomllib

MAIN = "fn main() {}\n"
# Where Cargo looks for a target that gives its name and no path.
INFERRED = {"bin": "src/bin", "example": "examples", "test": "tests", "bench": "benches"}
# Where Cargo puts a package's own artifacts, in a profile's directory.
ARTIFACTS = ("deps", ".fingerprint", "build", "examples")


def load(path):
    with path.open("rb") as stream:
        return tomllib.load(stream)


def manifests(root):
    """The workspace's packages as Cargo finds them: the root's own and its members'."""
    workspace = load(root / "Cargo.toml")
    found = [(root, workspace)] if "package" in workspace else []
    for pattern in workspace.get("workspace", {}).get("members", []):
        found += [(path.parent, load(path)) for path in sorted(root.glob(f"{pattern}/Cargo.toml"))]
    return found


def inferred_path(kind, name, package):
    if kind == "bin" and name == package:
        return "src/main.rs"
    return f"{INFERRED[kind]}/{name}.rs"


def named_targets(manifest):
    """The `[[bin]]`, `[[example]]`, `[[test]]` and `[[bench]]` entries: name and path."""
    package = manifest["package"]["name"]
    for kind in INFERRED:
        for target in manifest.get(kind, []):
            name = target.get("name", package)
            yield name, target.get("path", inferred_path(kind, name, package))


def build_script(manifest):
    """The build script to stand in for, if the package's dependencies need one built."""
    declared = manifest["package"].get("build")
    if isinstance(declared, str):
        return declared
    if declared is not False and "build-dependencies" in manifest:
        return "build.rs"
    return None


def stubs(manifest):
    """Path and contents of each file that makes the package load and build empty.

    A manifest cannot say whether the package has a library, a binary or both where Cargo
    looks by default, so the stub has both. Either way its dependencies are the same.
    """
    files = {manifest.get("lib", {}).get("path", "src/lib.rs"): "", "src/main.rs": MAIN}
    files.update({path: MAIN for _, path in named_targets(manifest)})
    if script := build_script(manifest):
        files[script] = MAIN
    return files


def package_names(root):
    return [manifest["package"]["name"] for _, manifest in manifests(root)]


def write_stubs(root):
    for directory, manifest in manifests(root):
        for name, contents in stubs(manifest).items():
            path = directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents, encoding="utf-8")
    return package_names(root)


def crate_names(manifest):
    """The names Cargo gives the package's artifacts: the package's and each target's."""
    package = manifest["package"]["name"]
    names = {package, manifest.get("lib", {}).get("name", package)}
    names.update(name for name, _ in named_targets(manifest))
    return names | {name.replace("-", "_") for name in names}


def artifact_places(target):
    """A profile's directory and the directories of artifacts in it.

    What a third-party build script wrote further down is not Strata's, whatever its name.
    """
    profiles = [path for path in target.iterdir() if path.is_dir()]
    nested = [profile / name for profile in profiles for name in ARTIFACTS]
    return profiles + [path for path in nested if path.is_dir()]


def leftovers(root, target):
    """What a build of the stubs left in a target directory, by Cargo's artifact names."""
    names = set().union(*(crate_names(manifest) for _, manifest in manifests(root)))
    alternatives = "|".join(sorted(re.escape(name) for name in names))
    # `<name>-<hash>` in deps, .fingerprint and build; `<name>` and `lib<name>` beside them.
    artifact = re.compile(rf"(?:lib)?(?:{alternatives})(?:-[0-9a-f]{{16}})?(?:\.[A-Za-z0-9.]+)?")
    return sorted(str(path.relative_to(target)) for place in artifact_places(target)
                  for path in place.iterdir() if artifact.fullmatch(path.name))


def report_packages(names, root):
    if not names:
        print(f"No package manifest under {root}.", file=sys.stderr)
        return 1
    print(" ".join(names))
    return 0


def report_leftovers(left):
    for path in left:
        print(f"left by the stubs: {path}", file=sys.stderr)
    return 1 if left else 0


def main(arguments=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("stub", "packages", "leftovers"):
        commands.add_parser(name).add_argument("root", type=Path)
    commands.choices["leftovers"].add_argument("target", type=Path)
    options = parser.parse_args(arguments)
    if options.command == "leftovers":
        return report_leftovers(leftovers(options.root, options.target))
    listed = write_stubs if options.command == "stub" else package_names
    return report_packages(listed(options.root), options.root)


if __name__ == "__main__":
    sys.exit(main())
