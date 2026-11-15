"""Regression checks for the migration runner's verification oracle, not migration."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest


RUNNER = Path(__file__).with_name("test-history-schema-migration.py")
SPEC = importlib.util.spec_from_file_location("history_schema_migration", RUNNER)
MIGRATION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MIGRATION)


class SchemaOracleTests(unittest.TestCase):
    def state(self, version):
        tables = [f"{name}_v{version}" for name in ("profiles", "history", "audit")]
        if version >= 7:
            tables.append("outcome_operations_v7")
        if version >= 8:
            tables += ["batch_operations_v8", "batch_captures_v8",
                       "manual_transactions_v8", "manual_transaction_members_v8",
                       "manual_transaction_bindings_v8"]
        return {"userVersion": version, "tables": tables}

    def test_supported_physical_versions_are_accepted(self):
        for version in (6, 7, 8):
            with self.subTest(version=version):
                MIGRATION.assert_version(self.state(version), version)

    def test_retained_old_privacy_table_cannot_pass_new_schema(self):
        for name in ("profiles", "history", "audit"):
            with self.subTest(table=name):
                state = self.state(8)
                state["tables"].append(f"{name}_v7")
                with self.assertRaises(AssertionError):
                    MIGRATION.assert_version(state, 8)

    def test_missing_batch_facility_cannot_pass_new_schema(self):
        for name in self.state(8)["tables"]:
            if not (name.startswith("batch_") or name.startswith("manual_")):
                continue
            with self.subTest(table=name):
                state = self.state(8)
                state["tables"].remove(name)
                with self.assertRaises(AssertionError):
                    MIGRATION.assert_version(state, 8)

    def test_non_forward_pair_fails_before_binary_access(self):
        result = subprocess.run(
            [sys.executable, str(RUNNER), "--old-harness", "missing-harness",
             "--probe-old", "--old-schema", "7", "--new-schema", "7"],
            capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("requires a newer supported schema", result.stderr)
        self.assertNotIn("FileNotFoundError", result.stderr)


if __name__ == "__main__":
    unittest.main()
