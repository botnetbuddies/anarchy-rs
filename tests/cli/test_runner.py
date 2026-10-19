"""Ensure regression checks reject changed output and unexpected exits."""

import hashlib
import json
import unittest

from run import HERE, check_result


class RegressionChecks(unittest.TestCase):
    def test_exact_output(self):
        stdout = b"anna\nannakey\n"
        case = {"kind": "names", "stdout_sha256": hashlib.sha256(stdout).hexdigest()}
        self.assertIsNone(check_result(case, 0, stdout))
        for changed in [b"anna\n", b"annakey\nanna\n", stdout + b"anna\n"]:
            self.assertIsNotNone(check_result(case, 0, changed))
        self.assertIsNotNone(check_result(case, 1, stdout))

    def test_error_exit(self):
        case = {"kind": "error"}
        self.assertIsNone(check_result(case, 1, b""))
        self.assertIsNotNone(check_result(case, 0, b""))
        self.assertIsNotNone(check_result(case, -11, b""))

    def test_usage(self):
        case = {"kind": "usage"}
        self.assertIsNone(check_result(case, 0, b"Usage: anarchy-rs [OPTIONS]"))
        self.assertIsNotNone(check_result(case, 0, b"anna\n"))
        self.assertIsNotNone(check_result(case, 1, b"Usage: anarchy-rs [OPTIONS]"))

    def test_case_manifest(self):
        cases = json.loads((HERE / "cases.json").read_text())
        self.assertTrue(cases)
        self.assertEqual(len(cases), len({case["id"] for case in cases}))
        for case in cases:
            with self.subTest(case=case["id"]):
                self.assertIn(case["kind"], {"names", "listfmt", "recognise", "usage", "error"})
                self.assertTrue(all(isinstance(arg, str) for arg in case["args"]))
                if case["kind"] not in {"usage", "error"}:
                    self.assertRegex(case["stdout_sha256"], r"^[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
