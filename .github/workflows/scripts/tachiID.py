#!/usr/bin/env python3
"""Look up Tachiyomi/Mihon source IDs from the Keiyoushi extension index."""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, NamedTuple


DEFAULT_INDEX_URL = "https://raw.githubusercontent.com/keiyoushi/extensions/repo/index.min.json"


class SourceRecord(NamedTuple):
    name: str
    lang: str
    tachi_id: int
    base_url: str
    extension_name: str
    package: str
    apk: str
    extension_lang: str
    version: str
    version_id: int | None
    nsfw: bool


def _parse_tachi_id(value: Any) -> int | None:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value)
        except ValueError:
            return None
    return None


def flatten_sources(index: list[dict[str, Any]]) -> list[SourceRecord]:
    sources: list[SourceRecord] = []
    for extension in index:
        extension_name = str(extension.get("name", ""))
        package = str(extension.get("pkg", ""))
        apk = str(extension.get("apk", ""))
        extension_lang = str(extension.get("lang", ""))
        version = str(extension.get("version", ""))
        nsfw = bool(extension.get("nsfw", 0))

        for source in extension.get("sources", []):
            if not isinstance(source, dict):
                continue
            tachi_id = _parse_tachi_id(source.get("id"))
            if tachi_id is None:
                continue

            version_id = source.get("versionId")
            if not isinstance(version_id, int):
                version_id = None

            sources.append(
                SourceRecord(
                    name=str(source.get("name", "")),
                    lang=str(source.get("lang", extension_lang)),
                    tachi_id=tachi_id,
                    base_url=str(source.get("baseUrl", "")),
                    extension_name=extension_name,
                    package=package,
                    apk=apk,
                    extension_lang=extension_lang,
                    version=version,
                    version_id=version_id,
                    nsfw=nsfw,
                )
            )
    return sources


def search_sources(
    sources: list[SourceRecord],
    queries: list[str],
    *,
    langs: set[str] | None = None,
    exact: bool = False,
) -> list[SourceRecord]:
    normalized_langs = {lang.casefold() for lang in langs} if langs else None
    normalized_queries = [query.casefold() for query in queries if query.strip()]

    results: list[SourceRecord] = []
    for source in sources:
        if normalized_langs is not None and source.lang.casefold() not in normalized_langs:
            continue

        if not normalized_queries:
            results.append(source)
            continue

        if exact:
            haystacks = {source.name.casefold(), source.extension_name.casefold()}
            if any(query in haystacks for query in normalized_queries):
                results.append(source)
            continue

        searchable = " ".join(
            [
                source.name,
                source.extension_name,
                source.package,
                source.base_url,
                source.lang,
            ]
        ).casefold()
        if all(query in searchable for query in normalized_queries):
            results.append(source)

    return results


def format_sources_json(sources: list[SourceRecord]) -> str:
    mappings = [
        {
            "name": source.name,
            "aidoku": None,
            "tachi": source.tachi_id,
        }
        for source in sources
    ]
    return json.dumps(mappings, indent=2)


def format_json(sources: list[SourceRecord]) -> str:
    rows = [
        {
            "name": source.name,
            "lang": source.lang,
            "id": str(source.tachi_id),
            "tachi": source.tachi_id,
            "baseUrl": source.base_url,
            "extension": source.extension_name,
            "package": source.package,
            "apk": source.apk,
            "version": source.version,
            "versionId": source.version_id,
            "nsfw": source.nsfw,
        }
        for source in sources
    ]
    return json.dumps(rows, indent=2)


def format_table(sources: list[SourceRecord]) -> str:
    if not sources:
        return "No matching Tachiyomi sources found."

    headers = ["Name", "Lang", "Tachiyomi ID", "Base URL", "Extension"]
    rows = [
        [
            source.name,
            source.lang,
            str(source.tachi_id),
            source.base_url,
            source.extension_name,
        ]
        for source in sources
    ]
    widths = [
        max(len(row[index]) for row in [headers, *rows])
        for index in range(len(headers))
    ]

    lines = [
        "  ".join(value.ljust(widths[index]) for index, value in enumerate(headers)),
        "  ".join("-" * width for width in widths),
    ]
    lines.extend(
        "  ".join(value.ljust(widths[index]) for index, value in enumerate(row))
        for row in rows
    )
    return "\n".join(lines)


def load_index_from_file(path: Path) -> list[dict[str, Any]]:
    with path.open("r", encoding="utf-8") as file:
        data = json.load(file)
    if not isinstance(data, list):
        raise ValueError(f"{path} does not contain a JSON array")
    return data


def fetch_index(url: str, *, timeout: float = 30.0) -> list[dict[str, Any]]:
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "tachi-aidoku-get-tachiyomi-ids/1.0"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        data = json.load(response)
    if not isinstance(data, list):
        raise ValueError(f"{url} did not return a JSON array")
    return data


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Look up Tachiyomi source IDs from the Keiyoushi extension index.",
    )
    parser.add_argument(
        "query",
        nargs="*",
        help="Search terms. With multiple terms, all terms must match.",
    )
    parser.add_argument(
        "--exact",
        action="store_true",
        help="Match source or extension name exactly instead of substring search.",
    )
    parser.add_argument(
        "--lang",
        action="append",
        default=[],
        help="Only include a source language. Can be passed more than once.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit detailed JSON rows.",
    )
    parser.add_argument(
        "--sources-json",
        action="store_true",
        help="Emit entries shaped for this repo's sources.json.",
    )
    parser.add_argument(
        "--index-url",
        default=DEFAULT_INDEX_URL,
        help=f"Extension index URL. Default: {DEFAULT_INDEX_URL}",
    )
    parser.add_argument(
        "--index-file",
        type=Path,
        help="Read a local Keiyoushi index JSON file instead of fetching the URL.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=0,
        help="Maximum number of rows to print. Default: no limit.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    try:
        index = (
            load_index_from_file(args.index_file)
            if args.index_file is not None
            else fetch_index(args.index_url)
        )
    except (OSError, urllib.error.URLError, json.JSONDecodeError, ValueError) as error:
        print(f"error: failed to load extension index: {error}", file=sys.stderr)
        return 1

    sources = flatten_sources(index)
    matches = search_sources(
        sources,
        args.query,
        langs=set(args.lang) if args.lang else None,
        exact=args.exact,
    )
    if args.limit > 0:
        matches = matches[: args.limit]

    if args.sources_json:
        print(format_sources_json(matches))
    elif args.json:
        print(format_json(matches))
    else:
        print(format_table(matches))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
