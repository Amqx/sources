from __future__ import annotations

import json
import shutil
import zipfile
from pathlib import Path

import pytest
from aidoku_workspace import cache, cli, packaging, process


@pytest.fixture
def cache_workspace(aidoku, source):
    path = source()
    template = aidoku.root / "templates" / "fixture"
    (template / "src").mkdir(parents=True)
    (template / "Cargo.toml").write_text('[package]\nname = "fixture"\n')
    (template / "src/lib.rs").write_text("// template\n")
    path.joinpath("Cargo.toml").write_text(
        '[package]\nname = "example"\n[dependencies]\naidoku.workspace = true\nfixture.workspace = true\n'
    )
    aidoku.root.joinpath("Cargo.toml").write_text(
        '[workspace]\nresolver = "2"\n'
        '[workspace.metadata.aidoku]\nmin-app-version = "0.8.5"\n'
        "[workspace.dependencies]\n"
        'aidoku = { version = "1" }\n'
        'fixture = { path = "templates/fixture" }\n'
        'unrelated = { version = "1" }\n'
    )
    aidoku.root.joinpath("Cargo.lock").write_text(
        "version = 4\n"
        '[[package]]\nname = "example"\nversion = "0.1.0"\n'
        'dependencies = ["aidoku", "fixture"]\n'
        '[[package]]\nname = "fixture"\nversion = "0.1.0"\n'
        '[[package]]\nname = "aidoku"\nversion = "1.0.0"\n'
        'source = "registry+test"\n'
        '[[package]]\nname = "unrelated"\nversion = "1.0.0"\n'
        'source = "registry+test"\n'
    )
    tooling = aidoku.root / "scripts" / "aidoku_workspace"
    tooling.mkdir(parents=True)
    aidoku.root.joinpath("scripts/aidoku.py").write_text("# startup wrapper\n")
    tooling.joinpath("cache.py").write_text("# package tooling\n")
    aidoku.root.joinpath("pyproject.toml").write_text(
        '[project]\nname = "fixture-project"\nrequires-python = ">=3.14"\n'
        'dependencies = ["runtime>=1"]\n'
        '[dependency-groups]\ndev = ["pytest>=9"]\n'
    )
    aidoku.root.joinpath("uv.lock").write_text(
        "version = 1\n"
        '[[package]]\nname = "fixture-project"\nversion = "0.1.0"\n'
        'source = { virtual = "." }\n'
        'dependencies = [{ name = "runtime" }]\n'
        '[package.dev-dependencies]\ndev = [{ name = "pytest" }]\n'
        '[[package]]\nname = "runtime"\nversion = "1.0.0"\n'
        'source = { registry = "https://pypi.org/simple" }\n'
        '[[package]]\nname = "pytest"\nversion = "9.0.0"\n'
        'source = { registry = "https://pypi.org/simple" }\n'
    )
    aidoku.root.joinpath(".python-version").write_text("3.14\n")
    return aidoku.resolve(["example"], "source")[0], template, tooling / "cache.py"


@pytest.mark.parametrize(
    "changed",
    ["source", "template", "dependency", "build_config", "tooling", "entry", "asset", "runtime"],
)
def test_reuse_fingerprint_invalidates_for_relevant_inputs(aidoku, cache_workspace, changed):
    member, template, tooling = cache_workspace
    before = cache.fingerprint(aidoku, member)
    cache_dir = aidoku.root / "cache"
    cache_dir.mkdir()
    cache_dir.joinpath(cache.MANIFEST_NAME).write_text(json.dumps({member.dir_name: before}))
    with zipfile.ZipFile(cache_dir / "example.aix", "w") as archive:
        archive.writestr("Payload/source.json", member.source_json.read_bytes())
    assert cache.Cache(aidoku, cache_dir).restore(member)
    member.package.unlink()
    if changed == "source":
        member.path.joinpath("src/lib.rs").write_text("// changed source\n")
    elif changed == "template":
        template.joinpath("src/lib.rs").write_text("// changed template\n")
    elif changed == "dependency":
        manifest = aidoku.root / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace('aidoku = { version = "1" }', 'aidoku = { version = "2" }'))
    elif changed == "build_config":
        config = aidoku.root / ".cargo/config.toml"
        config.parent.mkdir()
        config.write_text('[build]\ntarget = "wasm32-unknown-unknown"\n')
    elif changed == "tooling":
        tooling.write_text("# changed package tooling\n")
    elif changed == "entry":
        aidoku.root.joinpath("scripts/aidoku.py").write_text("# changed startup wrapper\n")
    elif changed == "asset":
        asset = tooling.parent / "templates" / "source.rs.template"
        asset.parent.mkdir()
        asset.write_text("// new scaffold asset\n")
    else:
        lock = aidoku.root / "uv.lock"
        lock.write_text(
            lock.read_text().replace('name = "runtime"\nversion = "1.0.0"', 'name = "runtime"\nversion = "2.0.0"')
        )
    assert cache.fingerprint(aidoku, member) != before
    assert not cache.Cache(aidoku, cache_dir).restore(member)
    assert not member.package.exists()


def test_unrelated_workspace_dependency_does_not_invalidate(aidoku, cache_workspace):
    member, _, _ = cache_workspace
    before = cache.fingerprint(aidoku, member)
    manifest = aidoku.root / "Cargo.toml"
    manifest.write_text(manifest.read_text().replace('unrelated = { version = "1" }', 'unrelated = { version = "2" }'))
    lock = aidoku.root / "Cargo.lock"
    lock.write_text(
        lock.read_text().replace('name = "unrelated"\nversion = "1.0.0"', 'name = "unrelated"\nversion = "2.0.0"')
    )
    assert cache.fingerprint(aidoku, member) == before


def test_tooling_paths_affect_hash_but_bytecode_and_temporary_files_do_not(aidoku, cache_workspace):
    member, _, tooling = cache_workspace
    asset = tooling.parent / "templates" / "source.rs.template"
    asset.parent.mkdir()
    asset.write_text("same content\n")
    before = cache.fingerprint(aidoku, member)
    asset.write_text("changed content\n")
    assert cache.fingerprint(aidoku, member) != before
    before = cache.fingerprint(aidoku, member)
    asset.rename(asset.with_name("template.rs.template"))
    assert cache.fingerprint(aidoku, member) != before
    renamed = cache.fingerprint(aidoku, member)
    tooling.parent.joinpath("__pycache__").mkdir()
    tooling.parent.joinpath("__pycache__/cache.cpython-314.pyc").write_bytes(b"bytecode")
    tooling.parent.joinpath("scratch.tmp").write_text("temporary")
    tooling.parent.joinpath("scratch.py~").write_text("editor backup")
    assert cache.fingerprint(aidoku, member) == renamed


def test_dev_only_python_dependency_update_preserves_reuse(aidoku, cache_workspace):
    member, _, _ = cache_workspace
    before = cache.fingerprint(aidoku, member)
    project = aidoku.root / "pyproject.toml"
    project.write_text(project.read_text().replace("pytest>=9", "pytest>=10"))
    lock = aidoku.root / "uv.lock"
    lock.write_text(
        lock.read_text().replace('name = "pytest"\nversion = "9.0.0"', 'name = "pytest"\nversion = "10.0.0"')
    )
    assert cache.fingerprint(aidoku, member) == before


def test_python_runtime_constraint_and_interpreter_changes_invalidate(aidoku, cache_workspace):
    member, _, _ = cache_workspace
    before = cache.fingerprint(aidoku, member)
    project = aidoku.root / "pyproject.toml"
    project.write_text(project.read_text().replace("runtime>=1", "runtime>=2"))
    assert cache.fingerprint(aidoku, member) != before
    project.write_text(project.read_text().replace("runtime>=2", "runtime>=1"))
    aidoku.root.joinpath(".python-version").write_text("3.15\n")
    assert cache.fingerprint(aidoku, member) != before


def test_transitive_python_runtime_update_invalidates(aidoku, cache_workspace):
    member, _, _ = cache_workspace
    lock = aidoku.root / "uv.lock"
    lock.write_text(
        lock.read_text().replace(
            'name = "runtime"\nversion = "1.0.0"\nsource = { registry = "https://pypi.org/simple" }',
            'name = "runtime"\nversion = "1.0.0"\nsource = { registry = "https://pypi.org/simple" }\n'
            'dependencies = [{ name = "transitive" }]\n'
            '[[package]]\nname = "transitive"\nversion = "1.0.0"\n'
            'source = { registry = "https://pypi.org/simple" }',
        )
    )
    before = cache.fingerprint(aidoku, member)
    lock.write_text(
        lock.read_text().replace(
            'name = "transitive"\nversion = "1.0.0"',
            'name = "transitive"\nversion = "2.0.0"',
        )
    )
    assert cache.fingerprint(aidoku, member) != before


@pytest.mark.parametrize("cached_id,cached_version", [("other.source", 3), ("en.example", 2), ("en.example", 3)])
def test_cache_restores_only_matching_id_and_version_atomically(
    aidoku, cache_workspace, cached_id, cached_version, monkeypatch
):
    member, _, _ = cache_workspace
    cache_dir = aidoku.root / "cache"
    cache_dir.mkdir()
    cache_dir.joinpath(cache.MANIFEST_NAME).write_text(json.dumps({member.dir_name: cache.fingerprint(aidoku, member)}))
    cached = cache_dir / "arbitrary-name.aix"
    with zipfile.ZipFile(cached, "w") as archive:
        archive.writestr("Payload/source.json", json.dumps({"info": {"id": cached_id, "version": cached_version}}))
    member.package.write_bytes(b"previous package")
    original_replace = cache.os.replace

    def inspect_swap(staging, destination):
        assert Path(destination).read_bytes() == b"previous package"
        assert Path(staging).read_bytes() == cached.read_bytes()
        original_replace(staging, destination)

    monkeypatch.setattr(cache.os, "replace", inspect_swap)
    restored = cache.Cache(aidoku, cache_dir).restore(member)
    assert restored is ((cached_id, cached_version) == member.source_key())
    assert member.package.read_bytes() == (cached.read_bytes() if restored else b"previous package")
    assert not member.package.with_name("package.aix.tmp").exists()


def test_manifest_records_directory_key_and_reuse_skips_build(aidoku, cache_workspace, monkeypatch):
    member, _, _ = cache_workspace
    output = aidoku.root / "cache" / cache.MANIFEST_NAME
    cli.main(["manifest", "example", "--output", str(output)], workspace=aidoku)
    assert json.loads(output.read_text()) == {"en.example": cache.fingerprint(aidoku, member)}
    wasm = aidoku.root / "main.wasm"
    wasm.write_bytes(b"wasm")
    packaging.write_aix(member, wasm, member.package)
    cached = output.parent / "package.aix"
    shutil.copyfile(member.package, cached)
    member.package.unlink()
    monkeypatch.setattr(process, "run", lambda *_: pytest.fail("unchanged package should not build"))
    cli.main(["package", "example", "--reuse-from", str(output.parent)], workspace=aidoku)
    assert member.package.read_bytes() == cached.read_bytes()
