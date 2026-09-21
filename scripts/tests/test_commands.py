from __future__ import annotations

import json
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

import pytest
from aidoku_workspace import cli, packaging, process
from aidoku_workspace.workspace import TARGET, Workspace
from PIL import Image


def test_resolve_path_directory_and_crate_with_distinct_source_id(aidoku, source, monkeypatch):
    path = source("ja.nicovideoseiga", "nicovideoseiga", "ja.seiga")
    monkeypatch.chdir(aidoku.root)
    for target in (str(path), "sources/ja.nicovideoseiga", "ja.nicovideoseiga", "nicovideoseiga"):
        assert aidoku.resolve([target], "source")[0].source_key() == ("ja.seiga", 3)
    assert len(aidoku.resolve(["nicovideoseiga", "ja.nicovideoseiga"], "source")) == 1
    with pytest.raises(SystemExit) as error:
        aidoku.resolve(["ja.seiga"], "source")
    assert error.value.code == 1


def test_workspace_contexts_discover_members_without_cross_contamination(aidoku, source, tmp_path):
    """Member discovery and dependency caches belong to each supplied root."""
    source("en.left", "left", "en.left")
    template = aidoku.root / "templates" / "fixture"
    template.mkdir()
    template.joinpath("Cargo.toml").write_text('[package]\nname = "fixture"\n')
    manifest = aidoku.root / "Cargo.toml"
    manifest.write_text(manifest.read_text() + 'fixture = { path = "templates/fixture" }\n')
    other_root = tmp_path / "other-workspace"
    shutil.copytree(aidoku.root, other_root)
    other = Workspace(other_root)

    assert aidoku.resolve(["left"], "source")[0].path == aidoku.root / "sources/en.left"
    assert other.resolve(["left"], "source")[0].path == other.root / "sources/en.left"
    assert aidoku.workspace_paths()["fixture"] == aidoku.root / "templates/fixture"
    assert other.workspace_paths()["fixture"] == other.root / "templates/fixture"


def test_package_builds_crates_separately_and_selects_named_wasm(aidoku, source, monkeypatch):
    first = source("en.first", "first", "en.first")
    second = source("en.second", "second", "en.second")
    build = aidoku.build_dir()
    build.mkdir(parents=True)
    (build / "first.wasm").write_bytes(b"FIRST")
    (build / "second.wasm").write_bytes(b"SECOND")
    (build / "unrelated.wasm").write_bytes(b"WRONG")
    calls = []
    monkeypatch.setattr(process, "run", lambda workspace, command: calls.append(command))

    cli.main(["package", "first", "second"], workspace=aidoku)

    assert calls == [
        ["cargo", "build", "--release", "--target", TARGET, "-p", "first"],
        ["cargo", "build", "--release", "--target", TARGET, "-p", "second"],
    ]
    for path, expected in ((first, b"FIRST"), (second, b"SECOND")):
        with zipfile.ZipFile(path / "package.aix") as archive:
            assert sorted(archive.namelist()) == ["Payload/icon.png", "Payload/main.wasm", "Payload/source.json"]
            assert archive.read("Payload/main.wasm") == expected
            assert json.loads(archive.read("Payload/source.json"))["info"]["id"] == path.name
        assert not (path / "package.aix.tmp").exists()


def test_package_replaces_archive_only_after_staging_is_complete(aidoku, source, monkeypatch):
    path = source()
    output = path / "package.aix"
    output.write_bytes(b"old archive")
    wasm = aidoku.root / "source.wasm"
    wasm.write_bytes(b"new wasm")
    original_replace = packaging.os.replace

    def inspect_swap(staging, destination):
        assert Path(destination).read_bytes() == b"old archive"
        with zipfile.ZipFile(staging) as archive:
            assert archive.read("Payload/main.wasm") == b"new wasm"
        original_replace(staging, destination)

    monkeypatch.setattr(packaging.os, "replace", inspect_swap)
    packaging.write_aix(aidoku.resolve(["example"], "source")[0], wasm, output)
    assert zipfile.is_zipfile(output)


def test_command_defaults_forwarding_and_exit_behavior(aidoku, source, monkeypatch, capsys):
    path = source()
    path.joinpath("package.aix").write_bytes(b"archive")
    parser = cli.parser(aidoku)
    assert parser.parse_args(["package"]).paths == []
    assert parser.parse_args(["package"]).skip_build is False
    assert parser.parse_args(["build"]).name == "Amqx's Sources"
    assert parser.parse_args(["serve"]).port == 8080
    assert parser.parse_args(["logcat"]).port == 9000
    assert parser.parse_args(["manifest"]).output == str(aidoku.root / "public/sources/build-manifest.json")
    commands = []
    monkeypatch.setattr(process, "forward", lambda workspace, command, args: commands.append((command, args)))
    for argv in (["verify", "example"], ["build", "example"], ["serve", "example"], ["logcat"]):
        cli.main(argv, workspace=aidoku)
    package = str(path / "package.aix")
    assert commands == [
        ("verify", [package]),
        ("build", ["--output", str(aidoku.root / "public"), "--name", "Amqx's Sources", package]),
        ("serve", ["--output", str(aidoku.root / "public"), "--port", "8080", package]),
        ("logcat", ["--port", "9000"]),
    ]
    with pytest.raises(SystemExit) as error:
        parser.parse_args([])
    assert error.value.code == 2
    path.joinpath("package.aix").unlink()
    with pytest.raises(SystemExit) as error:
        cli.main(["verify", "example"], workspace=aidoku)
    assert error.value.code == 1
    assert "has not been packaged yet" in capsys.readouterr().err


def test_missing_wasm_fails_with_skip_build_hint(aidoku, source, capsys):
    source()
    with pytest.raises(SystemExit) as error:
        cli.main(["package", "--skip-build", "example"], workspace=aidoku)
    assert error.value.code == 1
    assert "drop --skip-build" in capsys.readouterr().err


def test_entry_point_starts_from_a_foreign_working_directory(tmp_path):
    script = Path(__file__).resolve().parents[1] / "aidoku.py"
    result = subprocess.run(
        [sys.executable, str(script), "--help"],
        cwd=tmp_path,
        check=True,
        capture_output=True,
        text=True,
    )
    assert "Workspace-aware stand-in" in result.stdout


def test_icons_command_flattens_and_resizes_multiple_images(aidoku, tmp_path):
    transparent = tmp_path / "transparent.png"
    translucent = tmp_path / "translucent.png"
    Image.new("RGBA", (2, 2), (12, 34, 56, 0)).save(transparent)
    Image.new("RGBA", (2, 2), (255, 0, 0, 128)).save(translucent)

    cli.main(["icons", str(transparent), str(translucent)], workspace=aidoku)

    with Image.open(tmp_path / "transparent_white_128.png") as result:
        assert result.size == (128, 128)
        assert result.mode == "RGB"
        assert result.getpixel((0, 0)) == (255, 255, 255)
    with Image.open(tmp_path / "translucent_white_128.png") as result:
        assert result.size == (128, 128)
        assert result.mode == "RGB"
        assert result.getpixel((0, 0)) == (255, 127, 127)
