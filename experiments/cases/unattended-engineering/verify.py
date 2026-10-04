#!/usr/bin/env python3
"""Independently check the prepared subject in a disposable copy; no repairs or model calls."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET
ROOT = Path(__file__).resolve().parent
BASE = ['tests/models/test_responses.py','tests/test_decoders.py','tests/test_content.py','tests/test_exceptions.py','tests/test_utils.py']
EXCLUSIONS = 'not test_logging_request and not test_logging_redirect_chain and not test_httpcore_exception_mapping'


def checker_environment(work, python, temp):
    home = Path(temp)/"home"
    home.mkdir()
    return {"HOME":str(home), "TMPDIR":str(temp), "PATH":str(python.parent)+":/usr/bin:/bin",
            "LANG":"C.UTF-8", "PYTHONPATH":str(work), "PYTEST_DISABLE_PLUGIN_AUTOLOAD":"1",
            "PYTHONDONTWRITEBYTECODE":"1", "CI":"1", "OPENSSL_CONF":"/dev/null"}


def verify(subject, python, output):
    output.mkdir(parents=True, exist_ok=False)
    protected = json.loads((subject / '.experiment-protected.json').read_text())
    changed = [n for n, sha in protected.items() if not (subject/n).is_file() or (subject/n).is_symlink() or hashlib.sha256((subject/n).read_bytes()).hexdigest()!=sha]
    if changed:
        raise ValueError('protected files changed: '+', '.join(changed))
    with tempfile.TemporaryDirectory(prefix='hiroute-independent-grade-') as temp:
        work = Path(temp)/'subject'
        shutil.copytree(subject, work, ignore=shutil.ignore_patterns('.git','.venv','venv','__pycache__','.pytest_cache'), symlinks=True)
        shutil.copyfile(ROOT/'fixtures/test_incremental_probe.py',work/'test_incremental_probe.py')
        # Verify the official feature test bytes, not a subject-supplied substitute.
        reference = Path(temp)/'reference'
        subprocess.run(['git','clone','--quiet','--no-hardlinks',str(subject),str(reference)],check=True)
        spec=json.loads((ROOT/'case.json').read_text())
        subprocess.run(['git','checkout','--quiet','--detach',spec['baseline']],cwd=reference,check=True)
        subprocess.run(['git','apply',str(ROOT/'fixtures/test.patch')],cwd=reference,check=True)
        expected={str(p.relative_to(reference)):hashlib.sha256(p.read_bytes()).hexdigest() for p in reference.rglob('*') if p.is_file() and (p.is_relative_to(reference/'tests') or p.name in ('pyproject.toml','requirements.txt','test.sh'))}
        if protected != expected:
            raise ValueError('protected manifest does not match independently reconstructed baseline')
        env=checker_environment(work, python, temp)
        bootstrap='import pathlib,httpx,pytest,sys; assert pathlib.Path(httpx.__file__).resolve().is_relative_to(pathlib.Path.cwd()); raise SystemExit(pytest.main(sys.argv[1:]))'
        results=[]
        for label,paths,count in [('feature',['tests/test_json_stream.py'],108),('existing',BASE,229),('incrementality',['test_incremental_probe.py'],6)]:
            report=output/(label+'.xml')
            cmd=[str(python),'-c',bootstrap,'-p','anyio.pytest_plugin','-p','no:cacheprovider','-q',*paths,'-k',EXCLUSIONS,'--junitxml='+str(report)]
            with (output/(label+'.log')).open('w') as log:
                proc=subprocess.run(cmd,cwd=work,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=600)
            cases=list(ET.parse(report).getroot().iter('testcase')) if report.exists() else []
            passed=sum(not any(c.find(k) is not None for k in ['failure','error','skipped']) for c in cases)
            results.append(dict(gate=label,exit=proc.returncode,expected=count,observed=len(cases),passed=passed,green=proc.returncode==0 and len(cases)==passed==count))
        value=dict(scenario='green' if all(x['green'] for x in results) else 'red',gates=results,protected_changes=[],routing_assessed_separately=True)
        (output/'assessment.json').write_text(json.dumps(value,indent=2)+'\n')
        print(json.dumps(value))
        return all(x['green'] for x in results)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--subject',type=Path,required=True)
    p.add_argument('--python',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    raise SystemExit(0 if verify(a.subject.resolve(),a.python.absolute(),a.output.resolve()) else 1)
