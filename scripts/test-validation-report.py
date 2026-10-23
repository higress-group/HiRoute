#!/usr/bin/env python3
"""Regression diagnostics from #94-shaped logs; no Cargo/product validation claims."""
import importlib.util
import contextlib
import io
import json
from pathlib import Path
import tempfile
import types
import unittest

spec = importlib.util.spec_from_file_location("report", Path(__file__).with_name("validation-report.py"))
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


def target(name="p0_gateway_runtime", failed=False, seconds=42.11, test="reasoning::late_model"):
    return (f"     Running tests/{name}.rs (target/debug/deps/{name}-abcdef0123456789)\n"
            "running 1 test\n"
            f"test {test} ... {'FAILED' if failed else 'ok'}\n"
            f"test result: {'FAILED' if failed else 'ok'}. {0 if failed else 1} passed; "
            f"{1 if failed else 0} failed; 0 ignored; 0 measured; 31 filtered out; finished in {seconds}s\n")


def row(**kwargs):
    value = dict(id="run-a", plan="issue-94-b", phase="focused", sha="a" * 40,
                 process_exit=0, command=["cargo", "test", "-p", "hiroute-e2e", "--all-features"],
                 platform="linux", architecture="x86_64", rustc="rustc fixture", cargo="cargo fixture")
    value.update(kwargs)
    return value


class Parsing(unittest.TestCase):
    def test_nested_daemon_summary_is_not_added_to_parent(self):
        log = ("    Finished `test` profile [unoptimized + debuginfo] target(s) in 1m 09s\n"
               "     Running unittests src/lib.rs (target/debug/deps/hiroute_daemon-abcdef0123456789)\n"
               "running 2 tests\nrunning 1 test\ntest child ... ok\n"
               "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 181 filtered out; finished in 266.98s\n"
               "test parent ... ok\n"
               "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 267.00s\n")
        result = report.build_report(row(), log + target(seconds=220.11))
        self.assertTrue(result["timing_complete"])
        self.assertEqual([m["seconds"] for m in result["top_modules"]], [267., 220.11])
        self.assertEqual(result["modules"][0]["counts"]["passed"], 2)
        self.assertEqual(result["modules"][0]["child_summaries"], 1)
        self.assertEqual(result["build_events"], [dict(seconds=69., line=1, scope="outer")])

    def test_child_summary_does_not_complete_interrupted_parent(self):
        for count in (1, 2):
            with self.subTest(parent_count=count):
                log = ("Running unittests src/lib.rs (target/debug/deps/hiroute_daemon-abcdef0123456789)\n"
                       f"running {count} tests\nrunning 1 test\ntest child ... ok\n"
                       "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 20s\n")
                result = report.build_report(row(process_exit=-9), log)
                self.assertFalse(result["complete"])
                self.assertFalse(result["modules"][0]["complete"])
                self.assertEqual(result["top_modules"], [])

    def test_failure_and_unfinished_target_remain_distinct(self):
        log = target(failed=True) + "Running tests/next.rs (target/debug/deps/next-abcdef0123456789)\nrunning 1 test\n"
        result = report.build_report(row(process_exit=101), log)
        self.assertFalse(result["complete"])
        self.assertEqual(result["modules"][0]["failures"], ["reasoning::late_model"])
        self.assertTrue(result["modules"][0]["complete"])
        self.assertIsNone(result["modules"][1]["seconds"])
        self.assertFalse(result["modules"][1]["complete"])

    def test_compile_failure_has_no_invented_test_duration(self):
        result = report.build_report(row(process_exit=101), "error[E0425]: missing symbol\n")
        self.assertFalse(result["complete"])
        self.assertEqual(result["modules"], [])
        self.assertIsNone(result["timing"]["command_seconds"])

    def test_multiple_failed_binaries_are_retained(self):
        result = report.build_report(row(process_exit=101), target(failed=True) + target("routing_plans", True, test="golden"))
        self.assertEqual([m["failures"] for m in result["modules"]], [["reasoning::late_model"], ["golden"]])

    def test_94_same_named_targets_have_separate_timings_but_ambiguous_cross_run_identity(self):
        log = target() + target().replace("abcdef0123456789", "123456789abcdef0")
        result = report.build_report(row(), log)
        self.assertTrue(result["complete"])
        self.assertEqual(len(result['modules']), 2)
        self.assertNotEqual(result['modules'][0]['module'], result['modules'][1]['module'])
        self.assertTrue(all(not m['comparison_identity_unique'] for m in result['modules']))
        failed_log = target(failed=True) + target(failed=True).replace("abcdef0123456789", "123456789abcdef0")
        final = report.build_report(row(phase='final', process_exit=101), failed_log,
                                    [report.build_report(row(), target())])
        self.assertTrue(all(f['focused_observation'] == 'ambiguous_target_identity' for f in final['findings']))

    def test_repeated_identical_binary_is_not_assumed_independent(self):
        result = report.build_report(row(), target() + target())
        self.assertFalse(result["complete"])
        self.assertTrue(all(not m["complete"] for m in result["modules"]))

    def test_unassigned_summary_is_not_attributed_to_a_module(self):
        result = report.build_report(row(), "test result: ok. 1 passed; 0 failed; 0 ignored\n")
        self.assertFalse(result["complete"])
        self.assertEqual(result["modules"], [])
        self.assertEqual(len(result["unassigned_summaries"]), 1)

    def test_ansi_doc_tests_and_missing_time(self):
        log = "\x1b[32m   Doc-tests hiroute_domain\x1b[0m\nrunning 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
        result = report.build_report(row(), log)
        self.assertTrue(result["complete"])
        self.assertFalse(result["timing_complete"])
        self.assertIsNone(result["modules"][0]["seconds"])

    def test_unassigned_summary_keeps_otherwise_successful_report_incomplete(self):
        log = "test result: ok. 1 passed; 0 failed; 0 ignored\n" + target()
        self.assertFalse(report.build_report(row(), log)["observed_tests_passed"])


class Feedback(unittest.TestCase):
    def test_harness_settings_distinguish_omitted_environment_argument_and_unknown(self):
        command = ["cargo", "test", "--locked"]
        omitted = report.harness_concurrency(command, {})
        self.assertEqual(omitted["requested_threads"], [])
        self.assertTrue(omitted["environment_recorded"])
        self.assertEqual(omitted["configuration_source"], "default")
        self.assertIsNone(omitted["configured_thread_limit"])
        old = report.context(row(cpu_count=16))["harness_concurrency"]
        self.assertFalse(old["environment_recorded"])
        self.assertEqual(old["configuration_source"], "unknown")
        self.assertIsNone(old["effective_threads"])
        for arguments in (["--test-threads=2"], ["--test-threads", "2"]):
            context = report.harness_concurrency([*command, "--", *arguments], {"RUST_TEST_THREADS": "8"})
            self.assertEqual(context["requested_threads"], ["2"])
            self.assertEqual(context["environment_threads"], "8")
            self.assertEqual(context["configuration_source"], "argument")
            self.assertEqual(context["configured_thread_limit"], 2)
            self.assertIsNone(context["effective_threads"])

    def test_ambiguous_or_invalid_thread_setting_does_not_invent_limit(self):
        for arguments in (["--test-threads"], ["--test-threads=0"], ["--test-threads=oops"],
                          ["--test-threads=1", "--test-threads=2"]):
            context = report.harness_concurrency(["cargo", "test", "--", *arguments], {"RUST_TEST_THREADS": "8"})
            self.assertIsNone(context["configured_thread_limit"])
            self.assertIsNone(context["effective_threads"])
        self.assertIsNone(report.harness_concurrency(["cargo", "check"], {}))

    def test_same_command_with_different_launch_environment_reports_context_gap(self):
        command = row()["command"]
        focused = report.build_report(row(harness_concurrency=report.harness_concurrency(
            command, {"RUST_TEST_THREADS": "1"})), target())
        final = report.build_report(row(phase="final", process_exit=101,
                                       harness_concurrency=report.harness_concurrency(command, {})),
                                    target(failed=True), [focused])
        self.assertEqual(final["findings"][0]["context_differences"], ["harness_concurrency"])

    def test_child_failures_are_not_attributed_to_parent_regressions(self):
        focused = report.build_report(row(), target())
        # A parent can intentionally spawn a failing test and still pass its own assertion.
        nested = target(failed=True) + "test result: ok. 1 passed; 0 failed; 0 ignored; finished in 43s\n"
        final = report.build_report(row(phase="final", process_exit=101), nested, [focused])
        self.assertEqual([f["kind"] for f in final["findings"]], ["full_failure_unattributed"])
        earlier = report.build_report(row(), nested)
        final = report.build_report(row(phase="final", process_exit=101), target(failed=True), [earlier])
        self.assertEqual(final["findings"][0]["focused_observation"], "ambiguous_child_output")

    def test_94_package_pass_workspace_failure_is_context_gap_not_flake(self):
        focused = report.build_report(row(), target())
        final = report.build_report(row(phase="final", process_exit=101,
                                        command=["cargo", "test", "--workspace", "--exclude", "hiroute-desktop", "--all-features"]),
                                    target(failed=True), [focused])
        finding = final["findings"][0]
        self.assertEqual(finding["kind"], "focused_full_gap")
        self.assertEqual(finding["focused_observation"], "ok")
        self.assertEqual(finding["context_differences"], ["package_scope"])
        self.assertNotIn("flaky", json.dumps(final))

    def test_missing_case_and_ignored_case_are_not_success(self):
        for focused_log, expected in [(target(test="different"), "not_observed"),
                                      (target().replace("... ok", "... ignored").replace("1 passed; 0 failed; 0 ignored", "0 passed; 0 failed; 1 ignored"), "ignored")]:
            focused = report.build_report(row(), focused_log + target('other_target', test='other_passing_case'))
            final = report.build_report(row(phase="final", process_exit=101), target(failed=True), [focused])
            self.assertEqual(final["findings"][0]["focused_observation"], expected)

    def test_prior_sha_or_other_plan_is_not_same_candidate_evidence(self):
        focused = report.build_report(row(), target())
        for changed, expected in [(dict(sha="b" * 40), "different_revision"), (dict(plan="another"), "unrelated_plan")]:
            result = report.build_report(row(phase="final", process_exit=101, **changed), target(failed=True), [focused])
            self.assertEqual([f["kind"] for f in result["findings"]], [expected])

    def test_same_flags_with_missing_toolchain_remain_unknown(self):
        focused = report.build_report(row(rustc=None), target())
        result = report.build_report(row(phase="final", process_exit=101, rustc=None), target(failed=True), [focused])
        self.assertIn("rustc", result["findings"][0]["unknown_context"])

    def test_failed_focused_run_does_not_establish_escape(self):
        focused = report.build_report(row(process_exit=101), target(failed=True))
        result = report.build_report(row(phase="final", process_exit=101), target(failed=True), [focused])
        self.assertEqual(result["findings"], [])

    def test_repeated_final_warns_without_changing_exit(self):
        previous = report.build_report(row(phase="final"), target())
        result = report.build_report(row(phase="final"), target(), [previous])
        self.assertEqual(result["findings"][0]["kind"], "repeated_final")
        self.assertEqual(result["process_exit"], 0)
        record = row(phase='final')
        with contextlib.redirect_stderr(io.StringIO()) as error:
            report.preflight(record, [previous])
        self.assertIn('Repeated final', error.getvalue())
        self.assertIn('validation_advice', record)

    def test_zero_selected_focused_run_is_not_coverage(self):
        focused = report.build_report(row(), target().replace('running 1 test', 'running 0 tests')
                                      .replace('1 passed', '0 passed').replace('test reasoning::late_model ... ok\n', ''))
        result = report.build_report(row(phase='final', process_exit=101), target(failed=True), [focused])
        self.assertFalse(focused['observed_tests_passed'])
        self.assertEqual(result['findings'], [])

    def test_final_failure_without_test_output_requests_diagnosis(self):
        focused = report.build_report(row(), target())
        result = report.build_report(row(phase="final", process_exit=101), "compile error", [focused])
        self.assertEqual(result["findings"][0]["kind"], "full_failure_unattributed")


class Storage(unittest.TestCase):
    def test_report_survives_checkout_removal_and_never_changes_verdict(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            run = root / "run-a"
            run.mkdir()
            (run / "command.log").write_text(target())
            original = row(phase="final", scenario="expected_red")
            info = report.write_report(run, original)
            self.assertEqual(original["scenario"], "expected_red")
            self.assertTrue(Path(info["markdown"]).is_file())
            loaded = report.related_reports(root, dict(plan="issue-94-b", related_runs=["run-a"]))
            self.assertEqual(loaded[0]["sha"], original["sha"])
            self.assertTrue(info["follow_up"])
            with self.assertRaisesRegex(ValueError, "another plan"):
                report.related_reports(root, dict(plan="other", related_runs=["run-a"]))

    def test_related_id_cannot_escape_run_store(self):
        for value in ("../secret", "/tmp/secret", "a/b"):
            with self.assertRaises(ValueError):
                report.options(types.SimpleNamespace(plan="task", related_run=[value]))
        with self.assertRaises(ValueError):
            report.options(types.SimpleNamespace(related_run=["id"]))

    def test_report_write_failure_does_not_mask_process_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            record = row(process_exit=101)
            info = report.write_report(Path(folder) / "missing", record)
            self.assertFalse(info["complete"])
            self.assertIn("error", info)
            self.assertEqual(record["process_exit"], 101)

    def test_smoke_import_requires_owned_path_and_matching_revision(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            checkout = root / "checkout"
            checkout.mkdir()
            value = dict(schema="hiroute.smoke.result/v1", source_revision="a" * 40, run_id="smoke-1",
                         cases=[dict(id="gateway", execution="executed", state="green", build_ms=120,
                                     execution_ms=50, unclassified_ms=0, timing_complete=True, secret="not copied")])
            path = checkout / "smoke.json"
            path.write_text(json.dumps(value))
            result = report.smoke_timings("smoke report=" + str(path), checkout, "a" * 40)
            self.assertEqual(result[0]["cases"][0]["build_ms"], 120)
            self.assertNotIn("secret", result[0]["cases"][0])
            self.assertEqual(report.smoke_timings("smoke report=" + str(path), checkout, "b" * 40), [])
            outside = root / "outside.json"
            outside.write_text(json.dumps(value))
            (checkout / "link").symlink_to(outside)
            self.assertEqual(report.smoke_timings("smoke report=" + str(checkout / "link"), checkout, "a" * 40), [])

    def test_successful_smoke_captured_stdout_uses_current_command_window(self):
        with tempfile.TemporaryDirectory() as folder:
            checkout = Path(folder)
            for name, start in [('current', 1100), ('old', 100)]:
                directory = checkout / 'target/smoke' / name
                directory.mkdir(parents=True)
                (directory / 'report.json').write_text(json.dumps(dict(schema='hiroute.smoke.result/v1',
                    source_revision='a' * 40, started_unix_ms=start, finished_unix_ms=start + 100,
                    run_id=name, cases=[])))
            values = report.smoke_timings('', checkout, 'a' * 40, 1, 2)
            self.assertEqual([v['run_id'] for v in values], ['current'])
            self.assertEqual(report.smoke_timings('', checkout, 'a' * 40), [])


if __name__ == "__main__":
    unittest.main()
