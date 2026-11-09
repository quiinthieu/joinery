"""Prevent native recovery acceptance from passing on incomplete evidence."""
import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location("native_recovery", Path(__file__).with_name("native-recovery.py"))
recovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recovery)


class RecoveryAcceptanceTests(unittest.TestCase):
    def setup_report(self):
        return {"passed": True, "recoveryStage": "setup", "recovery": {
            "baselinePersisted": True, "cancelKeptSession": True,
            "cancelKeptTransaction": True, "draftMutationEpochMs": 1000,
        }}

    def test_pending_close_requires_exit_before_autosave_deadline(self):
        for elapsed in (0, 100, 399.99):
            self.assertEqual(recovery.acceptance_errors(self.setup_report(), "setup", 0, False, 1000 + elapsed), [])
        for elapsed in (-1, 400, 900):
            self.assertTrue(recovery.acceptance_errors(self.setup_report(), "setup", 0, False, 1000 + elapsed))

    def test_setup_needs_cancel_and_baseline_checks(self):
        for key in ("baselinePersisted", "cancelKeptSession", "cancelKeptTransaction"):
            report = self.setup_report()
            del report["recovery"][key]
            self.assertTrue(recovery.acceptance_errors(report, "setup", 0, False, 1100), key)

    def test_report_cannot_hide_process_failure_or_watchdog(self):
        for code, timed_out in ((1, False), (0, True), (-9, True)):
            self.assertTrue(recovery.acceptance_errors(self.setup_report(), "setup", code, timed_out, 1100))

    def test_reopen_requires_every_database_and_workspace_check(self):
        report = {"passed": True, "recoveryStage": "verify",
                  "recovery": dict.fromkeys(recovery.VERIFY_CHECKS, True)}
        report["recovery"]["invocationsBeforeObserver"] = dict.fromkeys(("connect", "prepareExecution", "execute"), 0)
        self.assertEqual(recovery.acceptance_errors(report, "verify", 0, False, 2000), [])
        for key in recovery.VERIFY_CHECKS:
            incomplete = {**report, "recovery": {**report["recovery"], key: False}}
            self.assertTrue(recovery.acceptance_errors(incomplete, "verify", 0, False, 2000), key)

    def test_no_replay_needs_zero_backend_invocations(self):
        for counts in ({}, None, {"connect": False, "prepareExecution": 0, "execute": 0},
                       {"connect": 1, "prepareExecution": 0, "execute": 0},
                       {"connect": 0, "prepareExecution": 0, "execute": 1}):
            report = {"passed": True, "recoveryStage": "verify", "recovery": {
                **dict.fromkeys(recovery.VERIFY_CHECKS, True), "invocationsBeforeObserver": counts,
            }}
            self.assertTrue(recovery.acceptance_errors(report, "verify", 0, False, 2000))

    def test_malformed_or_wrong_stage_evidence_fails(self):
        for report in ({}, {"passed": True, "recoveryStage": "setup", "recovery": []},
                       {"passed": True, "recoveryStage": "verify", "recovery": {}}):
            self.assertTrue(recovery.acceptance_errors(report, "setup", 0, False, 1100))
        for mutation in (True, None, "1000", float("nan")):
            report = self.setup_report()
            report["recovery"]["draftMutationEpochMs"] = mutation
            self.assertTrue(recovery.acceptance_errors(report, "setup", 0, False, 1100))


if __name__ == "__main__":
    unittest.main()
