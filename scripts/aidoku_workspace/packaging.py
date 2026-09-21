from __future__ import annotations

import os
import zipfile
from pathlib import Path

from . import process
from .common import die, info
from .workspace import TARGET, Member, Workspace


def wasm_for(workspace: Workspace, name: str) -> Path | None:
    """Find the wasm that Cargo writes for this package only."""
    for candidate in (name.replace("-", "_"), name):
        path = workspace.build_dir() / (candidate + ".wasm")
        if path.is_file():
            return path
    return None


def write_aix(member: Member, wasm: Path, output: Path) -> None:
    """Atomically zip a member's resources and wasm in Aidoku's Payload layout."""
    entries = [(path, "Payload/" + path.name) for path in sorted((member.path / "res").iterdir()) if path.is_file()]
    entries.append((wasm, "Payload/main.wasm"))
    staging = output.with_name(output.name + ".tmp")
    with zipfile.ZipFile(staging, "w", zipfile.ZIP_DEFLATED) as archive:
        for path, arcname in entries:
            entry = zipfile.ZipInfo.from_file(path, arcname)
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = 0o100755 << 16
            archive.writestr(entry, path.read_bytes())
    os.replace(staging, output)


def validate_member(member: Member) -> str:
    """Validate a source before either restoring or freshly packaging it."""
    if member.name is None:
        die(f"{member}: could not read the package name from Cargo.toml")
    if not member.source_json.is_file():
        die(f"{member}: res/source.json is missing")
    return member.name


def package_member(workspace: Workspace, member: Member, *, skip_build: bool = False) -> None:
    """Build and archive one member; cache policy belongs to the CLI."""
    name = validate_member(member)
    if not skip_build:
        process.run(
            workspace,
            ["cargo", "build", "--release", "--target", TARGET, "-p", name],
        )
    wasm = wasm_for(workspace, name)
    if wasm is None:
        die(
            "{}: no {}.wasm in {}{}".format(
                member,
                name,
                workspace.rel(workspace.build_dir()),
                " (drop --skip-build to build it)" if skip_build else "",
            )
        )
    write_aix(member, wasm, member.package)
    info(f"packaged {member} -> {workspace.rel(member.package)}")
