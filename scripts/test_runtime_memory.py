import unittest
from runtime_memory import select_processes, summarize


class AttributionTests(unittest.TestCase):
    def test_shared_terminal_responsibility_never_counts_other_webviews(self):
        rows = [(10, "joinery"), (20, "com.apple.WebKit.WebContent"), (30, "other-app")]
        selected, status = select_processes(10, rows, lambda _: 999)
        self.assertEqual(selected, [(10, "main")])
        self.assertEqual(status, "shared_or_unavailable_responsibility")

    def test_launchd_helpers_need_responsibility_match_not_name_match(self):
        owners = {10: 10, 20: 10, 30: 30, 40: 10}
        rows = [(10, "joinery"), (20, "com.apple.WebKit.WebContent"),
                (30, "com.apple.WebKit.WebContent"), (40, "other-helper")]
        selected, status = select_processes(10, rows, owners.get)
        self.assertEqual(selected, [(10, "main"), (20, "webkit"), (40, "helper")])
        self.assertEqual(status, "responsibility_attributed")

    def test_unavailable_api_does_not_group_all_unknown_processes(self):
        selected, _ = select_processes(10, [(20, "WebKit")], lambda _: -1)
        self.assertEqual(selected, [(10, "main")])

    def test_missing_helpers_or_failed_reads_are_not_complete_totals(self):
        main = {"pid": 10, "role": "main", "rssBytes": 100, "physicalFootprintBytes": 50}
        helper = {"pid": 20, "role": "webkit", "rssBytes": 200, "physicalFootprintBytes": 80}
        result = summarize([main, helper], [], "responsibility_attributed")
        self.assertTrue(result["sampleComplete"])
        self.assertEqual(result["summedRssBytes"], 300)
        self.assertEqual(result["summedPhysicalFootprintBytes"], 130)
        self.assertFalse(summarize([main], [], "responsibility_attributed")["sampleComplete"])
        self.assertFalse(summarize([main, helper], [30], "responsibility_attributed")["sampleComplete"])
        self.assertFalse(summarize([main, helper], [], "shared_or_unavailable_responsibility")["sampleComplete"])


if __name__ == "__main__":
    unittest.main()
