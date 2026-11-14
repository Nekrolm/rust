import unittest
from pathlib import Path
import tempfile
from bench import CASES, diagnostics, edit, summarize


class HarnessTests(unittest.TestCase):
    def test_physical_edits(self):
        source = "struct State;\n\nfn generic<T>() { work(); }\n\nfn end() {}\n"
        cfg = {"generic_marker": "fn generic", "type_marker": "struct State",
               "compensated_end_marker": "fn end", "bottom_marker": "fn end",
               "body_replace": ["work();", "other();"]}
        for case in CASES:
            result = edit(source, case, cfg)
            if case in ("noop", "touch"):
                self.assertEqual(result, source)
            elif case == "compensated":
                self.assertEqual(result.count("\n"), source.count("\n"))
                self.assertEqual(result.splitlines().index("fn end() {}"), source.splitlines().index("fn end() {}"))
            elif case == "body":
                self.assertIn("other();", result)
                self.assertEqual(result.count("\n"), source.count("\n"))
            else:
                self.assertEqual(result.count("\n"), source.count("\n") + 1)

    def test_failed_builds_excluded_and_small_sample_disclosed(self):
        rows = [{"compiler": "baseline", "debug": 0, "opt": 0, "case": "top",
                 "exit_code": code, "wall_seconds": value} for code, value in [(0, 1), (0, 3), (1, 100)]]
        result, = summarize(rows)
        self.assertEqual(result["median_seconds"], 2)
        self.assertEqual(result["n"], 2)
        self.assertFalse(result["sufficient_repeated_samples"])

    def test_report_denominator_and_missing_report(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "build.log"
            log.write_text('rustc-relocation: {"schema":1,"event":"cgu","count":2,"outcome":"fallback","reason":"unsupported_lto","skipped":"none","eligible":false}\n'
                           'rustc-relocation: {"schema":1,"event":"cgu","count":1,"outcome":"accepted","reason":"validated_debug_refresh","skipped":"llvm_optimization","eligible":true}\n')
            report = diagnostics(log)
            self.assertEqual(report["eligible_candidates"], 1)
            self.assertEqual(report["fallback_fraction_of_all_cgus"], 2 / 3)
            log.write_text("ordinary Cargo log\n")
            report = diagnostics(log)
            self.assertIsNone(report["fallback_fraction_of_all_cgus"])
            self.assertIsNone(report["eligible_candidates"])


if __name__ == "__main__":
    unittest.main()
