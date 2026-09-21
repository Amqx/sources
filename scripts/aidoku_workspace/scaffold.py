"""Source and template scaffold rendering."""

from __future__ import annotations

import json
import re
import shutil
import struct
import tomllib
import unicodedata
import zlib
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any, NamedTuple

from . import process
from .common import die, info
from .versions import workspace_min_app_version
from .workspace import Member, Workspace

LANGUAGE_CODES = {
    "ab",
    "aa",
    "af",
    "ak",
    "sq",
    "am",
    "ar",
    "an",
    "hy",
    "as",
    "av",
    "ae",
    "ay",
    "az",
    "bm",
    "ba",
    "eu",
    "be",
    "bn",
    "bi",
    "bs",
    "br",
    "bg",
    "my",
    "ca",
    "ch",
    "ce",
    "ny",
    "zh",
    "cu",
    "cv",
    "kw",
    "co",
    "cr",
    "hr",
    "cs",
    "da",
    "dv",
    "nl",
    "dz",
    "en",
    "eo",
    "et",
    "ee",
    "fo",
    "fj",
    "fi",
    "fr",
    "fy",
    "ff",
    "gd",
    "gl",
    "lg",
    "ka",
    "de",
    "el",
    "kl",
    "gn",
    "gu",
    "ht",
    "ha",
    "he",
    "hz",
    "hi",
    "ho",
    "hu",
    "is",
    "io",
    "ig",
    "id",
    "ia",
    "ie",
    "iu",
    "ik",
    "ga",
    "it",
    "ja",
    "jv",
    "kn",
    "kr",
    "ks",
    "kk",
    "km",
    "ki",
    "rw",
    "ky",
    "kv",
    "kg",
    "ko",
    "kj",
    "ku",
    "lo",
    "la",
    "lv",
    "li",
    "ln",
    "lt",
    "lu",
    "lb",
    "mk",
    "mg",
    "ms",
    "ml",
    "mt",
    "gv",
    "mi",
    "mr",
    "mh",
    "mn",
    "na",
    "nv",
    "nd",
    "nr",
    "ng",
    "ne",
    "no",
    "nb",
    "nn",
    "oc",
    "oj",
    "or",
    "om",
    "os",
    "pi",
    "ps",
    "fa",
    "pl",
    "pt",
    "pa",
    "qu",
    "ro",
    "rm",
    "rn",
    "ru",
    "se",
    "sm",
    "sg",
    "sa",
    "sc",
    "sr",
    "sn",
    "sd",
    "si",
    "sk",
    "sl",
    "so",
    "st",
    "es",
    "su",
    "sw",
    "ss",
    "sv",
    "tl",
    "ty",
    "tg",
    "ta",
    "tt",
    "te",
    "th",
    "bo",
    "ti",
    "to",
    "ts",
    "tn",
    "tr",
    "tk",
    "tw",
    "ug",
    "uk",
    "ur",
    "uz",
    "ve",
    "vi",
    "vo",
    "wa",
    "cy",
    "wo",
    "xh",
    "ii",
    "yi",
    "yo",
    "za",
    "zu",
} | {"ceb", "fil", "es-419", "pt-BR", "zh-Hans", "zh-Hant"}

# The upstream list spells this one `pt-br`, but all eight Brazilian sources in
# this repo use `pt-BR`, so accept either spelling and write the repo's.
LANGUAGE_ALIASES = {code.lower(): code for code in LANGUAGE_CODES}


def normalize_language(code: str) -> str:
    return LANGUAGE_ALIASES.get(code.lower(), code)


CONTENT_RATINGS: dict[str, int] = {"safe": 0, "contains-nsfw": 1, "primarily-nsfw": 2}

# source.rs.template is kept in sync by hand with the aidoku-rs CLI scaffold.
# Resolve assets from this package, independent of the caller's cwd.
ASSET_DIR = Path(__file__).resolve().parent / "templates"


def read_asset(name: str) -> str:
    return (ASSET_DIR / name).read_text(encoding="utf-8")


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
    """Creates a 128x128 opaque PNG placeholder icon."""
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
    """Source name -> crate name, following the ids already in the repo."""
    ascii_name = unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode("ascii").lower()
    cleaned = "".join(c for c in ascii_name if c.isalnum() or c == "_").strip("_")
    return "test_source" if cleaned == "test" else cleaned


def type_name_for(name: str) -> str:
    """Source or template name -> Rust type name, accents folded away."""
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
    """The `{ ... }` body of the first item matching `pattern`, brace-matched."""
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
    """Read a template crate to find out what a source built on it may write."""
    text = template_sources(path)

    match = re.search(r"^pub struct (\w+)<T: Impl>", text, re.MULTILINE)
    type_name = match.group(1) if match else None

    params = braced_block(text, r"^pub struct Params\b")
    fields = tuple(re.findall(r"^\tpub (\w+):", params, re.MULTILINE)) if params else ()
    attrs = re.search(r"((?:#\[[^\]]*\]\s*)*)pub struct Params\b", text, re.MULTILINE)
    params_has_default = bool(
        (attrs and "Default" in attrs.group(1)) or re.search(r"^impl Default for Params\b", text, re.MULTILINE)
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
        name for name in re.findall(r"^\tfn (\w+)", trait_body, re.MULTILINE) if name not in ("new", "params")
    )
    return TemplateSurface(type_name, fields, params_has_default, registers, overridable)


def render_source_template_lib(source_type: str, template: TemplateRef) -> str:
    """Build the source's lib.rs around what the template actually offers."""
    surface = template.surface
    if not surface.params_has_default:
        # no Default to fall back on, so name the fields that need filling in.
        # one diverging expression, or the fields after it are unreachable code
        hint = ", ".join(surface.params_fields)
        params_literal = f'todo!("set {hint}")' if hint else "todo!()"
    elif surface.params_fields:
        params_literal = "Params {\n\t\t\t..Default::default()\n\t\t}"
    else:
        # clippy flags `..Default::default()` on a struct with no fields
        params_literal = "Params::default()"

    return read_asset("source-wrapper.rs.template").format(
        lib=template.lib,
        template_imports=", ".join(sorted(["Impl", "Params", template.type_name])),
        source=source_type,
        params=params_literal,
        type_name=template.type_name,
        registers="".join(", " + trait for trait in surface.registers),
    )


def check_url(url: str) -> str | None:
    if not url.startswith(("http://", "https://")):
        return "URL must start with http:// or https://"
    return None


def check_languages(languages: Iterable[str]) -> str | None:
    unknown = [lang for lang in languages if lang not in LANGUAGE_CODES]
    if unknown:
        return "not valid ISO 639 language codes: {}".format(", ".join(unknown))
    return None


def add_workspace_dependency(workspace: Workspace, dep_name: str, path: str) -> None:
    """Register a new template in the root [workspace.dependencies] table."""
    manifest = workspace.root / "Cargo.toml"
    with manifest.open(encoding="utf-8", newline="") as file:
        text = file.read()
    try:
        dependencies = tomllib.loads(text)["workspace"]["dependencies"]
    except tomllib.TOMLDecodeError as error:
        die(f"{workspace.rel(manifest)} is not valid TOML: {error}")
    except KeyError:
        die("root Cargo.toml has no [workspace.dependencies] table")
    if dep_name in dependencies:
        die(f"{dep_name} is already a workspace dependency")

    lines = text.splitlines(keepends=True)
    newline = "\r\n" if "\r\n" in text else "\n"
    entry = f'{dep_name} = {{ path = "{path}" }}'

    header = re.compile(r"[ \t]*\[workspace\.dependencies\][ \t]*(?:#.*)?\r?\n?")
    start = next((i + 1 for i, line in enumerate(lines) if header.fullmatch(line)), None)
    if start is None:
        die("cannot locate [workspace.dependencies] in root Cargo.toml")
    end = start
    while end < len(lines) and not re.match(r"[ \t]*\[", lines[end]):
        end += 1

    # keep template entries together in their own block, sorted by name
    template_names = [
        name
        for name, spec in dependencies.items()
        if isinstance(spec, dict) and isinstance(spec.get("path"), str) and spec["path"].startswith("templates/")
    ]
    if template_names:
        neighbor = next((name for name in sorted(template_names) if name > dep_name), None)
        if neighbor is None:
            neighbor = max(template_names)
            offset = 1
        else:
            offset = 0
        key = re.escape(neighbor)
        key_line = re.compile(rf"[ \t]*(?:{key}|\"{key}\"|'{key}')[ \t]*=")
        insert_at = next((i + offset for i in range(start, end) if key_line.match(lines[i])), None)
        if insert_at is None:
            die(f"cannot locate {neighbor} in root Cargo.toml")
        lines.insert(insert_at, entry + newline)
    else:
        insert_at = end
        while insert_at > start and not lines[insert_at - 1].strip():
            insert_at -= 1
        lines[insert_at:insert_at] = [
            newline,
            "# Source templates (multisrc)" + newline,
            entry + newline,
        ]

    with manifest.open("w", encoding="utf-8", newline="") as file:
        file.write("".join(lines))
    workspace._workspace_paths = None
    info(f"added {dep_name} to [workspace.dependencies]")


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
        extra_deps = f"{template.crate}.workspace = true\n"
    else:
        lib = read_asset("source.rs.template").replace("{{SOURCE_NAME}}", source_type)
        extra_deps = ""

    (directory / "src" / "lib.rs").write_text(lib, encoding="utf-8")
    (directory / "Cargo.toml").write_text(
        read_asset("source.Cargo.toml.template").format(package=package, extra_deps=extra_deps),
        encoding="utf-8",
    )
    (directory / "res" / "source.json").write_text(
        json.dumps(source_json, indent="\t", ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    write_placeholder_icon(directory / "res" / "icon.png", source_json["info"]["id"])


def write_template(directory: Path, package: str, template_type: str) -> None:
    (directory / "src").mkdir(parents=True)
    (directory / "src" / "lib.rs").write_text(
        read_asset("template.rs.template").replace("{{TEMPLATE_NAME}}", template_type), encoding="utf-8"
    )
    (directory / "Cargo.toml").write_text(
        read_asset("template.Cargo.toml.template").format(package=package), encoding="utf-8"
    )


def validate_source_target(
    workspace: Workspace,
    *,
    path: str | None,
    name: str,
    languages: Sequence[str],
    existing: Sequence[Member] | None = None,
) -> None:
    """Reject invalid source destinations before template creation is considered."""
    package = package_name_for(name)
    if not package:
        die(f"could not derive a crate name from '{name}'")
    prefix = languages[0].split("-")[0] if len(languages) == 1 else "multi"
    source_id = f"{prefix}.{package}"
    directory = Path(path).resolve() if path else workspace.root / "sources" / source_id
    if directory.parent != workspace.root / "sources":
        die(f"{workspace.rel(directory)} is not directly under sources/, so it can't be a workspace member")
    if directory.exists():
        die(f"{workspace.rel(directory)} already exists")
    clash = next(
        (member for member in (existing or workspace.members()) if member.name == package),
        None,
    )
    if clash:
        die(f"the crate name '{package}' is already taken by {clash}; pass a different --name")


def init_source(
    workspace: Workspace,
    *,
    path: str | None,
    name: str,
    url: str,
    languages: Sequence[str],
    content_rating: str,
    create_template: bool,
    template_name: str | None,
) -> None:
    """Create a source member from fully specified, validated CLI values."""
    existing = workspace.members()

    problem = check_url(url)
    if problem:
        die(problem)
    source_languages = [lang for arg in languages for lang in arg.split()]
    source_languages = [normalize_language(lang) for lang in source_languages]
    problem = check_languages(source_languages)
    if problem:
        die(problem)
    if not source_languages:
        die("Languages (e.g. `en`, or `id pt ja`) is required")
    rating = content_rating

    validate_source_target(
        workspace,
        path=path,
        name=name,
        languages=source_languages,
        existing=existing,
    )
    package = package_name_for(name)
    # a regional code keeps only its base subtag in the id: the two pt-BR
    # sources here are pt.flowermanga and pt.mangalivre
    prefix = source_languages[0].split("-")[0] if len(source_languages) == 1 else "multi"
    source_id = f"{prefix}.{package}"

    directory = Path(path).resolve() if path else workspace.root / "sources" / source_id

    template = None
    if create_template or template_name:
        if template_name is None:
            die("a template name is required")
        template = init_template(workspace, template_name, existing, package)

    source_json = {
        "info": {
            "id": source_id,
            "name": name,
            "version": 1,
            "url": url,
            "contentRating": CONTENT_RATINGS[rating],
            "languages": source_languages,
            "minAppVersion": workspace_min_app_version(workspace),
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
    process.cargo_fmt(workspace, formatted)

    info(f"created {workspace.rel(directory)} ({package})")
    info("")
    info("next steps:")
    info("  * replace the placeholder res/icon.png with a real 128x128 opaque icon")
    if template and template.surface.overridable:
        info("  * {} can also override: {}".format(package, ", ".join(template.surface.overridable)))
    info(f"  * cargo clippy -p {package}")
    info(f"  * scripts/aidoku.py package {workspace.rel(directory)}")


def init_template(
    workspace: Workspace,
    template_name: str,
    existing: list[Member],
    source_package: str,
) -> TemplateRef:
    """Create the template crate, or reuse one the workspace already has."""
    base = package_name_for(template_name)
    type_name = type_name_for(template_name)
    if not base or not type_name:
        die(f"could not derive a crate name from '{template_name}'")

    directory = workspace.root / "templates" / base
    reused = next(
        (m for m in existing if m.kind == "template" and (m.path == directory or m.name == base)),
        None,
    )
    if reused:
        if not reused.name:
            die(f"{workspace.rel(reused.path)} has no [package] name in its Cargo.toml")
        info(f"reusing the existing {workspace.rel(reused.path)} template")
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
        die(f"{workspace.rel(directory)} already exists")

    # No two crates in a workspace may share a name. templates/mangaworld ships
    # as `mangaworld_template` because a source already claimed `mangaworld`;
    # follow that precedent rather than refusing to scaffold.
    taken = {m.name for m in existing} | {source_package}
    package = base if base not in taken else f"{base}_template"
    if package in taken:
        die(f"the crate name '{package}' is already taken; pass a different --template-name")

    write_template(directory, package, type_name)
    add_workspace_dependency(workspace, package, f"templates/{base}")
    info(f"created {workspace.rel(directory)} ({package})")

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
