#!/usr/bin/env python3
"""Ensure native verdicts and original revisions survive feedback generation."""
import importlib.util
import ast
import base64
import copy
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest
import sys
from unittest import mock

spec = importlib.util.spec_from_file_location("windows_report", Path(__file__).with_name("windows-ci-report.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def powershell_probe():
    """Parser/source-function checks in memory; no Cargo, PE or Task run."""
    directory = Path(__file__).parent
    inputs = {name: (directory / ('windows-' + name + '.ps1')).read_bytes()
              for name in ('native-scopes', 'headless-ci', 'native-replay')}
    bindings = [' $sources[' + repr(name) + ']=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String(' +
                repr(base64.b64encode(data).decode()) + '))' for name, data in inputs.items()]
    hashes = json.dumps({name:hashlib.sha256(data).hexdigest() for name,data in inputs.items()}, sort_keys=True)
    return ("$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'\n$sources=@{}\n" +
        '\n'.join(bindings) + '\n$hashes=' + repr(hashes) + "|ConvertFrom-Json\n" + r"""
$state=@{count=0}
function Require([bool]$Fact){if(-not $Fact){throw 'assessment-probe-rejected'};$state.count++}
function Reject([scriptblock]$Action){$rejected=$false;try{& $Action}catch{$rejected=$true};Require $rejected}
try {
 $asts=@{}
 foreach($key in $sources.Keys){
  $tokens=$null;$errors=$null
  $asts[$key]=[Management.Automation.Language.Parser]::ParseInput($sources[$key],[ref]$tokens,[ref]$errors)
  Require (@($errors).Count -eq 0)
 }
 . ([scriptblock]::Create($sources['native-scopes']))
 $ordering=@($asts['native-replay'].FindAll({param($node)$node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Get-WindowsReplayRecipes'},$true))
 Require ($ordering.Count -eq 1)
 . ([scriptblock]::Create($ordering[0].Extent.Text))
 $product=@(Get-WindowsRequiredRecipes product)
 $captures=@($product|Sort-Object|ForEach-Object{[pscustomobject]@{name=$_;required_cases=@('fixture_'+$_);harness_arguments=@('--exact')}})
 $before=$captures|ConvertTo-Json -Depth 5 -Compress
 Require (($captures.name -join ',') -cne ($product -join ','))
 $ordered=@(Get-WindowsReplayRecipes $captures $product)
 Require ($ordered.Count -eq $product.Count)
 Require (($ordered.name -join ',') -ceq ($product -join ','))
 $same=$true
 foreach($recipe in $ordered){$original=@($captures|Where-Object{$_.name -ceq $recipe.name});$same=$same -and $original.Count -eq 1 -and [object]::ReferenceEquals($recipe,$original[0])}
 Require $same
 Require (($captures|ConvertTo-Json -Depth 5 -Compress) -ceq $before)
 Require ([Array]::IndexOf($ordered.name,'headless') -lt [Array]::IndexOf($ordered.name,'foreground-ctrl-c') -and [Array]::IndexOf($ordered.name,'headless') -lt [Array]::IndexOf($ordered.name,'foreground-ctrl-break'))
 Reject {Get-WindowsReplayRecipes $captures @()}
 Reject {Get-WindowsReplayRecipes @() $product}
 Reject {Get-WindowsReplayRecipes @($captures|Select-Object -Skip 1) $product}
 Reject {Get-WindowsReplayRecipes (@($captures)+@($captures[0])) $product}
 Reject {Get-WindowsReplayRecipes (@($captures)+@([pscustomobject]@{name='foreign'})) $product}
 Reject {Get-WindowsReplayRecipes $captures @($product|ForEach-Object{$_.ToUpperInvariant()})}
 Reject {Get-WindowsReplayRecipes $captures (@($product)+@($product[0]))}
 Reject {Get-WindowsReplayRecipes $captures (@($product)+@(''))}
 $required=@(Get-WindowsRequiredRecipes background)
 Require ($required.Count -eq 5)
 $runtime=@(Get-WindowsAssessmentNames background hosted execute)
 $deferred=@(Get-WindowsAssessmentNames background hosted compile-only)
 Require ($runtime.Count -eq 3 -and $deferred.Count -eq 2)
 Require (@(Get-WindowsAssessmentNames background full execute).Count -eq 5)
 Require (@(Get-WindowsAssessmentNames background full compile-only).Count -eq 0)
 Reject {Get-WindowsRuntimeDisposition background unknown-new-recipe hosted}
 Reject {Get-WindowsRuntimeDisposition background unknown-new-recipe full}
 Reject {Get-WindowsRuntimeDisposition components jobs hosted}
 $crlf="fixture::one: test`r`nfixture::two: test`r`n`r`n2 tests, 0 benchmarks`r`n"
 $lf=$crlf.Replace("`r",'')
 Assert-WindowsDeferredInventory $crlf @('fixture::one','fixture::two');$state.count++
 Assert-WindowsDeferredInventory $lf @('fixture::one','fixture::two');$state.count++
 # The previous single-quoted backtick-r matcher loses this same CRLF list.
 Require ([regex]::Matches($crlf,'(?m)^([^\r\n]+): test`r?$').Count -eq 0)
 Reject {Assert-WindowsDeferredInventory '0 tests' @('fixture::one')}
 Reject {Assert-WindowsDeferredInventory $crlf @('fixture::one')}
 Reject {Assert-WindowsDeferredInventory $crlf @('fixture::one','fixture::one')}
 Reject {Assert-WindowsDeferredInventory $crlf @('fixture::one','fixture::foreign')}
 Reject {Assert-WindowsDeferredInventory $crlf.ToUpperInvariant() @('fixture::one','fixture::two')}
 foreach($key in @('headless-ci','native-replay')){
  $body=$asts[$key]
  if($key -eq 'headless-ci'){
   $function=@($body.FindAll({param($node)$node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Invoke-RequiredTests'},$true))
   Require ($function.Count -eq 1);$body=$function[0].Body
  }
  $branch=@($body.FindAll({param($node)$node -is [Management.Automation.Language.IfStatementAst] -and $node.Clauses[0].Item1.Extent.Text.Trim().TrimStart('(').TrimEnd(')') -ceq '$disposition -eq ''compile-only'''},$true))
  Require ($branch.Count -eq 1)
  $block=$branch[0].Clauses[0].Item2
  $terminal=@($block.FindAll({param($node)$node -is [Management.Automation.Language.ReturnStatementAst] -or $node -is [Management.Automation.Language.ContinueStatementAst]},$true))
  Require ($terminal.Count -eq 1)
  Require ($block.Extent.Text.Contains('Assert-WindowsDeferredInventory'))
  $runtimeTrue=@($body.FindAll({param($node)$node -is [Management.Automation.Language.AssignmentStatementAst] -and $node.Left.Extent.Text -ceq '$result.runtime_executed' -and $node.Right.Extent.Text -ceq '$true'},$true))
  Require ($runtimeTrue.Count -eq 1 -and $runtimeTrue[0].Extent.StartOffset -gt $block.Extent.EndOffset)
 }
 if($state.count -ne 42){throw 'assessment-probe-counter'}
 @{schema='hiroute.windows-assessment-tooling-probe/v1';scenario='tooling_policy_green';parser_files=$asts.Count;assertions=$state.count;source_files=$hashes;business_executed=$false;task_mutated=$false;product_runtime='not_assessed'}|ConvertTo-Json -Compress -Depth 5
} catch {
 @{schema='hiroute.windows-assessment-tooling-probe/v1';scenario='red';assertions=$state.count;business_executed=$false;task_mutated=$false;product_runtime='not_assessed';hresult=$_.Exception.HResult}|ConvertTo-Json -Compress;exit 1
}
""")


class NativeFeedback(unittest.TestCase):
    def test_core_timing_and_field_exclusions_remain_source_declared_opt_ins(self):
        repo = Path(__file__).resolve().parent.parent
        driver = (repo / 'scripts/windows-headless-ci.ps1').read_text()
        for package in ('application', 'observation', 'gateway'):
            with self.subTest(package=package):
                marker = "if ($Package -eq 'hiroute-" + package + "') {"
                block = driver.split(marker, 1)[1].split('\n    }', 1)[0]
                names = re.findall(r"'([^']+::[^']+)'", block)
                self.assertTrue(names)
                self.assertEqual(len(names), len(set(names)))
                sources = '\n'.join(path.read_text() for path in
                                    (repo / 'crates' / package / 'src').rglob('*.rs'))
                for name in names:
                    function = re.escape(name.rsplit('::', 1)[1])
                    explicit = re.findall(
                        r'#\[test\]\s*#\[ignore\s*=\s*"([^"\n]+)"\]\s*fn\s+'
                        + function + r'\s*\(', sources)
                    self.assertEqual(len(explicit), 1, name)

    def fixture(self, root, scenario="green", revision="a" * 40):
        provenance = dict(source_revision="a" * 40, host_architecture="X64", cargo="cargo fixed",
                          rustc="rustc fixed", plan="windows-headless", phase="focused", related_runs=["123"])
        (root / "provenance.json").write_text(json.dumps(provenance), encoding="utf-8-sig")
        native = dict(name="jobs", source_revision=revision, cases=["job_case"], scenario=scenario,
                      process_exit=0 if scenario == "green" else 101, plan="windows-headless", phase="focused",
                      related_runs=["123"], command=["cargo", "test", "--test", "job_windows"])
        (root / "jobs-result.json").write_text(json.dumps(native))
        state = "ok" if scenario == "green" else "FAILED"
        (root / "jobs.log").write_text("     Running tests\\job_windows.rs (target\\debug\\deps\\job.exe)\n"
                                       "running 1 test\n"
                                       f"test job_case ... {state}\n"
                                       f"test result: {state}. {int(scenario == 'green')} passed; "
                                       f"{int(scenario != 'green')} failed; 0 ignored; finished in 0.25s\n")

    def test_green_preserves_native_sha_case_and_external_ci_link(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            report = module.collect(root)
            native = json.loads((root / "feedback/jobs/validation-report.json").read_text())
            self.assertEqual(report["related_ci_runs"], ["123"])
            self.assertEqual(native["sha"], "a" * 40)
            self.assertEqual(native["scenario"], "green")
            self.assertTrue(native["observed_tests_passed"])
            self.assertEqual(native["top_modules"][0]["seconds"], .25)

    def test_red_does_not_become_green_after_derivation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root, "red")
            module.collect(root)
            record = json.loads((root / "feedback/jobs/validation-report.json").read_text())
            self.assertEqual(record["process_exit"], 101)
            self.assertEqual(record["scenario"], "red")
            self.assertFalse(record["observed_tests_passed"])

    def test_foreign_candidate_case_record_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root, revision="b" * 40)
            with self.assertRaisesRegex(ValueError, "another candidate"):
                module.collect(root)

    def test_failed_package_prerequisite_does_not_hide_case_reports_or_become_green(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root, "red")
            (root / "package-result.json").write_bytes(b"")
            report = module.collect(root)
            self.assertEqual(report["reports"][0]["scenario"], "red")
            self.assertEqual(report["prerequisites"], [{"name": "package-result", "result_present": False}])


class AssessmentFeedback(unittest.TestCase):
    """Synthetic closed receipts exercise reporting, not Windows/PE execution."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.runtime = ['background-capture', 'packaged-proxy-oracle', 'packaged-foreground']
        self.deferred = ['background', 'background-standard-owner', 'background-standard-upgrade']
        self.provenance = dict(source_revision='a'*40, assessment='hosted', scope='background',
            runtime_recipes=self.runtime, deferred_recipes=self.deferred, full_scope=False, candidate_complete=False,
            host_architecture='X64', cargo='cargo fixture', rustc='rustc fixture', plan='windows-headless',
            phase='focused', related_runs=[])
        self.records = []
        for name, count in zip(self.runtime + self.deferred, [2,5,1,1,1,1]):
            cases = [name.replace('-', '_') + '_case_' + str(index) for index in range(count)]
            deferred = name in self.deferred
            row = dict(name=name, source_revision='a'*40, assessment='hosted', scenario='not_assessed' if deferred else 'green',
                cases=cases, executed=0 if deferred else count, ignored=0, process_exit=None if deferred else 0,
                runtime_executed=not deferred, build_status='compiled', inventory_exit=0, inventory_verified=True,
                artifact_capture_verified=True, command=['fixture-list' if deferred else 'fixture-test'],
                plan='windows-headless', phase='focused')
            if deferred:row['deferred_reason'] = 'requires_real_logged_owner'
            else:
                row['tests'] = dict(passed=count, failed=0, ignored=0)
                (self.root / (name + '.log')).write_text('Running tests\\fixture.rs (fixture.exe)\n'
                    + f'test result: ok. {count} passed; 0 failed; 0 ignored; finished in 0.1s\n')
            self.records.append(row)
        self.manifest = dict(schema='hiroute.windows-native-artifacts/v1',
                             recipes=[dict(name=row['name'], required_cases=row['cases']) for row in self.records],
                             files=[{'fixture_only': index} for index in range(11)], package={'fixture_only': True},
                             product_binaries={'hiroute':'fixture-cli', 'hirouted':'fixture-daemon'})
        (self.root / 'native-artifacts.json').write_text(json.dumps(self.manifest))
        self.outcome = dict(schema='hiroute.windows-native-scope-outcome/v1', source_revision='a'*40,
            scope='background', assessment='hosted', invocation_failed=False, failure_count=0,
            capture_requested=True, artifact_verification_passed=True, runtime_import_audit_passed=True,
            native_manifest_sha256=hashlib.sha256((self.root/'native-artifacts.json').read_bytes()).hexdigest())
        # Evaluate the exact canonical CLI's success receipt, rather than adding
        # a field that the real producer never emits.
        tree = ast.parse((Path(__file__).parent / 'windows-native-artifacts.py').read_bytes())
        receipts = [node.args[0].args[0] for node in ast.walk(tree)
                    if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == 'print'
                    and node.args and isinstance(node.args[0], ast.Call)
                    and isinstance(node.args[0].func, ast.Attribute) and node.args[0].func.attr == 'dumps'
                    and node.args[0].args and isinstance(node.args[0].args[0], ast.Dict)]
        self.assertEqual(len(receipts), 1)
        value = dict(self.manifest, build={'source_sha': 'a'*40})
        receipt = eval(compile(ast.Expression(receipts[0]), '<canonical-native-receipt>', 'eval'),
                       {'__builtins__': {}, 'len': len}, {'value': value})
        self.assertEqual(set(receipt), {'schema', 'source_sha', 'recipes'})
        (self.root / 'artifact-verification.json').write_text(json.dumps(receipt))
        (self.root / 'runtime-audit.json').write_text(json.dumps(dict(source_sha='a'*40, dependency_gate='green',
            dll_resolution_and_runtime='not_assessed', production_release_acceptance='not_assessed')))
        self.patch = mock.patch.object(module, 'verify_artifacts', return_value=self.manifest)
        self.verifier = self.patch.start();self.addCleanup(self.patch.stop)
        self.write()

    def write(self):
        (self.root / 'provenance.json').write_text(json.dumps(self.provenance))
        (self.root / 'scope-outcome.json').write_text(json.dumps(self.outcome))
        for row in self.records:(self.root / (row['name'] + '-result.json')).write_text(json.dumps(row))

    def collect(self):
        self.write();return module.collect(self.root)

    def test_hosted_compile_and_eight_runtime_cases_never_claim_full_background(self):
        value = self.collect()
        self.assertEqual(value['scenario'], 'not_assessed');self.assertTrue(value['assessment_checks_passed'])
        self.assertFalse(value['candidate_complete']);self.assertFalse(value['full_scope'])
        self.assertEqual(value['background_acceptance'], 'not_assessed')
        self.assertEqual(value['runtime_cases_executed'], 8);self.assertEqual(value['deferred_cases'], 3)
        record = json.loads((self.root/'feedback/background/result.json').read_text())
        self.assertEqual(record['status'], 'not_assessed');self.assertIsNone(record['process_exit'])
        derived = json.loads((self.root/'feedback/background/validation-report.json').read_text())
        self.assertFalse(derived['observed_tests_passed']);self.assertEqual(derived['scenario'], 'not_assessed')
        self.verifier.assert_called_once_with(self.root, 'a'*40)

    def test_deferred_cannot_hide_body_exit_ignored_missing_capture_or_compile(self):
        original = copy.deepcopy(self.records[-1])
        changes = [('scenario','green'), ('runtime_executed',True), ('executed',1), ('executed',False),
            ('process_exit',0), ('ignored',1), ('ignored',False), ('inventory_exit',None), ('inventory_exit',False),
            ('inventory_verified',False), ('artifact_capture_verified',False), ('build_status','not_assessed'),
            ('tests',{'passed':1}), ('command_started_at',1), ('deferred_reason','prerequisite_failed')]
        for key, value in changes:
            with self.subTest(key=key, value=value):
                self.records[-1] = dict(original, **{key:value})
                with self.assertRaises(ValueError):self.collect()
        self.records[-1] = original;self.collect()

    def test_selected_full_task_prerequisite_red_remains_red(self):
        self.provenance.update(assessment='full', runtime_recipes=self.runtime+self.deferred, deferred_recipes=[], full_scope=True)
        self.outcome['assessment'] = 'full'
        for row in self.records:
            row['assessment'] = 'full'
            if row['name'] in self.deferred:
                row.update(scenario='red', process_exit=101, runtime_executed=True)
        value = self.collect()
        self.assertEqual(value['scenario'], 'red');self.assertFalse(value['assessment_checks_passed'])
        self.assertEqual(value['background_acceptance'], 'red');self.assertFalse(value['candidate_complete'])
        self.verifier.assert_not_called()

    def test_full_requires_real_execution_of_every_original_recipe(self):
        self.provenance.update(assessment='full', runtime_recipes=self.runtime+self.deferred, deferred_recipes=[], full_scope=True)
        self.outcome['assessment'] = 'full'
        for row in self.records:
            row.update(assessment='full', scenario='green', runtime_executed=True, process_exit=0, executed=len(row['cases']))
            row.pop('deferred_reason',None)
        value = self.collect()
        self.assertEqual(value['scenario'],'green');self.assertTrue(value['full_scope'])
        self.assertEqual(value['background_acceptance'],'green');self.assertFalse(value['candidate_complete'])
        self.assertEqual(value['runtime_cases_executed'],11);self.assertEqual(value['deferred_cases'],0)
        self.records[-1].update(scenario='not_assessed',runtime_executed=False,process_exit=None,executed=0)
        with self.assertRaises(ValueError):self.collect()

    def test_runtime_zero_missing_or_unknown_recipe_and_body_failure_stay_red(self):
        original = copy.deepcopy(self.records)
        self.records[0].update(executed=0)
        with self.assertRaises(ValueError):self.collect()
        self.records = copy.deepcopy(original);self.records[0].update(scenario='red', process_exit=101)
        value = self.collect();self.assertEqual(value['scenario'], 'red')
        self.records = copy.deepcopy(original)
        for row in [None, dict(original[0], name='unknown-new-recipe')]:
            if row is None:
                missing = self.records.pop();(self.root/(missing['name']+'-result.json')).unlink()
            else:self.records.append(row)
            with self.assertRaises(ValueError):self.collect()
            for path in self.root.glob('*-result.json'):path.unlink()
            self.records = copy.deepcopy(original);self.write()

    def test_capture_missing_foreign_manifest_or_pe_verifier_failure_cannot_pass(self):
        for key in ('capture_requested','artifact_verification_passed','runtime_import_audit_passed'):
            self.outcome[key] = False
            with self.assertRaises(ValueError):self.collect()
            self.outcome[key] = True
        self.manifest['recipes'].pop()
        with self.assertRaises(ValueError):self.collect()
        self.manifest['recipes'].append(dict(name=self.records[-1]['name'], required_cases=self.records[-1]['cases']))
        self.verifier.side_effect = ValueError('actual source PE byte verification failed')
        with self.assertRaisesRegex(ValueError, 'PE byte'):self.collect()

    def test_package_or_product_missing_and_runtime_or_release_promotion_are_rejected(self):
        for key, value in [('package',None), ('product_binaries',{'hiroute':'fixture-cli'})]:
            original=self.manifest[key];self.manifest[key]=value
            with self.assertRaises(ValueError):self.collect()
            self.manifest[key]=original
        path=self.root/'runtime-audit.json';original=path.read_bytes()
        for key in ('dll_resolution_and_runtime','production_release_acceptance'):
            data=json.loads(original);data[key]='green';path.write_text(json.dumps(data))
            with self.assertRaises(ValueError):self.collect()
        path.write_bytes(original);self.collect()

    def test_outer_exception_or_final_import_failure_is_not_hidden_by_good_case_rows(self):
        self.outcome.update(invocation_failed=True)
        self.assertEqual(self.collect()['scenario'], 'red')
        self.outcome.update(invocation_failed=False, failure_count=1)
        self.assertEqual(self.collect()['scenario'], 'red')

    def test_manifest_hash_or_original_required_case_difference_is_rejected(self):
        self.outcome['native_manifest_sha256'] = 'b'*64
        with self.assertRaises(ValueError):self.collect()
        self.outcome['native_manifest_sha256'] = hashlib.sha256((self.root/'native-artifacts.json').read_bytes()).hexdigest()
        self.manifest['recipes'][0]['required_cases'] = ['foreign_case']
        with self.assertRaises(ValueError):self.collect()

    def test_scope_or_full_candidate_promotion_and_duplicate_json_are_rejected(self):
        for key, value in [('scope','agents'), ('candidate_complete',True), ('full_scope',True), ('assessment','unknown')]:
            original = self.provenance[key];self.provenance[key] = value
            with self.assertRaises(ValueError):self.collect()
            self.provenance[key] = original
        self.write();path = self.root/'provenance.json';path.write_text('{"assessment":"full","assessment":"hosted"}')
        with self.assertRaisesRegex(ValueError,'Duplicate'):module.collect(self.root)

    def test_source_function_probe_binds_three_actual_inputs_and_never_runs_business(self):
        body = powershell_probe()
        for name in ('native-scopes','headless-ci','native-replay'):
            data = (Path(__file__).parent/('windows-'+name+'.ps1')).read_bytes()
            self.assertIn(base64.b64encode(data).decode(), body)
            self.assertIn(hashlib.sha256(data).hexdigest(), body)
        self.assertIn('Parser]::ParseInput',body);self.assertIn('Assert-WindowsDeferredInventory $crlf',body)
        self.assertIn('ContinueStatementAst',body);self.assertIn('ReturnStatementAst',body)
        self.assertIn('business_executed=$false',body)
        self.assertNotIn('Start-Process',body);self.assertNotIn('New-ScheduledTask',body)
        self.assertNotIn('$script:count',body);self.assertIn('$state.count -ne 42',body)
        self.assertIn('Get-WindowsReplayRecipes $captures $product',body)
        self.assertIn('[object]::ReferenceEquals',body)
        self.assertIn("[Array]::IndexOf($ordered.name,'headless')",body)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == '--write-powershell-probe':
        with Path(sys.argv[2]).open('x',encoding='utf-8') as output:output.write(powershell_probe())
    else:
        unittest.main()
