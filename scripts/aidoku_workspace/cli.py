from __future__ import annotations

import argparse
import sys
from collections.abc import Callable, Sequence
from pathlib import Path

from . import process
from .cache import MANIFEST_NAME, Cache, write_manifest
from .common import die, info
from .icons import process_image
from .packaging import package_member, validate_member
from .scaffold import (
    CONTENT_RATINGS,
    check_languages,
    check_url,
    init_source,
    normalize_language,
    validate_source_target,
)
from .versions import sync_min_app_version
from .workspace import Workspace

SOURCE_LIST_NAME = "Amqx's Sources"


def prompt(
    question: str,
    default: str | None = None,
    validate: Callable[[str], str | None] | None = None,
) -> str:
    """Collect CLI-only interactive values before invoking scaffold operations."""
    if not sys.stdin.isatty():
        die(f"{question.lower()} is required (stdin is not a terminal)")
    suffix = f" [{default}]" if default else ""
    while True:
        answer = input(f"{question}{suffix}: ").strip() or (default or "")
        problem = validate(answer) if validate else None
        if not answer:
            continue
        if problem:
            print(problem, file=sys.stderr)
            continue
        return answer


def parser(workspace: Workspace | None = None) -> argparse.ArgumentParser:
    workspace = workspace or default_workspace()
    root = argparse.ArgumentParser(
        prog="scripts/aidoku.py",
        description="Aidoku workspace management and source packaging",
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
    package.add_argument("--skip-build", action="store_true", help="package the wasm already in target/")
    package.add_argument(
        "--reuse-from",
        metavar="DIR",
        help=f"directory of previously built .aix files (with a {MANIFEST_NAME}) to copy unchanged sources from instead of rebuilding them",
    )

    manifest = commands.add_parser("manifest", help="record build fingerprints for a later --reuse-from")
    manifest.add_argument("paths", nargs="*", metavar="SOURCE")
    manifest.add_argument(
        "-o",
        "--output",
        default=str(workspace.root / "public" / "sources" / MANIFEST_NAME),
        help="where to write the manifest",
    )

    build = commands.add_parser("build", help="build a source list from packaged sources")
    build.add_argument("files", nargs="*", metavar="FILE")
    build.add_argument("-o", "--output", default=str(workspace.root / "public"), help="output folder path")
    build.add_argument("-n", "--name", default=SOURCE_LIST_NAME, help="source list name")

    verify = commands.add_parser("verify", help="verify packaged sources")
    verify.add_argument("files", nargs="*", metavar="FILE")

    serve = commands.add_parser("serve", help="build a source list and serve it")
    serve.add_argument("files", nargs="*", metavar="FILE")
    serve.add_argument("-o", "--output", default=str(workspace.root / "public"), help="output folder path")
    serve.add_argument("-p", "--port", type=int, default=8080, help="port to serve on")

    logcat = commands.add_parser("logcat", help="open a server for log streaming")
    logcat.add_argument("-p", "--port", type=int, default=9000, help="port to listen on")

    icons = commands.add_parser("icons", help="flatten icons onto white and resize them to 128x128 PNGs")
    icons.add_argument("files", nargs="+", metavar="IMAGE", help="input images")

    init = commands.add_parser("init", help="scaffold a new source or template")
    init.add_argument("path", nargs="?", help="defaults to sources/<source id>")
    init.add_argument("-n", "--name", help="source name")
    init.add_argument("-u", "--url", help="source homepage url")
    init.add_argument("-l", "--languages", action="append", default=[], metavar="LANG", help="source languages")
    init.add_argument("-c", "--content-rating", choices=sorted(CONTENT_RATINGS))
    init.add_argument("--template", action="store_true", help="also scaffold a source template")
    init.add_argument("-t", "--template-name", help="template to create, or an existing one to use")

    min_app = commands.add_parser("min-app-version", help="check or set the minAppVersion every source declares")
    min_app.add_argument("paths", nargs="*", metavar="SOURCE")
    min_app.add_argument("--sync", action="store_true", help="write the workspace value into every source")
    min_app.add_argument(
        "--set", metavar="VERSION", dest="set_value", help="change the workspace value, then sync it out"
    )

    listing = commands.add_parser("list", help="list workspace members and their crate names")
    listing.add_argument("paths", nargs="*", metavar="MEMBER")
    listing.add_argument("-k", "--kind", choices=("source", "template"), help="limit to one kind of member")
    return root


def default_workspace() -> Workspace:
    return Workspace(Path(__file__).resolve().parents[2])


def command_package(workspace: Workspace, args: argparse.Namespace) -> None:
    targets = workspace.resolve(args.paths, "source")
    cache = Cache(workspace, Path(args.reuse_from).resolve()) if args.reuse_from else None
    reused = 0
    for member in targets:
        validate_member(member)
        if cache is not None and cache.restore(member):
            reused += 1
            info(f"reused {member} -> {workspace.rel(member.package)}")
        else:
            package_member(workspace, member, skip_build=args.skip_build)
    if not targets:
        info("nothing to package")
    elif cache is not None:
        info(f"reused {reused} of {len(targets)} packages")


def command_list(workspace: Workspace, args: argparse.Namespace) -> None:
    rows = [
        (str(member), member.name or "?", member.source_id() or "-")
        for member in workspace.resolve(args.paths, args.kind)
    ]
    if rows:
        widths = [max(len(row[index]) for row in rows) for index in range(3)]
        for row in rows:
            info("  ".join(value.ljust(widths[index]) for index, value in enumerate(row)).rstrip())


def execute(workspace: Workspace, args: argparse.Namespace) -> None:
    """Map parsed CLI values to explicit module operations."""
    if args.command == "package":
        command_package(workspace, args)
    elif args.command == "manifest":
        write_manifest(workspace, workspace.resolve(args.paths, "source"), Path(args.output).resolve())
    elif args.command == "build":
        output = str(Path(args.output).resolve())
        process.forward(
            workspace, "build", ["--output", output, "--name", args.name, *workspace.package_files(args.files)]
        )
        info(f"wrote source list to {workspace.rel(output)}")
    elif args.command == "verify":
        process.forward(workspace, "verify", workspace.package_files(args.files))
    elif args.command == "serve":
        output = str(Path(args.output).resolve())
        process.forward(
            workspace, "serve", ["--output", output, "--port", str(args.port), *workspace.package_files(args.files)]
        )
    elif args.command == "logcat":
        process.forward(workspace, "logcat", ["--port", str(args.port)])
    elif args.command == "icons":
        for path in args.files:
            process_image(path)
    elif args.command == "init":
        name = args.name or prompt("Source name")
        url = args.url or prompt("Source URL", validate=check_url)
        problem = check_url(url)
        if problem:
            die(problem)
        languages = [language for value in args.languages for language in value.split()]
        if not languages:
            languages = prompt(
                "Languages (e.g. `en`, or `id pt ja`)", validate=lambda value: check_languages(value.split())
            ).split()
        languages = [normalize_language(language) for language in languages]
        problem = check_languages(languages)
        if problem:
            die(problem)
        rating = args.content_rating or prompt(
            "Content rating ({})".format("/".join(CONTENT_RATINGS)),
            default="safe",
            validate=lambda value: None if value in CONTENT_RATINGS else "unknown content rating",
        )
        template_name = args.template_name
        if (args.template or template_name) and template_name is None:
            validate_source_target(workspace, path=args.path, name=name, languages=languages)
            template_name = prompt("Template name")
        init_source(
            workspace,
            path=args.path,
            name=name,
            url=url,
            languages=languages,
            content_rating=rating,
            create_template=args.template,
            template_name=template_name,
        )
    elif args.command == "min-app-version":
        if args.set_value is not None and args.paths:
            die("--set changes the value for the whole workspace; drop the source arguments")
        sync_min_app_version(
            workspace, workspace.resolve(args.paths, "source"), sync=args.sync, set_value=args.set_value
        )
    elif args.command == "list":
        command_list(workspace, args)


def main(argv: Sequence[str] | None = None, workspace: Workspace | None = None) -> None:
    workspace = workspace or default_workspace()
    args = parser(workspace).parse_args(argv)
    try:
        execute(workspace, args)
    except KeyboardInterrupt:
        raise SystemExit(130)
