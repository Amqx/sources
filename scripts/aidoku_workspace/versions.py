from __future__ import annotations

import json
import re
from collections.abc import Sequence
from pathlib import Path

from .common import die, info
from .workspace import Member, Workspace, load_toml

VERSION_NUMBER = re.compile(r"\d+(?:\.\d+)*")
MIN_APP_VERSION = re.compile(r'("minAppVersion"\s*:\s*)"[^"]*"')
MIN_APP_VERSION_KEY = re.compile(r'(?m)^([ \t]*min-app-version\s*=\s*)"[^"]*"')


def workspace_min_app_version(workspace: Workspace) -> str:
    """The `minAppVersion` every source is expected to declare."""
    value = (
        load_toml(workspace.root / "Cargo.toml")
        .get("workspace", {})
        .get("metadata", {})
        .get("aidoku", {})
        .get("min-app-version")
    )
    if not isinstance(value, str) or not value:
        die("no [workspace.metadata.aidoku] min-app-version in the root Cargo.toml")
    return value


def set_workspace_min_app_version(workspace: Workspace, value: str) -> None:
    """Rewrite the canonical value."""
    path = workspace.root / "Cargo.toml"
    with path.open(encoding="utf-8", newline="") as f:
        text = f.read()
    header = re.search(r"(?m)^\[workspace\.metadata\.aidoku\][^\n]*\n", text)
    if not header:
        die(f"no [workspace.metadata.aidoku] table in {workspace.rel(path)}")
    following = re.search(r"(?m)^\[", text[header.end() :])
    stop = len(text) if following is None else header.end() + following.start()
    table, count = MIN_APP_VERSION_KEY.subn(lambda m: f'{m.group(1)}"{value}"', text[header.end() : stop], count=1)
    if not count:
        die("no min-app-version key under [workspace.metadata.aidoku]")
    with path.open("w", encoding="utf-8", newline="") as f:
        f.write(text[: header.end()] + table + text[stop:])


def info_span(text: str) -> tuple[int, int] | None:
    """Indices of the braces opening and closing the `info` object."""
    opening = re.search(r'"info"\s*:\s*\{', text)
    if not opening:
        return None
    depth = 0
    quoted = False
    escaped = False
    start = opening.end() - 1
    for index in range(start, len(text)):
        char = text[index]
        if escaped:
            escaped = False
        elif char == "\\":
            escaped = quoted
        elif char == '"':
            quoted = not quoted
        elif not quoted:
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    return (start, index)
    return None


def source_min_app_version(workspace: Workspace, path: Path) -> str | None:
    """The declared value, or None if the key isn't there."""
    try:
        with path.open(encoding="utf-8") as f:
            document = json.load(f)
    except OSError as error:
        die(f"cannot read {workspace.rel(path)}: {error}")
    except ValueError as error:
        die(f"{workspace.rel(path)} is not valid JSON: {error}")
    value = document.get("info", {}).get("minAppVersion")
    return value if isinstance(value, str) else None


def write_min_app_version(workspace: Workspace, path: Path, value: str) -> bool:
    """Set `info.minAppVersion`, returning whether the file changed."""
    with path.open(encoding="utf-8", newline="") as f:
        text = f.read()
    try:
        json.loads(text)
    except ValueError as error:
        die(f"{workspace.rel(path)} is not valid JSON: {error}")

    span = info_span(text)
    if span is None:
        die(f"no info object in {workspace.rel(path)}")
    start, end = span

    block, count = MIN_APP_VERSION.subn(lambda m: f'{m.group(1)}"{value}"', text[start:end], count=1)
    if not count:
        last = len(block.rstrip()) - 1  # end of the final property's value
        if last < 0 or block[last] == "{":
            die(f"empty info object in {workspace.rel(path)}")
        line = block[block.rfind("\n", 0, last) + 1 : last + 1]
        indent = line[: len(line) - len(line.lstrip())]
        newline = "\r\n" if "\r\n" in text else "\n"
        block = f'{block[: last + 1]},{newline}{indent}"minAppVersion": "{value}"{block[last + 1 :]}'

    patched = text[:start] + block + text[end:]
    if patched == text:
        return False
    try:
        json.loads(patched)
    except ValueError as error:
        die(f"patching {workspace.rel(path)} produced invalid JSON: {error}")
    with path.open("w", encoding="utf-8", newline="") as f:
        f.write(patched)
    return True


def sync_min_app_version(
    workspace: Workspace,
    sources: Sequence[Member],
    *,
    sync: bool = False,
    set_value: str | None = None,
) -> None:
    """Hold every source to one `minAppVersion`."""
    if set_value is not None:
        if not VERSION_NUMBER.fullmatch(set_value):
            die(f"'{set_value}' is not a version number")
        set_workspace_min_app_version(workspace, set_value)
        info(f"root Cargo.toml: min-app-version = {set_value}")

    value = workspace_min_app_version(workspace)

    if set_value is None and not sync:
        drift = [(m, source_min_app_version(workspace, m.source_json)) for m in sources]
        drift = [(m, found) for m, found in drift if found != value]
        for member, found in drift:
            info("{}: {}".format(member, found or "missing"))
        if drift:
            die(f"{len(drift)} of {len(sources)} sources are not on {value}; rerun with --sync")
        info(f"all {len(sources)} sources are on {value}")
        return

    changed = [m for m in sources if write_min_app_version(workspace, m.source_json, value)]
    for member in changed:
        info(f"updated {member}")
    info(f"{len(changed)} of {len(sources)} sources changed; all now on {value}")
