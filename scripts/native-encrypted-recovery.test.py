"""Reject incomplete native encrypted-recovery and ciphertext evidence."""
import copy
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import uuid


spec = importlib.util.spec_from_file_location(
    "native_encrypted_recovery", Path(__file__).with_name("native-encrypted-recovery.py")
)
recovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recovery)


class EncryptedRecoveryAcceptanceTests(unittest.TestCase):
    def report(self, stage="encryptedSetup"):
        checks = recovery.SETUP_CHECKS if stage == "encryptedSetup" else recovery.VERIFY_CHECKS
        return {"passed": True, "recoveryStage": stage, "encryptedRecovery": {
            "checks": dict.fromkeys(checks, True),
            "invocations": dict.fromkeys(("connect", "prepareExecution", "execute"), 0),
            "groupId": str(uuid.uuid4()),
            "operationIds": [str(uuid.uuid4()) for _ in range(4)],
            "backups": [{"format": "encryptedV1", "metadata": {
                "id": str(uuid.uuid4()), "sizeBytes": 300, "digest": "a" * 64,
            }} for _ in range(2)],
        }}

    def errors(self, report, stage="encryptedSetup", code=0, timed_out=False):
        return recovery.acceptance_errors(report, stage, code, timed_out)

    def test_complete_evidence_requires_both_distinct_stages(self):
        for stage in recovery.STAGE_TIMEOUTS:
            self.assertEqual(self.errors(self.report(stage), stage), [])
        self.assertGreaterEqual(recovery.STAGE_TIMEOUTS["encryptedSetup"], 900)
        self.assertGreaterEqual(recovery.STAGE_TIMEOUTS["encryptedVerify"], 120)
        self.assertTrue(self.errors(self.report(), "setup"))
        self.assertTrue(self.errors(self.report(), "encryptedVerify"))

    def test_every_acl_lifecycle_and_reopen_check_is_mandatory(self):
        for stage, keys in (("encryptedSetup", recovery.SETUP_CHECKS),
                            ("encryptedVerify", recovery.VERIFY_CHECKS)):
            for key in keys:
                for missing in (None, False, 1, "true"):
                    report = self.report(stage)
                    report["encryptedRecovery"]["checks"][key] = missing
                    self.assertTrue(self.errors(report, stage), (stage, key, missing))

    def test_passing_report_does_not_hide_exit_failure_or_watchdog(self):
        for code, timed_out in ((1, False), (-9, False), (0, True)):
            self.assertTrue(self.errors(self.report(), code=code, timed_out=timed_out))

    def test_no_sql_counts_must_be_exact_non_boolean_zeroes(self):
        for key in ("connect", "prepareExecution", "execute"):
            for value in (None, True, False, 1, "0"):
                report = self.report()
                report["encryptedRecovery"]["invocations"][key] = value
                self.assertTrue(self.errors(report), (key, value))

    def test_original_operation_ids_cannot_be_missing_duplicated_or_noncanonical(self):
        for ids in ([], [str(uuid.uuid4())] * 4, ["missing"] * 4,
                    [str(uuid.uuid4()).upper() for _ in range(4)]):
            report = self.report()
            report["encryptedRecovery"]["operationIds"] = ids
            self.assertTrue(self.errors(report), ids)

    def test_only_two_exact_encrypted_artifact_identities_are_accepted(self):
        for field, value in (("format", "legacySqlite"), ("id", "../escape"),
                             ("sizeBytes", True), ("sizeBytes", 0),
                             ("digest", "A" * 64), ("digest", "not-a-digest")):
            report = self.report()
            backup = report["encryptedRecovery"]["backups"][0]
            (backup if field == "format" else backup["metadata"])[field] = value
            self.assertTrue(self.errors(report), (field, value))
        report = self.report()
        backups = report["encryptedRecovery"]["backups"]
        backups[1] = copy.deepcopy(backups[0])
        self.assertTrue(self.errors(report))
        backups.pop()
        self.assertTrue(self.errors(report))

    def test_malformed_evidence_never_passes(self):
        for report in (None, [], {}, {"passed": True},
                       {"passed": True, "recoveryStage": "encryptedSetup", "encryptedRecovery": []}):
            self.assertTrue(self.errors(report))


class CiphertextEvidenceTests(unittest.TestCase):
    valid_header = b"age-encryption.org/v1\n-> scrypt aabbccddeeff 18\nwrappedkey\n--- MAC\n"

    def fixture(self, root, payload=None):
        payload = self.valid_header + b"ciphertext" if payload is None else payload
        identity = str(uuid.uuid4())
        path = root / "backups" / f"{identity}.age"
        path.parent.mkdir(exist_ok=True)
        path.write_bytes(payload)
        backup = {"format": "encryptedV1", "metadata": {"id": identity,
                  "sizeBytes": len(payload), "digest": hashlib.sha256(payload).hexdigest()}}
        return path, backup

    def test_ciphertext_header_size_and_digest_are_independently_bound(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _, backup = self.fixture(root)
            errors, artifacts = recovery.artifact_evidence(root, [backup])
            self.assertEqual(errors, [])
            self.assertEqual(artifacts, [backup["metadata"]])
            for field, value in (("sizeBytes", 1), ("digest", "0" * 64)):
                changed = copy.deepcopy(backup)
                changed["metadata"][field] = value
                self.assertTrue(recovery.artifact_evidence(root, [changed])[0])

    def test_plaintext_wrong_work_factor_and_additional_recipient_fail(self):
        for payload in (b"SQLite format 3\x00plaintext", self.valid_header.replace(b" 18", b" 17"),
                        self.valid_header.replace(b"\n---", b"\n-> X25519 extra\nwrapped\n---"),
                        self.valid_header.replace(b"\n--- MAC\n", b"\n")):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                _, backup = self.fixture(root, payload)
                self.assertTrue(recovery.artifact_evidence(root, [backup])[0])

    def test_symlink_and_missing_ciphertext_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path, backup = self.fixture(root)
            target = root / "separate-ciphertext"
            path.rename(target)
            path.symlink_to(target)
            self.assertTrue(recovery.artifact_evidence(root, [backup])[0])
            path.unlink()
            self.assertTrue(recovery.artifact_evidence(root, [backup])[0])

    def test_noncanonical_artifact_identity_cannot_escape_owned_root(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertTrue(recovery.artifact_evidence(Path(directory), [
                {"metadata": {"id": "../outside"}}, None,
            ])[0])


if __name__ == "__main__":
    unittest.main()
