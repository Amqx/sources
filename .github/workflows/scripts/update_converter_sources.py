#!/usr/bin/env python3
"""Append missing Aidoku source mappings to Amqx/converter's sources.json."""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
from pathlib import Path
from typing import Any, Iterable, NamedTuple

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import tachiID  # noqa: E402


class AidokuSource(NamedTuple):
    aidoku_id: str
    name: str
    language: str | None


class UpdateResult(NamedTuple):
    added_count: int
    unmatched_count: int


def _read_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as file:
        return json.load(file)


def _write_json(path: Path, value: Any) -> None:
    with path.open("w", encoding="utf-8") as file:
        json.dump(value, file, indent=2, ensure_ascii=False)
        file.write("\n")


def _language_from_info(aidoku_id: str, info: dict[str, Any]) -> str | None:
    prefix = aidoku_id.split(".", 1)[0]
    if prefix == "multi":
        return None
    return prefix or None


def load_aidoku_sources(aidoku_root: Path) -> list[AidokuSource]:
    sources: list[AidokuSource] = []
    for source_json in sorted((aidoku_root / "sources").glob("*/res/source.json")):
        data = _read_json(source_json)
        if not isinstance(data, dict) or not isinstance(data.get("info"), dict):
            raise ValueError(f"{source_json} does not contain an info object")

        info = data["info"]
        aidoku_id = info.get("id")
        name = info.get("name")
        if not isinstance(aidoku_id, str) or not aidoku_id:
            raise ValueError(f"{source_json} is missing info.id")
        if not isinstance(name, str) or not name:
            raise ValueError(f"{source_json} is missing info.name")

        sources.append(
            AidokuSource(
                aidoku_id=aidoku_id,
                name=name,
                language=_language_from_info(aidoku_id, info),
            )
        )
    return sources


def load_converter_entries(converter_sources_json: Path) -> list[dict[str, Any]]:
    data = _read_json(converter_sources_json)
    if not isinstance(data, list):
        raise ValueError(f"{converter_sources_json} does not contain a JSON array")
    for index, entry in enumerate(data):
        if not isinstance(entry, dict):
            raise ValueError(f"{converter_sources_json}[{index}] is not an object")
    return data


def load_tachiyomi_sources(
    *,
    index_file: Path | None = None,
    index_url: str = tachiID.DEFAULT_INDEX_URL,
) -> list[Any]:
    index = (
        tachiID.load_index_from_file(index_file)
        if index_file is not None
        else tachiID.fetch_index(index_url)
    )
    return tachiID.flatten_sources(index)


def _compact(value: str) -> str:
    return "".join(character for character in value.casefold() if character.isalnum())


def _record_field(record: Any, field: str) -> str:
    value = getattr(record, field, "")
    return value if isinstance(value, str) else str(value)


def _record_tachi_id(record: Any) -> int | None:
    value = getattr(record, "tachi_id", None)
    return value if isinstance(value, int) else None


def _candidate_score(source: AidokuSource, record: Any) -> tuple[int, str, int] | None:
    if source.language is not None:
        record_language = _record_field(record, "lang").casefold()
        if record_language != source.language.casefold():
            return None

    record_id = _record_tachi_id(record)
    if record_id is None:
        return None

    source_name = source.name.casefold()
    source_name_compact = _compact(source.name)
    record_name = _record_field(record, "name")
    extension_name = _record_field(record, "extension_name")
    searchable = " ".join(
        [
            record_name,
            extension_name,
            _record_field(record, "package"),
            _record_field(record, "base_url"),
        ]
    ).casefold()

    if record_name.casefold() == source_name:
        return (0, record_name.casefold(), record_id)
    if extension_name.casefold() == source_name:
        return (1, extension_name.casefold(), record_id)
    if source_name_compact and _compact(record_name) == source_name_compact:
        return (2, record_name.casefold(), record_id)
    if source_name_compact and _compact(extension_name) == source_name_compact:
        return (3, extension_name.casefold(), record_id)

    query_terms = [term for term in source_name.split() if term]
    if query_terms and all(term in searchable for term in query_terms):
        return (4, record_name.casefold(), record_id)
    return None


def resolve_tachiyomi_id(
    source: AidokuSource,
    tachiyomi_sources: Iterable[Any],
) -> int | None:
    scored_records = [
        (score, record)
        for record in tachiyomi_sources
        if (score := _candidate_score(source, record)) is not None
    ]
    if not scored_records:
        return None

    scored_records.sort(key=lambda item: item[0])
    return _record_tachi_id(scored_records[0][1])


def update_converter_sources(
    *,
    aidoku_root: Path,
    converter_sources_json: Path,
    tachiyomi_sources: Iterable[Any],
) -> UpdateResult:
    aidoku_sources = load_aidoku_sources(aidoku_root)
    converter_entries = load_converter_entries(converter_sources_json)
    existing_aidoku_ids = {
        entry["aidoku"]
        for entry in converter_entries
        if isinstance(entry.get("aidoku"), str) and entry["aidoku"]
    }

    added_count = 0
    unmatched_count = 0
    tachiyomi_source_list = list(tachiyomi_sources)
    for source in sorted(aidoku_sources, key=lambda item: item.aidoku_id):
        if source.aidoku_id in existing_aidoku_ids:
            continue

        tachi_id = resolve_tachiyomi_id(source, tachiyomi_source_list)
        if tachi_id is None:
            unmatched_count += 1

        converter_entries.append(
            {
                "name": source.name,
                "aidoku": source.aidoku_id,
                "tachi": tachi_id,
            }
        )
        existing_aidoku_ids.add(source.aidoku_id)
        added_count += 1

    if added_count:
        _write_json(converter_sources_json, converter_entries)

    return UpdateResult(added_count=added_count, unmatched_count=unmatched_count)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Append missing Aidoku source mappings to converter sources.json.",
    )
    parser.add_argument(
        "--aidoku-root",
        type=Path,
        default=Path.cwd(),
        help="Path to the Aidoku sources repository root.",
    )
    parser.add_argument(
        "--converter-sources-json",
        type=Path,
        required=True,
        help="Path to Amqx/converter's sources.json file.",
    )
    parser.add_argument(
        "--index-file",
        type=Path,
        help="Read a local Keiyoushi index JSON file instead of fetching the URL.",
    )
    parser.add_argument(
        "--index-url",
        default=tachiID.DEFAULT_INDEX_URL,
        help=f"Keiyoushi extension index URL. Default: {tachiID.DEFAULT_INDEX_URL}",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    try:
        tachiyomi_sources = load_tachiyomi_sources(
            index_file=args.index_file,
            index_url=args.index_url,
        )
        result = update_converter_sources(
            aidoku_root=args.aidoku_root,
            converter_sources_json=args.converter_sources_json,
            tachiyomi_sources=tachiyomi_sources,
        )
    except (
        OSError,
        urllib.error.URLError,
        json.JSONDecodeError,
        ValueError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(
        "converter sources update: "
        f"added={result.added_count} unmatched_tachiyomi={result.unmatched_count}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
