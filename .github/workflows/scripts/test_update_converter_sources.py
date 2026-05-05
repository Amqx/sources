from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace


SCRIPT_DIR = Path(__file__).resolve().parent


def load_updater():
    module_path = SCRIPT_DIR / "update_converter_sources.py"
    if not module_path.exists():
        raise AssertionError("update_converter_sources.py does not exist")

    spec = importlib.util.spec_from_file_location("update_converter_sources", module_path)
    if spec is None or spec.loader is None:
        raise AssertionError("update_converter_sources.py could not be loaded")

    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_aidoku_source(root: Path, aidoku_id: str, name: str) -> None:
    source_json = root / "sources" / aidoku_id / "res" / "source.json"
    source_json.parent.mkdir(parents=True, exist_ok=True)
    source_json.write_text(
        json.dumps(
            {
                "info": {
                    "id": aidoku_id,
                    "name": name,
                    "version": 1,
                    "languages": [aidoku_id.split(".", 1)[0]],
                }
            }
        ),
        encoding="utf-8",
    )


class UpdateConverterSourcesTests(unittest.TestCase):
    def test_appends_missing_source_with_null_tachiyomi_id_when_no_match_exists(self):
        updater = load_updater()

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            write_aidoku_source(root, "en.present", "Present")
            write_aidoku_source(root, "en.nomatch", "No Match")
            converter_sources = root / "converter" / "sources.json"
            converter_sources.parent.mkdir()
            converter_sources.write_text(
                json.dumps(
                    [
                        {
                            "name": "Present",
                            "aidoku": "en.present",
                            "tachi": 123,
                        }
                    ]
                ),
                encoding="utf-8",
            )

            result = updater.update_converter_sources(
                aidoku_root=root,
                converter_sources_json=converter_sources,
                tachiyomi_sources=[],
            )

            entries = json.loads(converter_sources.read_text(encoding="utf-8"))
            self.assertEqual(result.added_count, 1)
            self.assertEqual(result.unmatched_count, 1)
            self.assertEqual(
                entries[-1],
                {
                    "name": "No Match",
                    "aidoku": "en.nomatch",
                    "tachi": None,
                },
            )

    def test_does_not_readd_source_when_aidoku_entry_already_has_null_tachiyomi_id(self):
        updater = load_updater()

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            write_aidoku_source(root, "en.nomatch", "No Match")
            converter_sources = root / "converter" / "sources.json"
            converter_sources.parent.mkdir()
            original_entries = [
                {
                    "name": "No Match",
                    "aidoku": "en.nomatch",
                    "tachi": None,
                }
            ]
            converter_sources.write_text(json.dumps(original_entries), encoding="utf-8")

            result = updater.update_converter_sources(
                aidoku_root=root,
                converter_sources_json=converter_sources,
                tachiyomi_sources=[],
            )

            entries = json.loads(converter_sources.read_text(encoding="utf-8"))
            self.assertEqual(result.added_count, 0)
            self.assertEqual(result.unmatched_count, 0)
            self.assertEqual(entries, original_entries)

    def test_resolves_tachiyomi_id_from_exact_source_name_and_language(self):
        updater = load_updater()

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            write_aidoku_source(root, "en.matchme", "Match Me")
            converter_sources = root / "converter" / "sources.json"
            converter_sources.parent.mkdir()
            converter_sources.write_text("[]", encoding="utf-8")
            tachiyomi_sources = [
                SimpleNamespace(
                    name="Match Me",
                    lang="en",
                    tachi_id=987654321,
                    extension_name="Match Me",
                    package="eu.kanade.tachiyomi.extension.en.matchme",
                    base_url="https://example.test",
                )
            ]

            result = updater.update_converter_sources(
                aidoku_root=root,
                converter_sources_json=converter_sources,
                tachiyomi_sources=tachiyomi_sources,
            )

            entries = json.loads(converter_sources.read_text(encoding="utf-8"))
            self.assertEqual(result.added_count, 1)
            self.assertEqual(result.unmatched_count, 0)
            self.assertEqual(entries[0]["tachi"], 987654321)


if __name__ == "__main__":
    unittest.main()
