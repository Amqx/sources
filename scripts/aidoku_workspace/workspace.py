from __future__ import annotations

import json
import os
import tomllib
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from .common import die

TARGET = "wasm32-unknown-unknown"


def read_package_name(manifest: Path) -> str | None:
    name = load_toml(manifest).get("package", {}).get("name")
    return name if isinstance(name, str) else None


def load_toml(path: Path) -> dict[str, Any]:
    try:
        with path.open("rb") as file:
            return tomllib.load(file)
    except OSError, tomllib.TOMLDecodeError:
        return {}


class Member:
    """A source or template crate belonging to one Workspace."""

    def __init__(self, workspace: Workspace, path: Path, kind: str) -> None:
        self.workspace = workspace
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

    def source_key(self) -> tuple[str, Any] | None:
        try:
            with self.source_json.open(encoding="utf-8") as file:
                info = json.load(file)["info"]
            return (info["id"], info["version"])
        except OSError, ValueError, KeyError:
            return None

    def source_id(self) -> str | None:
        key = self.source_key()
        return key[0] if key else None

    def __str__(self) -> str:
        return self.workspace.rel(self.path)


class Workspace:
    """All state for one workspace root."""

    def __init__(self, root: Path) -> None:
        self.root = root.resolve()
        self._workspace_paths: dict[str, Path] | None = None
        self._toolchain_id: str | None = None

    def rel(self, path: Path | str) -> str:
        try:
            return str(Path(path).resolve().relative_to(self.root))
        except ValueError:
            return str(path)

    def build_dir(self) -> Path:
        target_dir = os.environ.get("CARGO_TARGET_DIR") or (self.root / "target")
        return Path(target_dir) / TARGET / "release"

    def members(self, kind: str | None = None) -> list[Member]:
        found: list[Member] = []
        for group, directory in (
            ("source", self.root / "sources"),
            ("template", self.root / "templates"),
        ):
            if kind not in (None, group) or not directory.is_dir():
                continue
            for path in sorted(directory.iterdir()):
                if (path / "Cargo.toml").is_file():
                    found.append(Member(self, path, group))
        return found

    def resolve(self, targets: Sequence[str], kind: str | None = None) -> list[Member]:
        pool = self.members(kind)
        if not targets:
            return pool
        resolved: list[Member] = []
        for target in targets:
            needle = target.rstrip("/")
            candidate = Path(needle)
            if not candidate.is_absolute():
                candidate = Path.cwd() / needle
            matches = [member for member in pool if member.path == candidate.resolve()]
            if not matches:
                matches = [member for member in pool if member.dir_name == needle or member.name == needle]
            if not matches:
                die("no {} matching '{}'".format(kind or "workspace member", target))
            for member in matches:
                if member not in resolved:
                    resolved.append(member)
        return resolved

    def package_files(self, targets: Sequence[str]) -> list[str]:
        if not targets:
            files = sorted(str(member.package) for member in self.members("source") if member.package.is_file())
            if not files:
                die("no packages found; run `scripts/aidoku.py package` first")
            return files
        files: list[str] = []
        for target in targets:
            path = Path(target)
            if path.is_file():
                files.append(str(path.resolve()))
                continue
            member = self.resolve([target], "source")[0]
            if not member.package.is_file():
                die(f"{member} has not been packaged yet; run `scripts/aidoku.py package {target}`")
            files.append(str(member.package))
        return files

    def workspace_paths(self) -> dict[str, Path]:
        if self._workspace_paths is None:
            paths: dict[str, Path] = {}
            for name, spec in load_toml(self.root / "Cargo.toml").get("workspace", {}).get("dependencies", {}).items():
                if isinstance(spec, dict) and "path" in spec:
                    paths[name] = (self.root / spec["path"]).resolve()
            self._workspace_paths = paths
        return self._workspace_paths
