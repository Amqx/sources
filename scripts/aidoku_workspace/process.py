"""Workspace-rooted subprocess and upstream CLI operations."""

from __future__ import annotations

import shutil
import subprocess
from collections.abc import Sequence
from typing import Any

from .common import die
from .workspace import Workspace

CLI_INSTALL_HINT = "install it with: cargo install --git https://github.com/Amqx/aidoku-rs aidoku-cli"


def run(workspace: Workspace, command: list[str], **kwargs: Any) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(command, check=False, cwd=str(workspace.root), **kwargs)
    if result.returncode != 0:
        raise SystemExit(result.returncode)
    return result


def cargo_fmt(workspace: Workspace, packages: Sequence[str]) -> None:
    if not packages or shutil.which("cargo") is None:
        return
    subprocess.run(
        ["cargo", "fmt", *[argument for package in packages for argument in ("-p", package)]],
        check=False,
        cwd=str(workspace.root),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def require_cli() -> None:
    if shutil.which("aidoku") is None:
        die("the aidoku CLI is not installed; " + CLI_INSTALL_HINT)


def forward(workspace: Workspace, command: str, args: list[str]) -> None:
    require_cli()
    run(workspace, ["aidoku", command, *args])
