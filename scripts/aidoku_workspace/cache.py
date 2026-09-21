from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import zipfile
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from .common import info
from .workspace import Member, Workspace, load_toml, read_package_name

MANIFEST_NAME = "build-manifest.json"
FINGERPRINT_SKIP_DIRS = {"target", ".git", "__pycache__"}
FINGERPRINT_SKIP_FILES = {"package.aix"}
TOOLING_SKIP_DIRS = {"__pycache__", ".pytest_cache"}
TOOLING_SKIP_SUFFIXES = {".pyc", ".pyo", ".tmp", ".swp"}


def toolchain_id(workspace: Workspace) -> str:
    if workspace._toolchain_id is None:
        workspace._toolchain_id = "unknown"
        if shutil.which("rustc"):
            try:
                workspace._toolchain_id = subprocess.run(
                    ["rustc", "--version"], capture_output=True, text=True, check=True
                ).stdout.strip()
            except OSError, subprocess.CalledProcessError:
                pass
    return workspace._toolchain_id


def dependency_tables(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    tables = [manifest.get(key, {}) for key in ("dependencies", "build-dependencies")]
    for spec in manifest.get("target", {}).values():
        for key in ("dependencies", "build-dependencies"):
            tables.append(spec.get(key, {}))
    return [table for table in tables if isinstance(table, dict)]


def dependency_names(workspace: Workspace, path: Path) -> tuple[set[str], set[str]]:
    package_names: set[str] = set()
    workspace_keys: set[str] = set()
    workspace_dependencies = load_toml(workspace.root / "Cargo.toml").get("workspace", {}).get("dependencies", {})
    for table in dependency_tables(load_toml(path / "Cargo.toml")):
        for key, spec in table.items():
            resolved = spec
            if isinstance(spec, dict) and spec.get("workspace") is True:
                workspace_keys.add(key)
                resolved = workspace_dependencies.get(key, spec)
            package_names.add(resolved.get("package", key) if isinstance(resolved, dict) else key)
    return package_names, workspace_keys


def local_dependencies(workspace: Workspace, path: Path) -> list[Path]:
    found: list[Path] = []
    for table in dependency_tables(load_toml(path / "Cargo.toml")):
        for name, spec in table.items():
            if isinstance(spec, dict) and "path" in spec:
                found.append((path / spec["path"]).resolve())
            elif name in workspace.workspace_paths():
                found.append(workspace.workspace_paths()[name])
    return found


def dependency_closure(workspace: Workspace, member: Member) -> list[Path]:
    seen = {member.path}
    queue = [member.path]
    while queue:
        for dependency in local_dependencies(workspace, queue.pop()):
            if dependency not in seen and (dependency / "Cargo.toml").is_file():
                seen.add(dependency)
                queue.append(dependency)
    return sorted(seen)


def hash_tree(digest: Any, root: Path) -> None:
    for directory, directories, filenames in os.walk(root):
        directories[:] = sorted(item for item in directories if item not in FINGERPRINT_SKIP_DIRS)
        for name in sorted(filenames):
            if name in FINGERPRINT_SKIP_FILES or name.endswith(".tmp"):
                continue
            path = Path(directory) / name
            digest.update(path.relative_to(root).as_posix().encode())
            digest.update(b"\0")
            try:
                digest.update(hashlib.sha256(path.read_bytes()).digest())
            except OSError:
                digest.update(b"<unreadable>")


def canonical_hash(digest: Any, value: Any) -> None:
    digest.update(json.dumps(value, sort_keys=True, separators=(",", ":")).encode())
    digest.update(b"\0")


def root_build_config(workspace: Workspace, paths: Sequence[Path]) -> dict[str, Any]:
    manifest = load_toml(workspace.root / "Cargo.toml")
    root_workspace = manifest.get("workspace", {})
    keys: set[str] = set()
    for path in paths:
        keys.update(dependency_names(workspace, path)[1])
    dependencies = root_workspace.get("dependencies", {})
    return {
        "resolver": root_workspace.get("resolver"),
        "package": root_workspace.get("package", {}),
        "dependencies": {key: dependencies[key] for key in sorted(keys) if key in dependencies},
        "profile": manifest.get("profile", {}),
        "patch": manifest.get("patch", {}),
        "replace": manifest.get("replace", {}),
    }


def lock_ref(ref: str, packages: Sequence[dict[str, Any]]) -> dict[str, Any] | None:
    parts = ref.split(" ", 2)
    name, version = parts[0], parts[1] if len(parts) > 1 else None
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
    matches = [
        package
        for package in packages
        if package.get("name") == read_package_name(path / "Cargo.toml") and "source" not in package
    ]
    return matches[0] if len(matches) == 1 else None


def lock_dependency_closure(workspace: Workspace, paths: Sequence[Path]) -> list[dict[str, Any]] | None:
    packages = load_toml(workspace.root / "Cargo.lock").get("package")
    if not isinstance(packages, list):
        return None
    local_names = {read_package_name(path / "Cargo.toml") for path in paths}
    queue: list[dict[str, Any]] = []
    for path in paths:
        member_package = lock_package_for_member(path, packages)
        if member_package is None:
            return None
        for ref in member_package.get("dependencies", []):
            package = lock_ref(ref, packages)
            if package is None:
                return None
            if package.get("name") in dependency_names(workspace, path)[0] and package.get("name") not in local_names:
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
        result, key=lambda package: (package.get("name", ""), package.get("version", ""), package.get("source", ""))
    )


def hash_tooling(digest: Any, workspace: Workspace) -> None:
    """Hash the startup wrapper and package code/assets in a stable order."""
    paths = [workspace.root / "scripts" / "aidoku.py"]
    package = workspace.root / "scripts" / "aidoku_workspace"
    paths.extend(
        path
        for path in package.rglob("*")
        if path.is_file()
        and not any(part in TOOLING_SKIP_DIRS for part in path.relative_to(package).parts)
        and path.suffix not in TOOLING_SKIP_SUFFIXES
        and not path.name.endswith("~")
        and not path.name.startswith(".DS_Store")
    )
    for path in sorted(paths, key=lambda item: item.relative_to(workspace.root).as_posix()):
        digest.update(path.relative_to(workspace.root).as_posix().encode())
        digest.update(b"\0")
        try:
            digest.update(hashlib.sha256(path.read_bytes()).digest())
        except OSError:
            digest.update(b"<missing>")


def python_runtime_packages(workspace: Workspace) -> list[dict[str, Any]] | None:
    """The root project's locked runtime closure, excluding dev dependencies."""
    project = load_toml(workspace.root / "pyproject.toml").get("project", {})
    packages = load_toml(workspace.root / "uv.lock").get("package")
    if not isinstance(packages, list):
        return None
    roots = [
        package
        for package in packages
        if package.get("name") == project.get("name") and package.get("source", {}).get("virtual") == "."
    ]
    if len(roots) != 1:
        return None
    by_name: dict[str, list[dict[str, Any]]] = {}
    for package in packages:
        by_name.setdefault(package.get("name", ""), []).append(package)
    queue = list(roots[0].get("dependencies", []))
    seen: set[tuple[str, str, str]] = set()
    result: list[dict[str, Any]] = []
    while queue:
        reference = queue.pop()
        matches = [
            package
            for package in by_name.get(reference.get("name", ""), [])
            if ("version" not in reference or package.get("version") == reference["version"])
            and ("source" not in reference or package.get("source") == reference["source"])
        ]
        if len(matches) != 1:
            return None
        package = matches[0]
        identity = (package.get("name", ""), package.get("version", ""), str(package.get("source", "")))
        if identity in seen:
            continue
        seen.add(identity)
        result.append(package)
        queue.extend(package.get("dependencies", []))
    return sorted(
        result,
        key=lambda package: (
            package.get("name", ""),
            package.get("version", ""),
            str(package.get("source", "")),
        ),
    )


def hash_python_runtime(digest: Any, workspace: Workspace) -> None:
    """Hash the project's full runtime closure, including other scripts' deps.

    This intentionally invalidates more than Aidoku's current stdlib-only code
    needs, while changes to dev-only packages leave reusable archives intact.
    """
    project = load_toml(workspace.root / "pyproject.toml").get("project", {})
    canonical_hash(
        digest,
        {
            "requires-python": project.get("requires-python"),
            "dependencies": project.get("dependencies", []),
        },
    )
    packages = python_runtime_packages(workspace)
    if packages is None:
        # Missing or ambiguous lock data must not allow an unsafe cache hit.
        path = workspace.root / "uv.lock"
        digest.update(hashlib.sha256(path.read_bytes()).digest() if path.is_file() else b"<missing uv.lock>")
    else:
        canonical_hash(digest, packages)
    path = workspace.root / ".python-version"
    digest.update(hashlib.sha256(path.read_bytes()).digest() if path.is_file() else b"<missing .python-version>")


def fingerprint(workspace: Workspace, member: Member) -> str:
    digest = hashlib.sha256()
    digest.update(toolchain_id(workspace).encode())
    digest.update(b"\0")
    paths = dependency_closure(workspace, member)
    canonical_hash(digest, root_build_config(workspace, paths))
    locked = lock_dependency_closure(workspace, paths)
    if locked is None:
        try:
            digest.update(hashlib.sha256((workspace.root / "Cargo.lock").read_bytes()).digest())
        except OSError:
            digest.update(b"\0")
    try:
        digest.update(hashlib.sha256((workspace.root / ".cargo" / "config.toml").read_bytes()).digest())
    except OSError:
        digest.update(b"\0")
    hash_tooling(digest, workspace)
    hash_python_runtime(digest, workspace)
    if locked is not None:
        canonical_hash(digest, locked)
    for path in paths:
        digest.update(workspace.rel(path).encode())
        digest.update(b"\0")
        hash_tree(digest, path)
    return digest.hexdigest()


def read_manifest(path: Path) -> dict[str, str]:
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except OSError, ValueError:
        return {}
    return {key: value for key, value in loaded.items() if isinstance(value, str)} if isinstance(loaded, dict) else {}


def aix_source_key(path: Path) -> tuple[str, Any] | None:
    try:
        with zipfile.ZipFile(path) as archive, archive.open("Payload/source.json") as file:
            info = json.loads(file.read().decode())["info"]
        return (info["id"], info["version"])
    except OSError, ValueError, KeyError, zipfile.BadZipFile:
        return None


class Cache:
    def __init__(self, workspace: Workspace, directory: Path) -> None:
        self.workspace = workspace
        self.directory = directory
        self.fingerprints = read_manifest(directory / MANIFEST_NAME)
        self.by_key: dict[tuple[str, Any], Path] = {}
        if not directory.is_dir():
            info(f"{workspace.rel(directory)} does not exist; building every source")
            return
        for path in sorted(directory.glob("*.aix")):
            if (key := aix_source_key(path)) is not None:
                self.by_key[key] = path

    def restore(self, member: Member) -> bool:
        if self.fingerprints.get(member.dir_name) != fingerprint(self.workspace, member):
            return False
        cached = self.by_key.get(member.source_key()) if member.source_key() else None
        if cached is None:
            return False
        staging = member.package.with_name(member.package.name + ".tmp")
        shutil.copyfile(cached, staging)
        os.replace(staging, member.package)
        return True


def write_manifest(workspace: Workspace, targets: Sequence[Member], output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps({member.dir_name: fingerprint(workspace, member) for member in targets}, indent=2, sort_keys=True)
        + "\n",
        encoding="utf-8",
    )
    info(f"wrote {len(targets)} fingerprints to {workspace.rel(output)}")
