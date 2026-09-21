from __future__ import annotations

import json
import struct
import tomllib

import pytest
from aidoku_workspace import cli, process, scaffold


def init(aidoku, *extra):
    cli.main(
        [
            "init",
            "-n",
            "Example Site",
            "-u",
            "https://example.test",
            "-l",
            "en",
            "-c",
            "safe",
            *extra,
        ],
        workspace=aidoku,
    )
    return aidoku.root / "sources/en.examplesite"


def test_init_generates_ordinary_workspace_member(aidoku, monkeypatch):
    monkeypatch.setattr(process, "cargo_fmt", lambda *_: None)
    monkeypatch.chdir(aidoku.root)
    path = init(aidoku)
    assert path.joinpath("Cargo.toml").read_text() == (
        '[package]\nname = "examplesite"\nversion.workspace = true\n'
        'edition.workspace = true\n\n[lib]\ncrate-type = ["cdylib"]\n\n'
        "[dependencies]\naidoku.workspace = true\n\n"
        '[dev-dependencies]\naidoku = { workspace = true, features = ["test"] }\n'
        "aidoku-test.workspace = true\n"
    )
    lib = path.joinpath("src/lib.rs").read_text()
    assert lib.startswith("#![no_std]\n")
    assert "struct ExampleSite;" in lib
    assert "impl Source for ExampleSite" in lib
    assert "fn get_search_manga_list(" in lib
    assert "fn get_manga_update(" in lib
    assert "fn get_page_list(" in lib
    assert "register_source!(ExampleSite, ListingProvider, Home, DeepLinkHandler);" in lib
    document = json.loads(path.joinpath("res/source.json").read_text())
    assert document == {
        "info": {
            "id": "en.examplesite",
            "name": "Example Site",
            "version": 1,
            "url": "https://example.test",
            "contentRating": 0,
            "languages": ["en"],
            "minAppVersion": "0.8.5",
        }
    }
    icon = path / "res/icon.png"
    data = icon.read_bytes()
    assert data.startswith(b"\x89PNG\r\n\x1a\n")
    assert struct.unpack(">II", data[16:24]) == (128, 128)
    assert data[25] == 2  # opaque RGB


@pytest.mark.parametrize(
    "params,registers,expected",
    [
        (
            "#[derive(Default)]\npub struct Params {\n\tpub base_url: String,\n}",
            "impl<T: Impl> Home for Wrapper<T> {}\nimpl<T: Impl> DeepLinkHandler for Wrapper<T> {}",
            ("Params {", "..Default::default()", "Home, DeepLinkHandler"),
        ),
        (
            "pub struct Params {\n\tpub base_url: String,\n}",
            "impl<T: Impl> DeepLinkHandler for Wrapper<T> {}",
            ('todo!("set base_url")', "DeepLinkHandler"),
        ),
    ],
)
def test_init_inspects_existing_template_trait_surface(aidoku, monkeypatch, params, registers, expected):
    monkeypatch.setattr(process, "cargo_fmt", lambda *_: None)
    template = aidoku.root / "templates/custom"
    (template / "src").mkdir(parents=True)
    template.joinpath("Cargo.toml").write_text('[package]\nname = "custom"\n')
    template.joinpath("src/lib.rs").write_text(
        "pub struct Wrapper<T: Impl> { inner: T }\n"
        + params
        + "\n"
        + "pub trait Impl {\n\tfn new() -> Self;\n\tfn params(&self) -> Params;\n"
        + "\tfn get_home(&self) -> i32 { 0 }\n}\n"
        + registers
        + "\n"
    )
    path = init(aidoku, "-t", "custom")
    lib = path.joinpath("src/lib.rs").read_text()
    for text in expected:
        assert text in lib
    assert "custom.workspace = true" in path.joinpath("Cargo.toml").read_text()
    assert "fn get_home" not in lib  # defaulted methods are not stubbed


def test_init_creates_template_and_workspace_dependency(aidoku, monkeypatch):
    monkeypatch.setattr(process, "cargo_fmt", lambda *_: None)
    monkeypatch.chdir(aidoku.root)
    path = init(aidoku, "--template", "-t", "Fresh Theme")
    template = aidoku.root / "templates/freshtheme"
    assert template.joinpath("Cargo.toml").read_text() == (
        '[package]\nname = "freshtheme"\nversion.workspace = true\n'
        "edition.workspace = true\n\n[dependencies]\naidoku.workspace = true\n\n"
        '[dev-dependencies]\naidoku = { workspace = true, features = ["test"] }\n'
        "aidoku-test.workspace = true\n"
    )
    template_lib = template.joinpath("src/lib.rs").read_text()
    assert "pub struct FreshTheme<T: Impl>" in template_lib
    assert "pub trait Impl" in template_lib
    assert "impl<T: Impl> Source for FreshTheme<T>" in template_lib
    assert 'freshtheme = { path = "templates/freshtheme" }' in aidoku.root.joinpath("Cargo.toml").read_text()
    assert "freshtheme.workspace = true" in path.joinpath("Cargo.toml").read_text()
    assert (
        "register_source!(FreshTheme<ExampleSite>, ListingProvider, Home, DeepLinkHandler);"
        in path.joinpath("src/lib.rs").read_text()
    )


def test_member_name_accepts_toml_string_syntax(aidoku, source):
    path = source()
    path.joinpath("Cargo.toml").write_text("[package]\nname = 'example' # crate name\n")
    assert aidoku.resolve(["example"])[0].path == path


def test_template_dependency_insertion_uses_parsed_paths(aidoku):
    manifest = aidoku.root / "Cargo.toml"
    manifest.write_text(
        manifest.read_text().replace(
            "[workspace.dependencies]\n",
            "[workspace.dependencies] # shared\n"
            "alpha = { path='templates/alpha' }\n"
            'zeta = { version = "1", path = "templates/zeta" }\n',
        )
    )
    scaffold.add_workspace_dependency(aidoku, "middle", "templates/middle")
    text = manifest.read_text()
    assert text.index("alpha =") < text.index("middle =") < text.index("zeta =")
    assert tomllib.loads(text)["workspace"]["dependencies"]["middle"]["path"] == "templates/middle"


def test_init_rejects_provided_invalid_url_before_prompting_for_other_fields(aidoku, monkeypatch, capsys):
    monkeypatch.setattr(
        cli, "prompt", lambda *_args, **_kwargs: pytest.fail("invalid URL should stop before prompting")
    )
    with pytest.raises(SystemExit) as error:
        cli.main(["init", "-n", "Example Site", "-u", "example.test"], workspace=aidoku)
    assert error.value.code == 1
    assert "URL must start" in capsys.readouterr().err


def test_init_empty_language_argument_prompts_and_validates(aidoku, monkeypatch):
    monkeypatch.setattr(process, "cargo_fmt", lambda *_: None)
    answers = []

    def provide_language(question, **_kwargs):
        answers.append(question)
        return "en"

    monkeypatch.setattr(cli, "prompt", provide_language)
    cli.main(
        ["init", "-n", "Example Site", "-u", "https://example.test", "-l", "", "-c", "safe"],
        workspace=aidoku,
    )
    assert answers == ["Languages (e.g. `en`, or `id pt ja`)"]
    assert (aidoku.root / "sources/en.examplesite").is_dir()


def test_min_app_version_checks_then_syncs_preserving_json_format(aidoku, source, capsys):
    path = source()
    source_json = path / "res/source.json"
    original = (
        '{\r\n\t"info": {\r\n\t\t"id": "en.example",\r\n'
        '\t\t"version": 3,\r\n\t\t"minAppVersion": "0.7.0"\r\n'
        '\t},\r\n\t"other": {"minAppVersion": "leave-me", "array": [1,  2]}\r\n}\r\n'
    )
    source_json.write_bytes(original.encode())
    with pytest.raises(SystemExit) as error:
        cli.main(["min-app-version", "example"], workspace=aidoku)
    assert error.value.code == 1
    assert "1 of 1 sources" in capsys.readouterr().err
    cli.main(["min-app-version", "--sync", "example"], workspace=aidoku)
    assert source_json.read_bytes() == original.replace('"0.7.0"', '"0.8.5"').encode()
    cli.main(["min-app-version", "example"], workspace=aidoku)
    assert "all 1 sources are on 0.8.5" in capsys.readouterr().out


def test_min_app_version_adds_missing_key_with_existing_indent(aidoku, source):
    path = source()
    source_json = path / "res/source.json"
    source_json.write_text('{\n\t"info": {\n\t\t"id": "en.example",\n\t\t"version": 3\n\t},\n\t"other": "kept"\n}\n')
    cli.main(["min-app-version", "--sync", "example"], workspace=aidoku)
    assert source_json.read_text() == (
        '{\n\t"info": {\n\t\t"id": "en.example",\n\t\t"version": 3,\n'
        '\t\t"minAppVersion": "0.8.5"\n\t},\n\t"other": "kept"\n}\n'
    )
