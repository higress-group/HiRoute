#!/usr/bin/env python3
"""Exercise generation with synthetic descriptive growth; never write maintained output."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("current_model_generator", HERE / "generate.py")
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)


class ModelDataSizeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.current = GENERATOR.compile_candidate()
        cls.bundle = json.loads(cls.current["model-data.json"])

    def synthetic_catalog(self, target_bytes):
        catalog = copy.deepcopy(self.bundle["metadata_catalog"])
        originals = copy.deepcopy(catalog["model_records"])
        catalog["source_catalog_digest"] = GENERATOR.digest("synthetic-size-regression")
        bundle = {**self.bundle, "metadata_catalog": catalog}
        size = len(GENERATOR.canonical(bundle)) + 1
        index = 0
        while size < target_bytes:
            record = copy.deepcopy(originals[index % len(originals)])
            record["model_record_key"] = f"synthetic/{index:06}/{record['model_record_key']}"
            catalog["model_records"].append(record)
            size += len(GENERATOR.canonical(record)) + 1
            index += 1
        catalog["model_records"].sort(key=lambda value: value["model_record_key"])
        return catalog

    def test_complete_output_above_former_limit_remains_deterministic(self):
        catalog = self.synthetic_catalog(8 * 1024 * 1024 - 16 * 1024)
        with patch.object(GENERATOR, "project_metadata_catalog", return_value=catalog):
            first = GENERATOR.compile_candidate()
            second = GENERATOR.compile_candidate()
        self.assertEqual(first, second)
        payload = first["model-data.json"]
        self.assertGreater(len(payload), 2 * 1024 * 1024)
        self.assertLessEqual(len(payload), 8 * 1024 * 1024)
        self.assertTrue(payload.endswith(b"\n"))
        self.assertEqual(json.loads(payload)["metadata_catalog"], catalog)
        self.assertEqual(first["runtime-projection.json"], self.current["runtime-projection.json"])
        self.assertEqual(first["coverage.json"], self.current["coverage.json"])

    def test_complete_output_above_eight_mib_is_rejected(self):
        catalog = self.synthetic_catalog(8 * 1024 * 1024 + 1)
        with patch.object(GENERATOR, "project_metadata_catalog", return_value=catalog):
            with self.assertRaisesRegex(ValueError, "8 MiB model-data limit"):
                GENERATOR.compile_candidate()


if __name__ == "__main__":
    unittest.main()
