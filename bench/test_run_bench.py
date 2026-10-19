import unittest

from run_bench import summarize


class HyperfineExportTests(unittest.TestCase):
    def test_v1_export_has_timing_but_no_memory_measurement(self):
        result = {"command": "example", "min": 0.01, "mean": 0.02, "max": 0.04,
                  "times": [0.01, 0.01, 0.04], "stddev": 0.01}
        self.assertEqual(summarize(result),
                         {"min": 0.01, "mean": 0.02, "max": 0.04, "rss_bytes": None})

    def test_v2_export_keeps_memory_in_bytes(self):
        result = {"command": "example", "summary": {
            "time_wall_clock": {"min": 0.01, "mean": 0.02, "max": 0.04},
            "memory_peak_resident": {"min": 1024, "mean": 2048, "max": 4096},
        }}
        self.assertEqual(summarize(result),
                         {"min": 0.01, "mean": 0.02, "max": 0.04, "rss_bytes": 4096})


if __name__ == "__main__":
    unittest.main()
