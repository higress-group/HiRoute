"""Independent staged assertions; run generated subjects in an OS sandbox."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import unittest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', type=Path, required=True)
    parser.add_argument('--stage', type=int, choices=range(1, 7), required=True)
    args = parser.parse_args()
    project = args.project.resolve(strict=True)
    os.environ['AUDIT_PROJECT'] = str(project)
    os.environ['AUDIT_STAGE'] = str(args.stage)
    path = Path(__file__).parent / 'grading' / 'test_audit.py'
    spec = importlib.util.spec_from_file_location('audit_grader', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    suite = unittest.defaultTestLoader.loadTestsFromModule(module)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    selected = result.testsRun - len(result.skipped)
    failed_cases = sorted({getattr(test, 'test_case', test).id()
                           for test, _ in [*result.failures, *result.errors]})
    print(json.dumps({'stage': args.stage, 'selected': selected,
                      'failed_cases': failed_cases,
                      'assertion_failures': len(result.failures),
                      'assertion_errors': len(result.errors),
                      'passed': selected - len(failed_cases)}))
    return 0 if result.wasSuccessful() and selected > 0 else 1


if __name__ == '__main__':
    sys.exit(main())
