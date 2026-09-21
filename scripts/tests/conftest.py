"""Small Cargo workspaces for testing the existing aidoku command interface."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

from aidoku_workspace.workspace import Workspace


@pytest.fixture
def aidoku(tmp_path: Path):
    """A minimal independent workspace for each test."""
    workspace = Workspace(tmp_path)
    workspace._toolchain_id = "rustc test"
    (tmp_path / "sources").mkdir()
    (tmp_path / "templates").mkdir()
    (tmp_path / "Cargo.toml").write_text(
        '[workspace]\nresolver = "2"\nmembers = ["sources/*", "templates/*"]\n'
        '[workspace.metadata.aidoku]\nmin-app-version = "0.8.5"\n'
        '[workspace.dependencies]\naidoku = { version = "1" }\n'
    )
    (tmp_path / "Cargo.lock").write_text("version = 4\n")
    return workspace


@pytest.fixture
def source(aidoku):
    def create(directory: str = "en.example", crate: str = "example", source_id: str = "en.example", version: int = 3):
        path = aidoku.root / "sources" / directory
        (path / "src").mkdir(parents=True)
        (path / "res").mkdir()
        (path / "Cargo.toml").write_text(
            f'[package]\nname = "{crate}"\nversion.workspace = true\n[dependencies]\naidoku.workspace = true\n'
        )
        (path / "src" / "lib.rs").write_text("#![no_std]\n")
        (path / "res" / "source.json").write_text(
            json.dumps({"info": {"id": source_id, "version": version, "minAppVersion": "0.8.5"}}) + "\n"
        )
        (path / "res" / "icon.png").write_bytes(b"icon")
        return path

    return create
