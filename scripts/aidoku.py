#!/usr/bin/env python3
"""Workspace-aware stand-in for the `aidoku` CLI.

The upstream CLI assumes each source is a standalone crate with its own target
directory. Every source and template in this repo is a member of one Cargo
workspace instead, which breaks two of its commands:

  * `aidoku package` copies the *first* .wasm it finds in the build directory.
    With one shared `target/` that is whichever source happened to build first,
    not the one being packaged.
  * `aidoku init` scaffolds a standalone crate: pinned dependency versions, its
    own `[profile.*]`, `.cargo/config.toml`, `Cargo.lock` and git repo. A
    workspace member has to inherit all of that from the root instead.

Those two are reimplemented here against the workspace layout. The rest
(`build`, `verify`, `serve`, `logcat`) only ever touch `.aix` files, so they are
forwarded to the real CLI with the workspace's defaults filled in.

`package --reuse-from` and `manifest` are additions on top: together they let a
build copy back the packages whose inputs haven't changed since the last one,
so CI only recompiles the sources a push actually touched.

Run `scripts/aidoku.py --help`, or `--help` on any command, for usage.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tomllib
import unicodedata
import zipfile
import zlib
from pathlib import Path
from typing import Any, Callable, Iterable, NamedTuple, NoReturn, Sequence

ROOT: Path = Path(__file__).resolve().parent.parent
TARGET = "wasm32-unknown-unknown"
SOURCE_LIST_NAME = "Amqx's Sources"
MANIFEST_NAME = "build-manifest.json"
CLI_INSTALL_HINT = (
    "install it with: cargo install --git https://github.com/Aidoku/aidoku-rs aidoku-cli"
)


# ---------------------------------------------------------------- utilities

def die(msg: str) -> NoReturn:
    print("error: {}".format(msg), file=sys.stderr)
    raise SystemExit(1)


def info(msg: str) -> None:
    print(msg, flush=True)


def rel(path: Path | str) -> str:
    """Path relative to the repo root, for display."""
    try:
        return str(Path(path).resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def build_dir() -> Path:
    """The one directory every workspace member's wasm lands in."""
    target_dir = os.environ.get("CARGO_TARGET_DIR") or (ROOT / "target")
    return Path(target_dir) / TARGET / "release"


def run(cmd: list[str], **kwargs: Any) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(cmd, cwd=str(ROOT), **kwargs)
    if result.returncode != 0:
        raise SystemExit(result.returncode)
    return result


def cargo_fmt(packages: Sequence[str]) -> None:
    """Best effort: a missing rustfmt shouldn't fail an otherwise fine scaffold."""
    if not packages or shutil.which("cargo") is None:
        return
    command = ["cargo", "fmt"]
    for package in packages:
        command += ["-p", package]
    subprocess.run(
        command, cwd=str(ROOT), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )


def require_cli() -> None:
    if shutil.which("aidoku") is None:
        die("the aidoku CLI is not installed; " + CLI_INSTALL_HINT)


def forward(command: str, args: list[str]) -> None:
    """Hand a workspace-agnostic command off to the real CLI."""
    require_cli()
    run(["aidoku", command] + args)


# ------------------------------------------------------------ workspace model

class Member:
    """A source or template crate in the workspace."""

    def __init__(self, path: Path, kind: str) -> None:
        self.path = path
        self.kind = kind
        self.name = read_package_name(path / "Cargo.toml")

    @property
    def dir_name(self) -> str:
        return self.path.name

    @property
    def source_json(self) -> Path:
        return self.path / "res" / "source.json"

    @property
    def package(self) -> Path:
        return self.path / "package.aix"

    def source_id(self) -> str | None:
        key = self.source_key()
        return key[0] if key else None

    def source_key(self) -> tuple[str, Any] | None:
        """(id, version), which is what identifies a built package."""
        try:
            with self.source_json.open(encoding="utf-8") as f:
                info = json.load(f)["info"]
            return (info["id"], info["version"])
        except (OSError, ValueError, KeyError):
            return None

    def __str__(self) -> str:
        return rel(self.path)


def read_package_name(manifest: Path) -> str | None:
    """The `name` of the [package] table, which is what `cargo -p` wants."""
    try:
        text = manifest.read_text(encoding="utf-8")
    except OSError:
        return None
    in_package = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_package = stripped == "[package]"
            continue
        if in_package:
            match = re.match(r'name\s*=\s*"([^"]+)"', stripped)
            if match:
                return match.group(1)
    return None


def members(kind: str | None = None) -> list[Member]:
    found = []
    for group, directory in (("source", ROOT / "sources"), ("template", ROOT / "templates")):
        if kind not in (None, group) or not directory.is_dir():
            continue
        for path in sorted(directory.iterdir()):
            if (path / "Cargo.toml").is_file():
                found.append(Member(path, group))
    return found


def resolve(targets: Sequence[str], kind: str | None = None) -> list[Member]:
    """Look up members by directory path, directory name or package name.

    With no targets, every member of the requested kind is returned, so
    `package` with no arguments packages the whole repo like the old script did.
    """
    pool = members(kind)
    if not targets:
        return pool

    resolved = []
    for target in targets:
        needle = target.rstrip("/")
        candidate = Path(needle)
        if not candidate.is_absolute():
            candidate = Path.cwd() / needle
        matches = [m for m in pool if m.path == candidate.resolve()]
        if not matches:
            matches = [m for m in pool if m.dir_name == needle or m.name == needle]
        if not matches:
            die("no {} matching '{}'".format(kind or "workspace member", target))
        for match in matches:
            if match not in resolved:
                resolved.append(match)
    return resolved


def package_files(targets: Sequence[str]) -> list[str]:
    """The `.aix` paths to hand the real CLI.

    Its build/verify/serve commands only take files, but it's friendlier to
    accept the same source specs the other commands do. With nothing given,
    every source that has been packaged is used.
    """
    if not targets:
        files = sorted(str(m.package) for m in members("source") if m.package.is_file())
        if not files:
            die("no packages found; run `scripts/aidoku.py package` first")
        return files

    files = []
    for target in targets:
        path = Path(target)
        if path.is_file():
            # resolved against the caller's cwd, because the CLI runs in ROOT
            files.append(str(path.resolve()))
            continue
        member = resolve([target], "source")[0]
        if not member.package.is_file():
            die("{} has not been packaged yet; run `scripts/aidoku.py package {}`".format(
                member, target
            ))
        files.append(str(member.package))
    return files


# --------------------------------------------------------------- fingerprints

# Nothing under here feeds the wasm, so it must not feed the fingerprint either.
FINGERPRINT_SKIP_DIRS = {"target", ".git", "__pycache__"}
FINGERPRINT_SKIP_FILES = {"package.aix"}


def toolchain_id() -> str:
    """`rustc --version`, so a compiler bump invalidates every fingerprint."""
    global _TOOLCHAIN_ID
    if _TOOLCHAIN_ID is None:
        _TOOLCHAIN_ID = "unknown"
        if shutil.which("rustc") is not None:
            try:
                result = subprocess.run(
                    ["rustc", "--version"], capture_output=True, text=True, check=True
                )
                _TOOLCHAIN_ID = result.stdout.strip()
            except (OSError, subprocess.CalledProcessError):
                pass
    return _TOOLCHAIN_ID


_TOOLCHAIN_ID: str | None = None


def workspace_paths() -> dict[str, Path]:
    """Dependency name -> member directory, from `[workspace.dependencies]`.

    Members declare templates as `iken.workspace = true`, so the root manifest
    is the only place that says which directory that name points at.
    """
    global _WORKSPACE_PATHS
    if _WORKSPACE_PATHS is None:
        paths: dict[str, Path] = {}
        try:
            with (ROOT / "Cargo.toml").open("rb") as f:
                manifest = tomllib.load(f)
        except (OSError, tomllib.TOMLDecodeError):
            manifest = {}
        workspace = manifest.get("workspace", {})
        for name, spec in workspace.get("dependencies", {}).items():
            if isinstance(spec, dict) and "path" in spec:
                paths[name] = (ROOT / spec["path"]).resolve()
        _WORKSPACE_PATHS = paths
    return _WORKSPACE_PATHS


_WORKSPACE_PATHS: dict[str, Path] | None = None


def load_toml(path: Path) -> dict[str, Any]:
    try:
        with path.open("rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return {}


def dependency_tables(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    """Non-dev dependency tables whose contents can feed the wasm."""
    tables = [manifest.get(key, {}) for key in ("dependencies", "build-dependencies")]
    for spec in manifest.get("target", {}).values():
        for key in ("dependencies", "build-dependencies"):
            tables.append(spec.get(key, {}))
    return [table for table in tables if isinstance(table, dict)]


def dependency_names(path: Path) -> tuple[set[str], set[str]]:
    """(Cargo package names, inherited workspace dependency keys) used by a crate."""
    package_names: set[str] = set()
    workspace_keys: set[str] = set()
    workspace = load_toml(ROOT / "Cargo.toml").get("workspace", {})
    workspace_dependencies = workspace.get("dependencies", {})
    for table in dependency_tables(load_toml(path / "Cargo.toml")):
        for key, spec in table.items():
            resolved = spec
            if isinstance(spec, dict) and spec.get("workspace") is True:
                workspace_keys.add(key)
                resolved = workspace_dependencies.get(key, spec)
            if isinstance(resolved, dict):
                package_names.add(resolved.get("package", key))
            else:
                package_names.add(key)
    return package_names, workspace_keys


def local_dependencies(path: Path) -> list[Path]:
    """The workspace members this crate links against, dev-dependencies aside.

    Tests never end up in the wasm, so a change to one shouldn't force every
    source built on that template to be rebuilt.
    """
    manifest = load_toml(path / "Cargo.toml")

    known = workspace_paths()
    found = []
    for table in dependency_tables(manifest):
        for name, spec in table.items():
            if isinstance(spec, dict) and "path" in spec:
                found.append((path / spec["path"]).resolve())
            elif name in known:
                found.append(known[name])
    return found


def dependency_closure(member: Member) -> list[Path]:
    """Every member directory whose contents can change this one's wasm."""
    seen = {member.path}
    queue = [member.path]
    while queue:
        for dependency in local_dependencies(queue.pop()):
            if dependency not in seen and (dependency / "Cargo.toml").is_file():
                seen.add(dependency)
                queue.append(dependency)
    return sorted(seen)


def hash_tree(digest: "hashlib._Hash", root: Path) -> None:
    """Fold a directory's paths and contents into `digest`, order-independently."""
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d not in FINGERPRINT_SKIP_DIRS)
        base = Path(dirpath)
        for name in sorted(filenames):
            if name in FINGERPRINT_SKIP_FILES or name.endswith(".tmp"):
                continue
            path = base / name
            digest.update(path.relative_to(root).as_posix().encode("utf-8"))
            digest.update(b"\0")
            try:
                # the path is folded in above either way, so a file that can't
                # be read doesn't hash the same as one that isn't there
                digest.update(hashlib.sha256(path.read_bytes()).digest())
            except OSError:
                digest.update(b"<unreadable>")


def canonical_hash(digest: "hashlib._Hash", value: Any) -> None:
    """Hash TOML-derived data without making comments or ordering significant."""
    digest.update(json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8"))
    digest.update(b"\0")


def root_build_config(paths: Sequence[Path]) -> dict[str, Any]:
    """Root settings that affect these crates, excluding unrelated workspace members."""
    manifest = load_toml(ROOT / "Cargo.toml")
    workspace = manifest.get("workspace", {})
    keys: set[str] = set()
    for path in paths:
        keys.update(dependency_names(path)[1])
    dependencies = workspace.get("dependencies", {})
    return {
        "resolver": workspace.get("resolver"),
        "package": workspace.get("package", {}),
        "dependencies": {key: dependencies[key] for key in sorted(keys) if key in dependencies},
        "profile": manifest.get("profile", {}),
        "patch": manifest.get("patch", {}),
        "replace": manifest.get("replace", {}),
    }


def lock_ref(ref: str, packages: Sequence[dict[str, Any]]) -> dict[str, Any] | None:
    """Resolve Cargo.lock's compact dependency reference to its package table."""
    parts = ref.split(" ", 2)
    name = parts[0]
    version = parts[1] if len(parts) > 1 else None
    source = parts[2][1:-1] if len(parts) > 2 and parts[2].startswith("(") else None
    matches = [
        package
        for package in packages
        if package.get("name") == name
        and (version is None or package.get("version") == version)
        and (source is None or package.get("source") == source)
    ]
    return matches[0] if len(matches) == 1 else None


def lock_package_for_member(path: Path, packages: Sequence[dict[str, Any]]) -> dict[str, Any] | None:
    name = read_package_name(path / "Cargo.toml")
    matches = [p for p in packages if p.get("name") == name and "source" not in p]
    return matches[0] if len(matches) == 1 else None


def lock_dependency_closure(paths: Sequence[Path]) -> list[dict[str, Any]] | None:
    """Lockfile package tables reachable through these crates' non-dev dependencies.

    Workspace member tables mix normal and dev dependencies in Cargo.lock. Seed the
    walk from the manifests' non-dev declarations, then follow the exact lockfile
    references. Local crates are hashed from disk and seeded independently.
    """
    lock = load_toml(ROOT / "Cargo.lock")
    packages = lock.get("package")
    if not isinstance(packages, list):
        return None
    local_names = {read_package_name(path / "Cargo.toml") for path in paths}
    queue: list[dict[str, Any]] = []
    for path in paths:
        member_package = lock_package_for_member(path, packages)
        if member_package is None:
            return None
        direct_names = dependency_names(path)[0]
        for ref in member_package.get("dependencies", []):
            package = lock_ref(ref, packages)
            if package is None:
                return None
            if package.get("name") in direct_names and package.get("name") not in local_names:
                queue.append(package)

    seen: set[tuple[Any, Any, Any]] = set()
    result: list[dict[str, Any]] = []
    while queue:
        package = queue.pop()
        identity = (package.get("name"), package.get("version"), package.get("source"))
        if identity in seen:
            continue
        seen.add(identity)
        result.append(package)
        for ref in package.get("dependencies", []):
            dependency = lock_ref(ref, packages)
            if dependency is None:
                return None
            if dependency.get("name") not in local_names:
                queue.append(dependency)
    return sorted(
        result,
        key=lambda p: (p.get("name", ""), p.get("version", ""), p.get("source", "")),
    )


def fingerprint(member: Member) -> str:
    """A hash of everything that decides what this source's .aix contains.

    Its own files, the templates it pulls in, its resolved dependency packages
    and workspace build settings, the toolchain, and this script (which lays the
    .aix out). Equal fingerprints mean a rebuild would produce the same package.
    """
    digest = hashlib.sha256()
    digest.update(toolchain_id().encode("utf-8"))
    digest.update(b"\0")
    paths = dependency_closure(member)
    canonical_hash(digest, root_build_config(paths))
    locked = lock_dependency_closure(paths)
    if locked is None:
        # A malformed or ambiguous lockfile must invalidate safely, not accidentally
        # make two builds appear equivalent.
        try:
            digest.update(hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).digest())
        except OSError:
            digest.update(b"\0")
    shared_inputs = (
        ROOT / ".cargo" / "config.toml",
        Path(__file__).resolve(),
    )
    for shared in shared_inputs:
        try:
            digest.update(hashlib.sha256(shared.read_bytes()).digest())
        except OSError:
            digest.update(b"\0")
    if locked is not None:
        canonical_hash(digest, locked)
    for path in paths:
        digest.update(rel(path).encode("utf-8"))
        digest.update(b"\0")
        hash_tree(digest, path)
    return digest.hexdigest()


def read_manifest(path: Path) -> dict[str, str]:
    try:
        with path.open(encoding="utf-8") as f:
            loaded = json.load(f)
    except (OSError, ValueError):
        return {}
    if not isinstance(loaded, dict):
        return {}
    return {k: v for k, v in loaded.items() if isinstance(v, str)}


def aix_source_key(path: Path) -> tuple[str, Any] | None:
    """The (id, version) a built package declares, read out of the archive."""
    try:
        with zipfile.ZipFile(str(path)) as archive:
            with archive.open("Payload/source.json") as f:
                info = json.loads(f.read().decode("utf-8"))["info"]
        return (info["id"], info["version"])
    except (OSError, ValueError, KeyError, zipfile.BadZipFile):
        return None


class Cache:
    """Packages from a previous build, reusable when the inputs haven't moved.

    CI points this at the deployed source list, so a push only recompiles the
    sources it actually touched; everything else is copied back verbatim.
    """

    def __init__(self, directory: Path) -> None:
        self.directory = directory
        self.fingerprints = read_manifest(directory / MANIFEST_NAME)
        self.by_key: dict[tuple[str, Any], Path] = {}
        if not directory.is_dir():
            info("{} does not exist; building every source".format(rel(directory)))
            return
        for path in sorted(directory.glob("*.aix")):
            # keyed by what the package declares rather than by its filename, so
            # this doesn't have to guess the naming the CLI wrote them out with
            key = aix_source_key(path)
            if key is not None:
                self.by_key[key] = path

    def restore(self, member: Member) -> bool:
        """Copy the cached .aix into place, if it's still the right one."""
        recorded = self.fingerprints.get(member.dir_name)
        if not recorded or recorded != fingerprint(member):
            return False
        # The id *and version* have to match: a manifest that has drifted out of
        # step with the packages beside it must fall back to a rebuild rather
        # than silently ship a package built from a different revision.
        key = member.source_key()
        cached = self.by_key.get(key) if key else None
        if cached is None:
            return False
        # staged and swapped in like write_aix does, so an interrupted copy
        # can't leave a truncated .aix for `build` or `verify` to pick up
        staging = member.package.with_name(member.package.name + ".tmp")
        shutil.copyfile(str(cached), str(staging))
        os.replace(str(staging), str(member.package))
        return True


# -------------------------------------------------------------------- package

def wasm_for(name: str) -> Path | None:
    """cargo writes lib artifacts with hyphens replaced by underscores."""
    for candidate in (name.replace("-", "_"), name):
        path = build_dir() / (candidate + ".wasm")
        if path.is_file():
            return path
    return None


def write_aix(member: Member, wasm: Path, output: Path) -> None:
    """Zip `res/` plus the wasm into the Payload/ layout the app expects."""
    res = member.path / "res"
    entries = [(f, "Payload/" + f.name) for f in sorted(res.iterdir()) if f.is_file()]
    entries.append((wasm, "Payload/main.wasm"))

    # build it beside the target and swap it in, so an interrupted run can't
    # leave a half-written .aix behind for `build` or `verify` to pick up
    staging = output.with_name(output.name + ".tmp")
    with zipfile.ZipFile(str(staging), "w", zipfile.ZIP_DEFLATED) as archive:
        for path, arcname in entries:
            entry = zipfile.ZipInfo.from_file(str(path), arcname)
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = 0o100755 << 16  # regular file, rwxr-xr-x
            with path.open("rb") as f:
                archive.writestr(entry, f.read())
    os.replace(str(staging), str(output))


def cmd_package(args: argparse.Namespace) -> None:
    targets = resolve(args.paths, "source")
    cache = Cache(Path(args.reuse_from).resolve()) if args.reuse_from else None
    reused = 0

    for member in targets:
        if member.name is None:
            die("{}: could not read the package name from Cargo.toml".format(member))
        if not member.source_json.is_file():
            die("{}: res/source.json is missing".format(member))

        if cache is not None and cache.restore(member):
            reused += 1
            info("reused {} -> {}".format(member, rel(member.package)))
            continue

        if not args.skip_build:
            # One package per invocation: `-p a -p b` unifies their feature sets
            # and would link features into a source that never asked for them.
            # The shared target dir still reuses every common dependency build.
            run(
                [
                    "cargo",
                    "build",
                    "--release",
                    "--target",
                    TARGET,
                    "-p",
                    member.name,
                ]
            )

        wasm = wasm_for(member.name)
        if wasm is None:
            die(
                "{}: no {}.wasm in {}{}".format(
                    member,
                    member.name,
                    rel(build_dir()),
                    " (drop --skip-build to build it)" if args.skip_build else "",
                )
            )

        # Picking the wasm by package name is the whole point of this script:
        # `aidoku package` would take whichever one is first in the directory.
        write_aix(member, wasm, member.package)
        info("packaged {} -> {}".format(member, rel(member.package)))

    if not targets:
        info("nothing to package")
    elif cache is not None:
        info("reused {} of {} packages".format(reused, len(targets)))


def cmd_manifest(args: argparse.Namespace) -> None:
    """Record what each source was built from, for the next run to compare against."""
    targets = resolve(args.paths, "source")
    fingerprints = {member.dir_name: fingerprint(member) for member in targets}
    output = Path(args.output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", encoding="utf-8") as f:
        json.dump(fingerprints, f, indent=2, sort_keys=True)
        f.write("\n")
    info("wrote {} fingerprints to {}".format(len(fingerprints), rel(output)))


# -------------------------------------------- build / verify / serve / logcat

def cmd_build(args: argparse.Namespace) -> None:
    files = package_files(args.files)
    output = str(Path(args.output).resolve())
    forward("build", ["--output", output, "--name", args.name] + files)
    info("wrote source list to {}".format(rel(output)))


def cmd_verify(args: argparse.Namespace) -> None:
    files = package_files(args.files)
    forward("verify", files)


def cmd_serve(args: argparse.Namespace) -> None:
    files = package_files(args.files)
    output = str(Path(args.output).resolve())
    forward("serve", ["--output", output, "--port", str(args.port)] + files)


def cmd_logcat(args: argparse.Namespace) -> None:
    forward("logcat", ["--port", str(args.port)])


def cmd_list(args: argparse.Namespace) -> None:
    """Not an upstream command, but the workspace needs a dir -> package map.

    `cargo build -p <name>` and `cargo clippy -p <name>` take the package name,
    which is the source id without its language prefix.
    """
    rows = []
    for member in resolve(args.paths, args.kind):
        rows.append((str(member), member.name or "?", member.source_id() or "-"))
    if not rows:
        return
    widths = [max(len(row[i]) for row in rows) for i in range(3)]
    for row in rows:
        info("  ".join(value.ljust(widths[i]) for i, value in enumerate(row)).rstrip())


# ----------------------------------------------------------------------- init

# ISO 639 codes accepted by the app, copied from the upstream CLI.
LANGUAGE_CODES = set("""
ab aa af ak sq am ar an hy as av ae ay az bm ba eu be bn bi bs br bg my ca ch ce
ny zh cu cv kw co cr hr cs da dv nl dz en eo et ee fo fj fi fr fy ff gd gl lg ka
de el kl gn gu ht ha he hz hi ho hu is io ig id ia ie iu ik ga it ja jv kn kr ks
kk km ki rw ky kv kg ko kj ku lo la lv li ln lt lu lb mk mg ms ml mt gv mi mr mh
mn na nv nd nr ng ne no nb nn oc oj or om os pi ps fa pl pt pa qu ro rm rn ru se
sm sg sa sc sr sn sd si sk sl so st es su sw ss sv tl ty tg ta tt te th bo ti to
ts tn tr tk tw ug uk ur uz ve vi vo wa cy wo xh ii yi yo za zu
""".split()) | {"ceb", "fil", "es-419", "pt-BR", "zh-Hans", "zh-Hant"}

# The upstream list spells this one `pt-br`, but all eight Brazilian sources in
# this repo use `pt-BR`, so accept either spelling and write the repo's.
LANGUAGE_ALIASES = {code.lower(): code for code in LANGUAGE_CODES}


def normalize_language(code: str) -> str:
    return LANGUAGE_ALIASES.get(code.lower(), code)

CONTENT_RATINGS: dict[str, int] = {"safe": 0, "contains-nsfw": 1, "primarily-nsfw": 2}

# A member inherits version, edition and the release profile from the root, so
# its manifest carries nothing but a name, its [lib] kind and its dependencies.
SOURCE_MANIFEST = """[package]
name = "{package}"
version.workspace = true
edition.workspace = true

[lib]
crate-type = ["cdylib"]

[dependencies]
aidoku.workspace = true
{extra_deps}
[dev-dependencies]
aidoku = {{ workspace = true, features = ["test"] }}
aidoku-test.workspace = true
"""

TEMPLATE_MANIFEST = """[package]
name = "{package}"
version.workspace = true
edition.workspace = true

[dependencies]
aidoku.workspace = true

[dev-dependencies]
aidoku = {{ workspace = true, features = ["test"] }}
aidoku-test.workspace = true
"""

# Kept in sync by hand with crates/cli/src/supporting/templates in aidoku-rs.
SOURCE_LIB = '''#![no_std]
use aidoku::{
\tAidokuError, Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeLayout, Listing,
\tListingProvider, Manga, MangaPageResult, Page, Result, Source,
\talloc::{String, Vec},
\tprelude::*,
};

struct {{SOURCE_NAME}};

impl Source for {{SOURCE_NAME}} {
\tfn new() -> Self {
\t\tSelf
\t}

\tfn get_search_manga_list(
\t\t&self,
\t\t_query: Option<String>,
\t\t_page: i32,
\t\t_filters: Vec<FilterValue>,
\t) -> Result<MangaPageResult> {
\t\tErr(AidokuError::Unimplemented)
\t}

\tfn get_manga_update(
\t\t&self,
\t\t_manga: Manga,
\t\t_needs_details: bool,
\t\t_needs_chapters: bool,
\t) -> Result<Manga> {
\t\tErr(AidokuError::Unimplemented)
\t}

\tfn get_page_list(&self, _manga: Manga, _chapter: Chapter) -> Result<Vec<Page>> {
\t\tErr(AidokuError::Unimplemented)
\t}
}

impl ListingProvider for {{SOURCE_NAME}} {
\tfn get_manga_list(&self, _listing: Listing, _page: i32) -> Result<MangaPageResult> {
\t\tErr(AidokuError::Unimplemented)
\t}
}

impl Home for {{SOURCE_NAME}} {
\tfn get_home(&self) -> Result<HomeLayout> {
\t\tErr(AidokuError::Unimplemented)
\t}
}

impl DeepLinkHandler for {{SOURCE_NAME}} {
\tfn handle_deep_link(&self, _url: String) -> Result<Option<DeepLinkResult>> {
\t\tErr(AidokuError::Unimplemented)
\t}
}

register_source!({{SOURCE_NAME}}, ListingProvider, Home, DeepLinkHandler);
'''

TEMPLATE_LIB = '''#![no_std]
use aidoku::{
\tAidokuError, Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeLayout, Listing,
\tListingProvider, Manga, MangaPageResult, Page, Result, Source,
\talloc::{String, Vec},
};

#[derive(Default)]
pub struct Params {}

pub struct {{TEMPLATE_NAME}}<T: Impl> {
\tinner: T,
\tparams: Params,
}

pub trait Impl {
\tfn new() -> Self;

\tfn params(&self) -> Params;

\tfn get_search_manga_list(
\t\t&self,
\t\t_params: &Params,
\t\t_query: Option<String>,
\t\t_page: i32,
\t\t_filters: Vec<FilterValue>,
\t) -> Result<MangaPageResult> {
\t\tErr(AidokuError::Unimplemented)
\t}

\tfn get_manga_update(
\t\t&self,
\t\t_params: &Params,
\t\t_manga: Manga,
\t\t_needs_details: bool,
\t\t_needs_chapters: bool,
\t) -> Result<Manga> {
\t\tErr(AidokuError::Unimplemented)
\t}

\tfn get_page_list(
\t\t&self,
\t\t_params: &Params,
\t\t_manga: Manga,
\t\t_chapter: Chapter,
\t) -> Result<Vec<Page>> {
\t\tErr(AidokuError::Unimplemented)
\t}

\tfn get_manga_list(
\t\t&self,
\t\t_params: &Params,
\t\t_listing: Listing,
\t\t_page: i32,
\t) -> Result<MangaPageResult> {
\t\tErr(AidokuError::Unimplemented)
\t}

\tfn get_home(&self, _params: &Params) -> Result<HomeLayout> {
\t\tErr(AidokuError::Unimplemented)
\t}

\tfn handle_deep_link(&self, _params: &Params, _url: String) -> Result<Option<DeepLinkResult>> {
\t\tErr(AidokuError::Unimplemented)
\t}
}

impl<T: Impl> Source for {{TEMPLATE_NAME}}<T> {
\tfn new() -> Self {
\t\tlet inner = T::new();
\t\tlet params = inner.params();
\t\tSelf { inner, params }
\t}

\tfn get_search_manga_list(
\t\t&self,
\t\tquery: Option<String>,
\t\tpage: i32,
\t\tfilters: Vec<FilterValue>,
\t) -> Result<MangaPageResult> {
\t\tself.inner
\t\t\t.get_search_manga_list(&self.params, query, page, filters)
\t}

\tfn get_manga_update(
\t\t&self,
\t\tmanga: Manga,
\t\tneeds_details: bool,
\t\tneeds_chapters: bool,
\t) -> Result<Manga> {
\t\tself.inner
\t\t\t.get_manga_update(&self.params, manga, needs_details, needs_chapters)
\t}

\tfn get_page_list(&self, manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
\t\tself.inner.get_page_list(&self.params, manga, chapter)
\t}
}

impl<T: Impl> ListingProvider for {{TEMPLATE_NAME}}<T> {
\tfn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
\t\tself.inner.get_manga_list(&self.params, listing, page)
\t}
}

impl<T: Impl> Home for {{TEMPLATE_NAME}}<T> {
\tfn get_home(&self) -> Result<HomeLayout> {
\t\tself.inner.get_home(&self.params)
\t}
}

impl<T: Impl> DeepLinkHandler for {{TEMPLATE_NAME}}<T> {
\tfn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
\t\tself.inner.handle_deep_link(&self.params, url)
\t}
}
'''


class TemplateSurface(NamedTuple):
    """What a template crate offers a source built on it."""

    type_name: str | None
    params_fields: tuple[str, ...]
    params_has_default: bool
    registers: tuple[str, ...]
    overridable: tuple[str, ...]


class TemplateRef(NamedTuple):
    """The template a source is being scaffolded against."""

    crate: str
    lib: str
    type_name: str
    surface: TemplateSurface
    created: bool


def png_chunk(tag: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))


def write_placeholder_icon(path: Path, seed: str) -> None:
    """A 128x128 opaque PNG.

    `aidoku verify` rejects an icon that isn't 128x128 or that has any
    transparency, so the empty file the upstream CLI drops here fails as soon as
    the source is packaged. This one passes, and the colour is derived from the
    source id so a forgotten placeholder still looks obviously wrong.
    """
    size = 128
    digest = zlib.crc32(seed.encode("utf-8"))
    color = bytes(((digest >> 16) & 0xFF, (digest >> 8) & 0xFF, digest & 0xFF))
    # color type 2 (truecolour, no alpha channel) is opaque by construction
    header = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)
    raw = b"".join(b"\x00" + color * size for _ in range(size))
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", header)
        + png_chunk(b"IDAT", zlib.compress(raw, 9))
        + png_chunk(b"IEND", b"")
    )


def package_name_for(name: str) -> str:
    """Source name -> crate name, following the ids already in the repo.

    The upstream CLI turns spaces into hyphens, but no source here has a hyphen
    in it: "Asura Scans" is `asurascans`, and "Danke furs Lesen" keeps its
    accent stripped down to `dankefurslesen`. Match that instead.
    """
    ascii_name = (
        unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode("ascii").lower()
    )
    cleaned = "".join(c for c in ascii_name if c.isalnum() or c == "_").strip("_")
    return "test_source" if cleaned == "test" else cleaned


def type_name_for(name: str) -> str:
    """Source or template name -> Rust type name, accents folded away.

    Each word is capitalized rather than left as typed, so `-t madara` yields
    `Madara`. Only the first letter is touched: `MangaThemesia` and `MMRCMS`
    have to survive intact.
    """
    ascii_name = unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode("ascii")
    words = re.split(r"[^0-9A-Za-z]+", ascii_name)
    return "".join(word[:1].upper() + word[1:] for word in words)


# What `register_source!` should list for a scaffold, in the order sources here
# write them. A template only gets the ones its wrapper actually implements.
CORE_TRAITS: tuple[str, ...] = ("ListingProvider", "Home", "DeepLinkHandler")


def template_sources(path: Path) -> str:
    """Every .rs file in a template crate; `Impl` often lives in imp.rs."""
    try:
        files = sorted((path / "src").rglob("*.rs"))
    except OSError:
        return ""
    texts = []
    for f in files:
        try:
            texts.append(f.read_text(encoding="utf-8"))
        except OSError:
            pass
    return "\n".join(texts)


def braced_block(text: str, pattern: str) -> str | None:
    """The `{ ... }` body of the first item matching `pattern`, brace-matched.

    Returns "" for an item that has no body at all (`pub struct Params;`) and
    None when the item isn't there. Regexes can't do this on their own: a trait
    body is full of nested braces.
    """
    match = re.search(pattern, text, re.MULTILINE)
    if not match:
        return None
    brace = text.find("{", match.end())
    semicolon = text.find(";", match.end())
    if brace < 0 or 0 <= semicolon < brace:
        return ""
    depth = 0
    for i in range(brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[brace + 1 : i]
    return None


def inspect_template(path: Path) -> TemplateSurface:
    """Read a template crate to find out what a source built on it may write.

    Templates are not interchangeable: madara implements ListingProvider, Home
    and DeepLinkHandler while mangaworld implements only DeepLinkHandler, guya's
    `Params` has no `Default` at all, and wpcomics declares `get_home` with a
    different arity than everyone else. Generating one fixed scaffold for all of
    them produces code that doesn't compile.
    """
    text = template_sources(path)

    match = re.search(r"^pub struct (\w+)<T: Impl>", text, re.MULTILINE)
    type_name = match.group(1) if match else None

    params = braced_block(text, r"^pub struct Params\b")
    fields = tuple(re.findall(r"^\tpub (\w+):", params, re.MULTILINE)) if params else ()
    attrs = re.search(r"((?:#\[[^\]]*\]\s*)*)pub struct Params\b", text, re.MULTILINE)
    params_has_default = bool(
        (attrs and "Default" in attrs.group(1))
        or re.search(r"^impl Default for Params\b", text, re.MULTILINE)
    )

    registers = ()
    if type_name:
        implemented = set(
            re.findall(
                r"^impl<T: Impl> (\w+) for " + re.escape(type_name) + r"<T>",
                text,
                re.MULTILINE,
            )
        )
        registers = tuple(trait for trait in CORE_TRAITS if trait in implemented)

    trait_body = braced_block(text, r"^pub trait Impl\b") or ""
    overridable = tuple(
        name
        for name in re.findall(r"^\tfn (\w+)", trait_body, re.MULTILINE)
        if name not in ("new", "params")
    )
    return TemplateSurface(type_name, fields, params_has_default, registers, overridable)


def render_source_template_lib(source_type: str, template: TemplateRef) -> str:
    """Build the source's lib.rs around what the template actually offers.

    Only `new` and `params` are written out. Everything else on the `Impl` trait
    has a default implementation, and their signatures vary between templates,
    so stubbing them blind is how a scaffold ends up not compiling. `init`
    prints the overridable ones instead.
    """
    surface = template.surface
    if not surface.params_has_default:
        # no Default to fall back on, so name the fields that need filling in.
        # one diverging expression, or the fields after it are unreachable code
        hint = ", ".join(surface.params_fields)
        params_literal = 'todo!("set {}")'.format(hint) if hint else "todo!()"
    elif surface.params_fields:
        params_literal = "Params {\n\t\t\t..Default::default()\n\t\t}"
    else:
        # clippy flags `..Default::default()` on a struct with no fields
        params_literal = "Params::default()"

    return (
        "#![no_std]\n"
        "use aidoku::{{Source, prelude::*}};\n"
        "use {lib}::{{{template_imports}}};\n"
        "\n"
        "struct {source};\n"
        "\n"
        "impl Impl for {source} {{\n"
        "\tfn new() -> Self {{\n\t\tSelf\n\t}}\n"
        "\n"
        "\tfn params(&self) -> Params {{\n\t\t{params}\n\t}}\n"
        "}}\n"
        "\n"
        "register_source!({type_name}<{source}>{registers});\n"
    ).format(
        lib=template.lib,
        template_imports=", ".join(sorted(["Impl", "Params", template.type_name])),
        source=source_type,
        params=params_literal,
        type_name=template.type_name,
        registers="".join(", " + trait for trait in surface.registers),
    )


def prompt(
    question: str,
    default: str | None = None,
    validate: Callable[[str], str | None] | None = None,
) -> str:
    if not sys.stdin.isatty():
        die("{} is required (stdin is not a terminal)".format(question.lower()))
    suffix = " [{}]".format(default) if default else ""
    while True:
        answer = input("{}{}: ".format(question, suffix)).strip() or (default or "")
        if not answer:
            continue
        problem = validate(answer) if validate else None
        if problem:
            print(problem, file=sys.stderr)
            continue
        return answer


def check_url(url: str) -> str | None:
    if not url.startswith(("http://", "https://")):
        return "URL must start with http:// or https://"
    return None


def check_languages(languages: Iterable[str]) -> str | None:
    unknown = [lang for lang in languages if lang not in LANGUAGE_CODES]
    if unknown:
        return "not valid ISO 639 language codes: {}".format(", ".join(unknown))
    return None


def add_workspace_dependency(dep_name: str, path: str) -> None:
    """Register a new template in the root [workspace.dependencies] table.

    Sources are picked up by the `sources/*` members glob, but a template only
    becomes usable once members can inherit it from the root manifest.
    """
    manifest = ROOT / "Cargo.toml"
    lines = manifest.read_text(encoding="utf-8").splitlines()
    entry = '{} = {{ path = "{}" }}'.format(dep_name, path)

    try:
        start = lines.index("[workspace.dependencies]") + 1
    except ValueError:
        die("root Cargo.toml has no [workspace.dependencies] table")
    end = start
    while end < len(lines) and not lines[end].startswith("["):
        end += 1

    # keep template entries together in their own block, sorted by name
    block = [i for i in range(start, end) if 'path = "templates/' in lines[i]]
    if block:
        insert_at = next(
            (i for i in block if lines[i].split(" =")[0] > dep_name), block[-1] + 1
        )
    else:
        insert_at = end
        while insert_at > start and not lines[insert_at - 1].strip():
            insert_at -= 1
        lines.insert(insert_at, "")
        lines.insert(insert_at + 1, "# Source templates (multisrc)")
        insert_at += 2

    lines.insert(insert_at, entry)
    manifest.write_text("\n".join(lines) + "\n", encoding="utf-8")
    info("added {} to [workspace.dependencies]".format(dep_name))


def write_source(
    directory: Path,
    package: str,
    source_json: dict[str, Any],
    template: TemplateRef | None = None,
) -> None:
    (directory / "src").mkdir(parents=True)
    (directory / "res").mkdir(parents=True)

    source_type = type_name_for(source_json["info"]["name"])
    if template:
        lib = render_source_template_lib(source_type, template)
        extra_deps = "{}.workspace = true\n".format(template.crate)
    else:
        lib = SOURCE_LIB.replace("{{SOURCE_NAME}}", source_type)
        extra_deps = ""

    (directory / "src" / "lib.rs").write_text(lib, encoding="utf-8")
    (directory / "Cargo.toml").write_text(
        SOURCE_MANIFEST.format(package=package, extra_deps=extra_deps), encoding="utf-8"
    )
    (directory / "res" / "source.json").write_text(
        json.dumps(source_json, indent="\t", ensure_ascii=False) + "\n", encoding="utf-8"
    )
    write_placeholder_icon(directory / "res" / "icon.png", source_json["info"]["id"])


def write_template(directory: Path, package: str, template_type: str) -> None:
    (directory / "src").mkdir(parents=True)
    (directory / "src" / "lib.rs").write_text(
        TEMPLATE_LIB.replace("{{TEMPLATE_NAME}}", template_type), encoding="utf-8"
    )
    (directory / "Cargo.toml").write_text(
        TEMPLATE_MANIFEST.format(package=package), encoding="utf-8"
    )


def cmd_init(args: argparse.Namespace) -> None:
    existing = members()

    name = args.name or prompt("Source name")
    url = args.url or prompt("Source URL", validate=check_url)
    problem = check_url(url)
    if problem:
        die(problem)
    languages = [lang for arg in args.languages for lang in arg.split()] or prompt(
        "Languages (e.g. `en`, or `id pt ja`)",
        validate=lambda answer: check_languages(answer.split()),
    ).split()
    languages = [normalize_language(lang) for lang in languages]
    problem = check_languages(languages)
    if problem:
        die(problem)
    rating = args.content_rating or prompt(
        "Content rating ({})".format("/".join(CONTENT_RATINGS)),
        default="safe",
        validate=lambda r: None if r in CONTENT_RATINGS else "unknown content rating",
    )

    package = package_name_for(name)
    if not package:
        die("could not derive a crate name from '{}'".format(name))
    # a regional code keeps only its base subtag in the id: the two pt-BR
    # sources here are pt.flowermanga and pt.mangalivre
    prefix = languages[0].split("-")[0] if len(languages) == 1 else "multi"
    source_id = "{}.{}".format(prefix, package)

    directory = Path(args.path).resolve() if args.path else ROOT / "sources" / source_id
    if directory.parent != ROOT / "sources":
        # the workspace members globs are `sources/*` and `templates/*`; a crate
        # anywhere else can't inherit `version`/`edition` and won't even parse
        die("{} is not directly under sources/, so it can't be a workspace member".format(
            rel(directory)
        ))
    if directory.exists():
        die("{} already exists".format(rel(directory)))
    clash = next((m for m in existing if m.name == package), None)
    if clash:
        die(
            "the crate name '{}' is already taken by {}; pass a different --name".format(
                package, clash
            )
        )

    template = None
    if args.template or args.template_name:
        template = init_template(args.template_name, existing, package)

    source_json = {
        "info": {
            "id": source_id,
            "name": name,
            "version": 1,
            "url": url,
            "contentRating": CONTENT_RATINGS[rating],
            "languages": languages,
        }
    }
    try:
        write_source(directory, package, source_json, template)
    except Exception:
        shutil.rmtree(directory, ignore_errors=True)
        raise

    # a long `register_source!` line needs wrapping, so let rustfmt have a pass
    # rather than shipping a scaffold that fails `cargo fmt --check`
    formatted = [package] + ([template.crate] if template and template.created else [])
    cargo_fmt(formatted)

    info("created {} ({})".format(rel(directory), package))
    info("")
    info("next steps:")
    info("  * replace the placeholder res/icon.png with a real 128x128 opaque icon")
    if template and template.surface.overridable:
        info(
            "  * {} can also override: {}".format(
                package, ", ".join(template.surface.overridable)
            )
        )
    info("  * cargo clippy -p {}".format(package))
    info("  * scripts/aidoku.py package {}".format(rel(directory)))


def init_template(
    template_name: str | None, existing: list[Member], source_package: str
) -> TemplateRef:
    """Create the template crate, or reuse one the workspace already has."""
    template_name = template_name or prompt("Template name")
    base = package_name_for(template_name)
    type_name = type_name_for(template_name)
    if not base or not type_name:
        die("could not derive a crate name from '{}'".format(template_name))

    directory = ROOT / "templates" / base
    reused = next(
        (m for m in existing if m.kind == "template" and (m.path == directory or m.name == base)),
        None,
    )
    if reused:
        if not reused.name:
            die("{} has no [package] name in its Cargo.toml".format(rel(reused.path)))
        info("reusing the existing {} template".format(rel(reused.path)))
        # read the crate rather than trusting how the argument was spelled: the
        # wrapper type can differ from the crate name entirely
        surface = inspect_template(reused.path)
        return TemplateRef(
            crate=reused.name,
            lib=reused.name.replace("-", "_"),
            type_name=surface.type_name or type_name,
            surface=surface,
            created=False,
        )

    if directory.exists():
        die("{} already exists".format(rel(directory)))

    # No two crates in a workspace may share a name. templates/mangaworld ships
    # as `mangaworld_template` because a source already claimed `mangaworld`;
    # follow that precedent rather than refusing to scaffold.
    taken = {m.name for m in existing} | {source_package}
    package = base if base not in taken else "{}_template".format(base)
    if package in taken:
        die("the crate name '{}' is already taken; pass a different --template-name".format(package))

    write_template(directory, package, type_name)
    add_workspace_dependency(package, "templates/{}".format(base))
    info("created {} ({})".format(rel(directory), package))

    # read back what was just written, so the source is generated from the same
    # inspection the reuse path goes through
    surface = inspect_template(directory)
    return TemplateRef(
        crate=package,
        lib=package.replace("-", "_"),
        type_name=surface.type_name or type_name,
        surface=surface,
        created=True,
    )


# ------------------------------------------------------------------ argparser

def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(
        prog="scripts/aidoku.py",
        description=__doc__.split("\n\n")[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Sources can be given as a path (sources/en.mysource), a directory\n"
            "name (en.mysource) or a crate name (mysource). With none given,\n"
            "every source in the workspace is used."
        ),
    )
    commands = root.add_subparsers(dest="command", metavar="<command>")
    commands.required = True

    package = commands.add_parser("package", help="build and package sources")
    package.add_argument("paths", nargs="*", metavar="SOURCE")
    package.add_argument(
        "--skip-build", action="store_true", help="package the wasm already in target/"
    )
    package.add_argument(
        "--reuse-from",
        metavar="DIR",
        help="directory of previously built .aix files (with a {}) to copy "
        "unchanged sources from instead of rebuilding them".format(MANIFEST_NAME),
    )
    package.set_defaults(func=cmd_package)

    manifest = commands.add_parser(
        "manifest", help="record build fingerprints for a later --reuse-from"
    )
    manifest.add_argument("paths", nargs="*", metavar="SOURCE")
    manifest.add_argument(
        "-o", "--output", default=str(ROOT / "public" / "sources" / MANIFEST_NAME),
        help="where to write the manifest",
    )
    manifest.set_defaults(func=cmd_manifest)

    build = commands.add_parser("build", help="build a source list from packaged sources")
    build.add_argument("files", nargs="*", metavar="FILE")
    build.add_argument("-o", "--output", default=str(ROOT / "public"), help="output folder path")
    build.add_argument("-n", "--name", default=SOURCE_LIST_NAME, help="source list name")
    build.set_defaults(func=cmd_build)

    verify = commands.add_parser("verify", help="verify packaged sources")
    verify.add_argument("files", nargs="*", metavar="FILE")
    verify.set_defaults(func=cmd_verify)

    serve = commands.add_parser("serve", help="build a source list and serve it")
    serve.add_argument("files", nargs="*", metavar="FILE")
    serve.add_argument("-o", "--output", default=str(ROOT / "public"), help="output folder path")
    serve.add_argument("-p", "--port", type=int, default=8080, help="port to serve on")
    serve.set_defaults(func=cmd_serve)

    logcat = commands.add_parser("logcat", help="open a server for log streaming")
    logcat.add_argument("-p", "--port", type=int, default=9000, help="port to listen on")
    logcat.set_defaults(func=cmd_logcat)

    init = commands.add_parser("init", help="scaffold a new source or template")
    init.add_argument("path", nargs="?", help="defaults to sources/<source id>")
    init.add_argument("-n", "--name", help="source name")
    init.add_argument("-u", "--url", help="source homepage url")
    init.add_argument(
        "-l", "--languages", action="append", default=[], metavar="LANG", help="source languages"
    )
    init.add_argument("-c", "--content-rating", choices=sorted(CONTENT_RATINGS))
    init.add_argument("--template", action="store_true", help="also scaffold a source template")
    init.add_argument("-t", "--template-name", help="template to create, or an existing one to use")
    init.set_defaults(func=cmd_init)

    listing = commands.add_parser("list", help="list workspace members and their crate names")
    listing.add_argument("paths", nargs="*", metavar="MEMBER")
    listing.add_argument(
        "-k", "--kind", choices=("source", "template"), help="limit to one kind of member"
    )
    listing.set_defaults(func=cmd_list)

    return root


def main() -> None:
    args = parser().parse_args()
    try:
        args.func(args)
    except KeyboardInterrupt:
        raise SystemExit(130)


if __name__ == "__main__":
    main()
