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

Run `scripts/aidoku.py --help`, or `--help` on any command, for usage.
"""

import argparse
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import unicodedata
import zipfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "wasm32-unknown-unknown"
SOURCE_LIST_NAME = "Amqx's Sources"
CLI_INSTALL_HINT = (
    "install it with: cargo install --git https://github.com/Aidoku/aidoku-rs aidoku-cli"
)


# ---------------------------------------------------------------- utilities

def die(msg):
    print("error: {}".format(msg), file=sys.stderr)
    raise SystemExit(1)


def info(msg):
    print(msg, flush=True)


def rel(path):
    """Path relative to the repo root, for display."""
    try:
        return str(Path(path).resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def build_dir():
    """The one directory every workspace member's wasm lands in."""
    target_dir = os.environ.get("CARGO_TARGET_DIR") or (ROOT / "target")
    return Path(target_dir) / TARGET / "release"


def run(cmd, **kwargs):
    result = subprocess.run(cmd, cwd=str(ROOT), **kwargs)
    if result.returncode != 0:
        raise SystemExit(result.returncode)
    return result


def cargo_fmt(packages):
    """Best effort: a missing rustfmt shouldn't fail an otherwise fine scaffold."""
    if not packages or shutil.which("cargo") is None:
        return
    command = ["cargo", "fmt"]
    for package in packages:
        command += ["-p", package]
    subprocess.run(
        command, cwd=str(ROOT), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )


def require_cli():
    if shutil.which("aidoku") is None:
        die("the aidoku CLI is not installed; " + CLI_INSTALL_HINT)


def forward(command, args):
    """Hand a workspace-agnostic command off to the real CLI."""
    require_cli()
    run(["aidoku", command] + args)


# ------------------------------------------------------------ workspace model

class Member:
    """A source or template crate in the workspace."""

    def __init__(self, path, kind):
        self.path = path
        self.kind = kind
        self.name = read_package_name(path / "Cargo.toml")

    @property
    def dir_name(self):
        return self.path.name

    @property
    def source_json(self):
        return self.path / "res" / "source.json"

    @property
    def package(self):
        return self.path / "package.aix"

    def source_id(self):
        try:
            with self.source_json.open(encoding="utf-8") as f:
                return json.load(f)["info"]["id"]
        except (OSError, ValueError, KeyError):
            return None

    def __str__(self):
        return rel(self.path)


def read_package_name(manifest):
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


def members(kind=None):
    found = []
    for group, directory in (("source", ROOT / "sources"), ("template", ROOT / "templates")):
        if kind not in (None, group) or not directory.is_dir():
            continue
        for path in sorted(directory.iterdir()):
            if (path / "Cargo.toml").is_file():
                found.append(Member(path, group))
    return found


def resolve(targets, kind=None):
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


def package_files(targets):
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
        if Path(target).is_file():
            files.append(target)
            continue
        member = resolve([target], "source")[0]
        if not member.package.is_file():
            die("{} has not been packaged yet; run `scripts/aidoku.py package {}`".format(
                member, target
            ))
        files.append(str(member.package))
    return files


# -------------------------------------------------------------------- package

def wasm_for(name):
    """cargo writes lib artifacts with hyphens replaced by underscores."""
    for candidate in (name.replace("-", "_"), name):
        path = build_dir() / (candidate + ".wasm")
        if path.is_file():
            return path
    return None


def write_aix(member, wasm, output):
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


def cmd_package(args):
    targets = resolve(args.paths, "source")

    for member in targets:
        if member.name is None:
            die("{}: could not read the package name from Cargo.toml".format(member))
        if not member.source_json.is_file():
            die("{}: res/source.json is missing".format(member))

        if not args.skip_build:
            # One package per invocation: `-p a -p b` unifies their feature sets
            # and would link features into a source that never asked for them.
            # The shared target dir still reuses every common dependency build.
            run(["cargo", "build", "--release", "--target", TARGET, "-p", member.name])

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


# -------------------------------------------- build / verify / serve / logcat

def cmd_build(args):
    files = package_files(args.files)
    forward("build", ["--output", args.output, "--name", args.name] + files)
    info("wrote source list to {}".format(rel(args.output)))


def cmd_verify(args):
    files = package_files(args.files)
    forward("verify", files)


def cmd_serve(args):
    files = package_files(args.files)
    forward("serve", ["--output", args.output, "--port", str(args.port)] + files)


def cmd_logcat(args):
    forward("logcat", ["--port", str(args.port)])


def cmd_list(args):
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
""".split()) | {"fil", "zh-Hans", "zh-Hant", "pt-br", "es-419"}

CONTENT_RATINGS = {"safe": 0, "contains-nsfw": 1, "primarily-nsfw": 2}

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

SOURCE_TEMPLATE_LIB = '''#![no_std]
use aidoku::{AidokuError, DeepLinkResult, HomeLayout, Result, Source, alloc::String, prelude::*};
{{TEMPLATE_USE}}

struct {{SOURCE_NAME}};

impl Impl for {{SOURCE_NAME}} {
\tfn new() -> Self {
\t\tSelf
\t}

\tfn params(&self) -> Params {
\t\t{{PARAMS_BODY}}
\t}

\tfn get_home(&self, _params: &Params) -> Result<HomeLayout> {
\t\tErr(AidokuError::Unimplemented)
\t}

\tfn handle_deep_link(&self, _params: &Params, _url: String) -> Result<Option<DeepLinkResult>> {
\t\tErr(AidokuError::Unimplemented)
\t}
}

register_source!({{TEMPLATE_NAME}}<{{SOURCE_NAME}}>, ListingProvider, Home, DeepLinkHandler);
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


def png_chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))


def write_placeholder_icon(path, seed):
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


def package_name_for(name):
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


def type_name_for(name):
    """Source or template name -> Rust type name, accents folded away."""
    ascii_name = unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode("ascii")
    return "".join(c for c in ascii_name if c.isalnum())


def prompt(question, default=None, validate=None):
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


def check_url(url):
    if not url.startswith(("http://", "https://")):
        return "URL must start with http:// or https://"
    return None


def check_languages(languages):
    unknown = [lang for lang in languages if lang not in LANGUAGE_CODES]
    if unknown:
        return "not valid ISO 639 language codes: {}".format(", ".join(unknown))
    return None


def add_workspace_dependency(dep_name, path):
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


def write_source(directory, package, source_json, template=None):
    (directory / "src").mkdir(parents=True)
    (directory / "res").mkdir(parents=True)

    source_type = type_name_for(source_json["info"]["name"])
    if template:
        dep_name, lib_name, template_type, is_new = template
        imports = ", ".join(sorted(["Impl", "Params", template_type]))
        # a freshly scaffolded template has no Params fields yet, and clippy
        # flags `..Default::default()` on a struct literal that fills them all
        params = (
            "Params::default()"
            if is_new
            else "Params {\n\t\t\t..Default::default()\n\t\t}"
        )
        lib = (
            SOURCE_TEMPLATE_LIB.replace(
                "{{TEMPLATE_USE}}", "use {}::{{{}}};".format(lib_name, imports)
            )
            .replace("{{PARAMS_BODY}}", params)
            .replace("{{TEMPLATE_NAME}}", template_type)
            .replace("{{SOURCE_NAME}}", source_type)
        )
        extra_deps = "{}.workspace = true\n".format(dep_name)
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


def write_template(directory, package, template_type):
    (directory / "src").mkdir(parents=True)
    (directory / "src" / "lib.rs").write_text(
        TEMPLATE_LIB.replace("{{TEMPLATE_NAME}}", template_type), encoding="utf-8"
    )
    (directory / "Cargo.toml").write_text(
        TEMPLATE_MANIFEST.format(package=package), encoding="utf-8"
    )


def cmd_init(args):
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
    source_id = "{}.{}".format(languages[0] if len(languages) == 1 else "multi", package)

    directory = Path(args.path) if args.path else ROOT / "sources" / source_id
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
    write_source(directory, package, source_json, template)

    # a long `register_source!` line needs wrapping, so let rustfmt have a pass
    # rather than shipping a scaffold that fails `cargo fmt --check`
    formatted = [package] + ([template[0]] if template and template[3] else [])
    cargo_fmt(formatted)

    info("created {} ({})".format(rel(directory), package))
    info("")
    info("next steps:")
    info("  * replace the placeholder res/icon.png with a real 128x128 opaque icon")
    info("  * cargo clippy -p {}".format(package))
    info("  * scripts/aidoku.py package {}".format(rel(directory)))


def init_template(template_name, existing, source_package):
    """Create the template crate, or reuse it if the workspace already has one.

    Returns the (crate name, lib name, type name) the new source needs, plus
    whether the crate was created rather than reused.
    """
    template_name = template_name or prompt("Template name")
    base = package_name_for(template_name)
    template_type = type_name_for(template_name)
    if not base or not template_type:
        die("could not derive a crate name from '{}'".format(template_name))

    directory = ROOT / "templates" / base
    reused = next(
        (m for m in existing if m.kind == "template" and (m.path == directory or m.name == base)),
        None,
    )
    if reused:
        info("reusing the existing {} template".format(rel(reused.path)))
        return (reused.name, reused.name.replace("-", "_"), template_type, False)

    if directory.exists():
        die("{} already exists".format(rel(directory)))

    # No two crates in a workspace may share a name. templates/mangaworld ships
    # as `mangaworld_template` because a source already claimed `mangaworld`;
    # follow that precedent rather than refusing to scaffold.
    taken = {m.name for m in existing} | {source_package}
    package = base if base not in taken else "{}_template".format(base)
    if package in taken:
        die("the crate name '{}' is already taken; pass a different --template-name".format(package))

    write_template(directory, package, template_type)
    add_workspace_dependency(package, "templates/{}".format(base))
    info("created {} ({})".format(rel(directory), package))
    return (package, package.replace("-", "_"), template_type, True)

# ------------------------------------------------------------------ argparser

def parser():
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
    package.set_defaults(func=cmd_package)

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


def main():
    args = parser().parse_args()
    try:
        args.func(args)
    except KeyboardInterrupt:
        raise SystemExit(130)


if __name__ == "__main__":
    main()
