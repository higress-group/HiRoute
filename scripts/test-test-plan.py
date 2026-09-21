#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest
import sys
import json
import subprocess
import tempfile

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location("test_plan", Path(__file__).with_name("test-plan.py"))
plan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plan)


class SelectionTests(unittest.TestCase):
    def test_validation_tooling_selects_runner_consumers_without_product_builds(self):
        result = plan.select(['scripts/validation.py', 'scripts/local-rust.py', 'scripts/pilot-builds.py',
                              'scripts/desktop-pilot.py', 'docs/validation-routing.md'])
        self.assertFalse(result['rust'])
        self.assertFalse(result['frontend'])
        for name in ('test-validation.py', 'test-local-rust.py', 'test-remote-rust.py',
                     'test-ci-shards.py', 'test-desktop-pilot.py', 'test-pilot-builds.py'):
            self.assertIn(['python3', 'scripts/' + name], result['commands'])
        mixed = plan.select(['scripts/validation.py', 'crates/daemon/src/lib.rs'])
        self.assertTrue(mixed['rust'])
        self.assertIn('daemon', mixed['groups'])

    def test_hosted_backend_execution_contract_runs_full_matrix(self):
        for path in sorted(plan.HOSTED_BACKEND_EXECUTION):
            with self.subTest(path=path):
                result = plan.select([path])
                self.assertEqual(result['mode'], 'full')
                self.assertTrue(result['rust'])
                self.assertTrue(result['frontend'])
                self.assertIn('--workspace', result['commands'][2])
                self.assertIn('hosted backend execution contract: ' + path, result['reasons'])
                self.assertIn(['python3', 'scripts/test-ci-shards.py'], result['commands'])

    def test_docs_do_not_build_rust(self):
        result = plan.select(["docs/testing.md", "README.md", "AGENTS.md"])
        self.assertFalse(result["rust"])
        self.assertFalse(result["frontend"])

    def test_frontend_has_no_native_or_rust_claim(self):
        result = plan.select(["apps/desktop/src/product/Models.tsx"])
        self.assertTrue(result["frontend"])
        self.assertFalse(result["rust"])
        self.assertFalse(result["native_required"])

    def test_website_and_publication_use_their_dedicated_workflow(self):
        result = plan.select([
            "apps/website/src/pages/index.astro",
            "apps/website/package-lock.json",
            ".github/workflows/website.yml",
            ".github/workflows/release.yml",
            ".github/scripts/deploy-website-oss.sh",
        ])
        self.assertEqual(result["mode"], "affected")
        self.assertFalse(result["rust"])
        self.assertFalse(result["frontend"])
        self.assertFalse(result["native_required"])
        self.assertIn(["python3", "scripts/test-contract-convergence.py"], result["commands"])

    def test_native_changes_require_platform_evidence(self):
        result = plan.select(["apps/desktop/src-tauri/src/main.rs"])
        self.assertTrue(result["native_required"])
        self.assertEqual(result["mode"], "affected")
        self.assertIn("cli", result["groups"])
        self.assertTrue(result["frontend"])

    def test_mixed_changes_union_groups(self):
        result = plan.select(["crates/cli/src/worker.rs", "crates/observation/src/lib.rs"])
        self.assertEqual(result["groups"], ["cli", "observation"])
        test = result["commands"][2]
        self.assertEqual(test.count("hiroute-daemon"), 1)

    def test_shared_unknown_and_dependencies_fall_back(self):
        for path in ["Cargo.lock", "crates/cli/Cargo.toml", "crates/domain/src/lib.rs",
                     "scripts/unknown-runner.py", "unknown/input", "assets/skills/hiroute-routing/SKILL.md"]:
            with self.subTest(path=path):
                self.assertEqual(plan.select([path])["mode"], "full")

    def test_gateway_preserves_distinct_feature_check(self):
        result = plan.select(["crates/gateway/src/lib.rs"])
        self.assertTrue(any("--no-default-features" in command for command in result["commands"]))
        self.assertIn("hiroute-e2e", result["commands"][2])

    def test_full_does_not_repeat_product_smoke(self):
        result = plan.select([], full=True)
        self.assertTrue(result["frontend"])
        self.assertIn("--workspace", result["commands"][2])
        self.assertFalse(any("smoke_cli" in command for command in result["commands"]))

    def test_business_scopes_have_core_smoke(self):
        for owner, group in plan.OWNERS.items():
            if group not in plan.PRODUCT_GROUPS:
                continue
            result = plan.select(["crates/" + owner + "/src/lib.rs"])
            self.assertTrue(any("smoke_cli" in command for command in result["commands"]))

    def test_indirect_consumers_are_selected(self):
        for owner in ("gateway", "integrations"):
            command = plan.select(["crates/" + owner + "/src/lib.rs"])["commands"][2]
            for package in ("hiroute-cpa-bridge", "hiroute-local-storage", "hiroute-release-facts", "hiroute-cli"):
                self.assertIn(package, command)

    def test_runtime_markdown_is_not_docs_only(self):
        self.assertEqual(plan.select(["crates/daemon/src/prompt.md"])["mode"], "full")

    def test_every_changed_scope_runs_contract_convergence(self):
        for paths in (
            ["docs/testing.md"],
            ["apps/desktop/src/App.tsx"],
            ["crates/domain/src/lib.rs"],
        ):
            with self.subTest(paths=paths):
                self.assertIn(
                    ["python3", "scripts/test-contract-convergence.py"],
                    plan.select(paths)["commands"],
                )
                self.assertIn(
                    ["python3", "scripts/check-contract-convergence.py"],
                    plan.select(paths)["commands"],
                )


    def test_diagnostic_internal_change_is_focused(self):
        for path in ("crates/diagnostics/src/level.rs", "crates/diagnostics/src/runtime.rs",
                     "crates/diagnostics/src/settings.rs", "crates/diagnostics/tests/writer_bounds.rs"):
            with self.subTest(path=path):
                result = plan.select([path, "AGENTS.md", "docs/diagnostic-troubleshooting.md"])
                self.assertEqual(result["mode"], "affected")
                self.assertTrue(result["rust"])
                self.assertFalse(result["frontend"])
                self.assertFalse(result["native_required"])
                self.assertEqual(result["groups"], ["diagnostics"])
                self.assertIn("hiroute-diagnostics", result["commands"][2])
                self.assertFalse(any("--workspace" in c or "smoke_cli" in c for c in result["commands"]))
                self.assertIn(["python3", "scripts/test-desktop-pilot.py"], result["commands"])

    def test_default_level_includes_release_regression(self):
        result = plan.select(["crates/diagnostics/src/level.rs"])
        release = [c for c in result["commands"] if "--release" in c]
        self.assertEqual(len(release), 1)
        self.assertIn("runtime::tests::unconfigured_runtime_uses_build_default_without_persisting", release[0])
        self.assertIn("--exact", release[0])
        writer = plan.select(["crates/diagnostics/src/writer.rs"])
        self.assertFalse(any("--release" in c for c in writer["commands"]))

    def test_native_dependency_change_still_expands(self):
        result = plan.select(["apps/desktop/src-tauri/Cargo.toml"])
        self.assertEqual(result["mode"], "full")
        self.assertTrue(result["native_required"])

    def test_diagnostic_contract_still_expands(self):
        for path in ("crates/diagnostics/src/event/worker.rs", "crates/diagnostics/src/record.rs",
                     "crates/diagnostics/src/identity.rs", "crates/diagnostics/src/error.rs"):
            self.assertEqual(plan.select([path])["mode"], "full")

    def test_diagnostics_mixed_with_business_preserves_smoke(self):
        result = plan.select(["crates/diagnostics/src/level.rs", "crates/daemon/src/control.rs"])
        self.assertEqual(result["groups"], ["daemon", "diagnostics"])
        self.assertTrue(any("smoke_cli" in c for c in result["commands"]))
        self.assertIn("hiroute-daemon", result["commands"][2])

    def test_selection_tooling_requires_only_python(self):
        result = plan.select(["scripts/test-plan.py", "scripts/test-test-plan.py", "AGENTS.md"])
        self.assertFalse(result["rust"])
        self.assertFalse(result["frontend"])
        self.assertIn(["python3", "scripts/test-test-plan.py"], result["commands"])
        mixed = plan.select(["scripts/test-plan.py", "crates/diagnostics/src/level.rs"])
        self.assertEqual(mixed["groups"], ["diagnostics"])
        self.assertFalse(mixed["frontend"])
        unknown = plan.select(["scripts/test-plan.py", "unknown/file"])
        self.assertEqual(unknown["mode"], "full")

    def test_explicit_full_overrides_focused_classification(self):
        result = plan.select(["crates/diagnostics/src/level.rs", "scripts/test-plan.py"], full=True)
        self.assertEqual(result["mode"], "full")
        self.assertIn("--workspace", result["commands"][2])

    def test_94_runtime_repair_is_focused_and_keeps_coverage_gate(self):
        result = plan.select(["tools/e2e-harness/tests/p0_gateway_runtime/reasoning.rs"])
        self.assertEqual(result['mode'], 'affected')
        tests = [c for c in result['commands'] if c[:2] == ['cargo', 'test']]
        self.assertEqual({c[c.index('--test') + 1] for c in tests}, {'p0_gateway_runtime', 'p0_gateway_protocol', 'p0_gateway_matrix_coverage'})
        self.assertTrue(result['rust'])
        self.assertFalse(result['frontend'])
        runtime = next(c for c in tests if 'p0_gateway_runtime' in c)
        self.assertIn('--test-threads=1', runtime)
        self.assertEqual(runtime[runtime.index('--features') + 1], 'serde_json/preserve_order')
        self.assertNotIn('--test-threads=1', next(c for c in tests if 'p0_gateway_matrix_coverage' in c))

    def test_shared_harness_and_dependencies_remain_full(self):
        for path in ['tools/e2e-harness/tests/runtime_support/mod.rs', 'tools/e2e-harness/Cargo.toml',
                     'tools/product-e2e/src/smoke/process.rs', 'e2e/schema/current-wire.json']:
            self.assertEqual(plan.select([path])['mode'], 'full')

    def test_responses_helper_includes_indirect_smoke_consumer(self):
        result = plan.select(['tools/e2e-harness/src/p0/client.rs'])
        commands = result['commands']
        self.assertTrue(any('--lib' in c for c in commands))
        self.assertTrue(any('p0_gateway_protocol' in c for c in commands))
        self.assertTrue(any('p0_gateway_oracle' in c for c in commands))
        self.assertTrue(any('smoke_cli' in c for c in commands))

    def test_routing_fixture_does_not_select_other_product_journeys(self):
        result = plan.select(['e2e/product/golden/routing/compiled-publication.v2.json'])
        tests = [c for c in result['commands'] if c[:2] == ['cargo', 'test']]
        self.assertEqual(len(tests), 1)
        self.assertIn('routing_plans', tests[0])

    def test_full_and_mixed_do_not_duplicate_already_selected_targets(self):
        path = 'tools/e2e-harness/tests/p0_gateway_runtime/reasoning.rs'
        for paths in ([path, 'Cargo.lock'], [path, 'crates/gateway/src/lib.rs']):
            result = plan.select(paths)
            self.assertFalse(any('p0_gateway_runtime' in c for c in result['commands']))
        full = plan.select([path], full=True)
        self.assertTrue(full['execution']['remote_exclusive'])
        self.assertNotIn('--no-fail-fast', full['commands'][2])

    def test_default_plans_do_not_delay_terminal_failure_feedback(self):
        for paths in (['Cargo.lock'], ['crates/daemon/src/lib.rs'],
                      ['tools/e2e-harness/tests/p0_gateway_runtime/reasoning.rs'],
                      ['tools/e2e-harness/src/p0/client.rs']):
            self.assertTrue(all('--no-fail-fast' not in c for c in plan.select(paths)['commands']))

    def test_bounded_consumers_keep_one_clippy_gate_per_package(self):
        paths = ['tools/e2e-harness/tests/p0_gateway_runtime/reasoning.rs',
                 'tools/e2e-harness/src/p0/client.rs', 'tools/product-e2e/tests/routing_plans.rs']
        result = plan.select(paths)
        checks = [c for c in result['commands'] if c[:2] == ['cargo', 'clippy']]
        self.assertEqual([c[c.index('-p') + 1] for c in checks], ['hiroute-e2e', 'hiroute-product-e2e'])
        for c in checks:
            self.assertIn('--all-targets', c)
            self.assertIn('--all-features', c)
            self.assertEqual(c[-3:], ['--', '-D', 'warnings'])
        self.assertIn('serde_json/preserve_order', checks[0])
        self.assertNotIn('serde_json/preserve_order', checks[1])
        for extra in ('Cargo.lock', 'crates/gateway/src/lib.rs'):
            mixed = plan.select([*paths, extra])
            checks = [c for c in mixed['commands'] if c[:2] == ['cargo', 'clippy']]
            # Full/groups already check hiroute-e2e; only uncovered product targets add a gate.
            self.assertEqual(sum('hiroute-e2e' in c for c in checks), 0 if extra == 'Cargo.lock' else 1)


class IntegrationPreflightTests(unittest.TestCase):
    def test_merge_with_lockfile_keeps_early_checks_and_final_obligations(self):
        paths = ['Cargo.lock', 'tools/e2e-harness/tests/p0_gateway_runtime.rs',
                 'crates/gateway/src/adapters/ingress.rs']
        final = plan.select(paths)
        original = json.loads(json.dumps(final))
        early = plan.integration_preflight(final)
        self.assertEqual(final, original)
        self.assertEqual(final['mode'], 'full')
        tests = [c for c in early['commands'] if c[:2] == ['cargo', 'test']]
        self.assertEqual(tests[0][-2:], ['--test', 'p0_gateway_matrix_coverage'])
        self.assertIn('p0_gateway_protocol', tests[1])
        self.assertIn('p0_gateway_runtime', tests[1])
        for command in tests:
            self.assertIn('--workspace', command)
            self.assertIn('hiroute-desktop', command)
            self.assertIn('--all-features', command)
        self.assertFalse(any('smoke_cli' in c or '--no-fail-fast' in c for c in early['commands']))
        self.assertTrue(early['remote_exclusive'])

    def test_production_protocol_changes_have_early_consumers(self):
        for path in ['crates/gateway/src/adapters/responses_ingress.rs',
                     'crates/gateway/src/model_ir/request.rs']:
            final = plan.select([path])
            early = plan.integration_preflight(final)
            self.assertEqual(set(early['targets']), {'p0_gateway_protocol', 'p0_gateway_matrix_coverage'})
            self.assertIn('gateway', final['groups'])
            self.assertTrue(any('smoke_cli' in c for c in final['commands']))
            test = next(c for c in early['commands'] if 'p0_gateway_protocol' in c)
            self.assertIn('hiroute-cpa-bridge', test)

    def test_runtime_repair_selects_protocol_consumer_before_full(self):
        final = plan.select(['tools/e2e-harness/tests/p0_gateway_runtime.rs'])
        self.assertTrue(any('p0_gateway_protocol' in c for c in final['commands']))
        early = plan.integration_preflight(final)
        tests = [c for c in early['commands'] if c[:2] == ['cargo', 'test']]
        self.assertEqual(len(tests), 2)
        self.assertIn('serde_json/preserve_order', tests[1])
        self.assertIn('--test-threads=1', tests[1])

    def test_manifest_producer_keeps_full_but_checks_coverage_first(self):
        final = plan.select(['tools/e2e-harness/src/p0/coverage.rs'])
        self.assertEqual(final['mode'], 'full')
        early = plan.integration_preflight(final)
        self.assertEqual(early['targets'], ['p0_gateway_matrix_coverage'])
        self.assertEqual(early['commands'][-1][-2:], ['--test', 'p0_gateway_matrix_coverage'])

    def test_frontend_native_and_smoke_obligations_survive_preflight(self):
        final = plan.select(['apps/desktop/src-tauri/src/main.rs',
                             'tools/e2e-harness/tests/p0_gateway_protocol.rs'])
        early = plan.integration_preflight(final)
        self.assertTrue(final['frontend'])
        self.assertTrue(final['native_required'])
        self.assertTrue(any('smoke_cli' in c for c in final['commands']))
        self.assertFalse(any('smoke_cli' in c for c in early['commands']))

    def test_mixed_cli_and_e2e_does_not_drop_the_test_owner(self):
        early = plan.integration_preflight(plan.select([
            'crates/cli/src/main.rs', 'tools/e2e-harness/tests/p0_gateway_runtime.rs']))
        for command in early['commands']:
            if command[:2] == ['cargo', 'test']:
                for package in ['hiroute-cli', 'hiroute-client-core', 'hiroute-daemon', 'hiroute-e2e']:
                    self.assertIn(package, command)
                self.assertIn('serde_json/preserve_order', command)

    def test_explicit_collection_only_changes_bounded_behavior_command(self):
        final = plan.select(['Cargo.lock', 'tools/e2e-harness/tests/p0_gateway_runtime.rs'])
        early = plan.integration_preflight(final, collect_failures=True)
        collection = [c for c in early['commands'] if '--no-fail-fast' in c]
        self.assertEqual(len(collection), 1)
        self.assertIn('p0_gateway_protocol', collection[0])
        self.assertIn('p0_gateway_runtime', collection[0])
        self.assertNotIn('p0_gateway_matrix_coverage', collection[0])
        self.assertTrue(all('--no-fail-fast' not in c for c in final['commands']))

    def test_unknown_changes_keep_full_without_inventing_early_coverage(self):
        final = plan.select(['unknown/contract'])
        self.assertEqual(final['mode'], 'full')
        early = plan.integration_preflight(final)
        self.assertEqual(early['targets'], [])
        self.assertFalse(any(c[0] == 'cargo' for c in early['commands']))
        self.assertTrue(early['diagnostic_only'])

    def test_only_the_known_operator_skill_is_tooling(self):
        final = plan.select(['.agents/skills/hiroute-integrate/SKILL.md'])
        self.assertFalse(final['rust'])
        self.assertFalse(final['frontend'])
        self.assertIn(['python3', 'scripts/test-test-plan.py'], final['commands'])
        for path in ['.agents/skills/unknown/SKILL.md', 'assets/skills/hiroute-integrate/SKILL.md',
                     '.agents/skills/hiroute-integrate/scripts/run.py']:
            self.assertEqual(plan.select([path])['mode'], 'full')

    def test_client_helper_retains_smoke_but_preflight_does_not_repeat_it(self):
        final = plan.select(['tools/e2e-harness/src/p0/client.rs'])
        self.assertTrue(any('smoke_cli' in c for c in final['commands']))
        early = plan.integration_preflight(final)
        self.assertIn('p0_gateway_matrix_coverage', early['targets'])
        self.assertFalse(any('smoke_cli' in c for c in early['commands']))


class RevisionPlans(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.script = Path(__file__).with_name('test-plan.py').resolve()
        self.git('init', '-q')
        hooks = self.root / '.git' / 'fixture-hooks'
        hooks.mkdir()
        self.git('config', 'core.hooksPath', str(hooks))
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        self.write('README.md', 'base')
        self.base = self.commit('base')
        self.write('Cargo.lock', 'shared contract')
        self.shared = self.commit('shared')

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.root), *args], text=True).strip()

    def write(self, name, value):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value)

    def commit(self, message):
        self.git('add', '.')
        self.git('-c', 'commit.gpgsign=false', 'commit', '-qm', message)
        return self.git('rev-parse', 'HEAD')

    def invoke(self, *args):
        return subprocess.run([sys.executable, str(self.script), *args], cwd=self.root, capture_output=True, text=True)

    def test_final_obligations_survive_iteration_scope(self):
        self.write('tools/e2e-harness/tests/p0_gateway_runtime/reasoning.rs', 'wire order')
        self.commit('focused repair')
        result = self.invoke('--base', self.base, '--since', self.shared)
        self.assertEqual(result.returncode, 0, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(value['mode'], 'full')
        self.assertEqual(value['iteration']['mode'], 'affected')
        self.assertTrue(value['iteration']['diagnostic_only'])
        self.assertEqual(value['iteration']['since'], self.shared)

    def test_rename_staged_unstaged_and_untracked_are_included(self):
        old = 'tools/product-e2e/tests/routing_plans.rs'
        self.write(old, 'old')
        checkpoint = self.commit('test')
        self.git('mv', old, 'tools/product-e2e/tests/unknown.rs')
        self.write('README.md', 'unstaged')
        self.write('new-unknown-file', 'untracked')
        result = json.loads(self.invoke('--base', self.base, '--since', checkpoint).stdout)['iteration']
        self.assertEqual(set(result['paths']), {old, 'tools/product-e2e/tests/unknown.rs', 'README.md', 'new-unknown-file'})
        self.assertEqual(result['mode'], 'full')

    def test_nonancestor_and_invalid_checkpoint_fail(self):
        self.git('checkout', '-qb', 'other', self.base)
        self.write('other.txt', 'other')
        unrelated = self.commit('other')
        self.git('checkout', '--detach', self.shared)
        for checkpoint in (unrelated, 'missing-ref', '--help'):
            result = self.invoke('--base', self.base, '--since', checkpoint)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, '')

    def test_integration_collection_uses_iteration_and_preserves_base_plan(self):
        self.write('tools/e2e-harness/tests/p0_gateway_runtime.rs', 'changed semantics')
        value = json.loads(self.invoke('--base', self.base, '--since', self.shared,
                                       '--integration', '--collect-failures').stdout)
        self.assertEqual(value['mode'], 'full')
        self.assertEqual(value['integration']['scope'], 'iteration')
        self.assertEqual(value['iteration']['mode'], 'affected')
        self.assertIn('p0_gateway_protocol', value['integration']['targets'])
        self.assertTrue(value['integration']['collect_failures'])
        self.assertTrue(value['integration']['remote_exclusive'])
        for command in value['integration']['commands']:
            if command[:2] == ['cargo', 'test']:
                self.assertIn('--workspace', command)
                self.assertIn('--all-features', command)
        self.assertTrue(all('--no-fail-fast' not in c for c in value['commands']))
        self.assertEqual(value['candidate'], self.shared)
        self.assertEqual(value['base'], self.base)

    def test_collection_and_integration_do_not_change_hosted_execution(self):
        for flags in [('--collect-failures',), ('--integration', '--run-ci'),
                      ('--integration', '--github-output')]:
            result = self.invoke('--base', self.base, *flags)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, '')

    def test_iteration_is_not_a_ci_waiver(self):
        result = self.invoke('--base', self.base, '--since', self.shared, '--run-ci')
        self.assertNotEqual(result.returncode, 0)

    def test_explicit_full_and_empty_iteration_keep_final(self):
        value = json.loads(self.invoke('--full', '--since', self.shared).stdout)
        self.assertEqual(value['mode'], 'full')
        self.assertEqual(value['iteration']['paths'], [])
        self.assertEqual(value['iteration']['commands'], [])


if __name__ == "__main__":
    unittest.main()
