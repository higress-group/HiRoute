"""Check saved writing artifacts, not the quality of their prose."""
import argparse
import json
from pathlib import Path
import re
import sys


def inspect(project, stage):
    result = {'stage': stage, 'artifacts': {}, 'errors': []}
    required = ['draft.md', 'review.json', 'final.md', 'final-review.json'][:stage]
    for name in required:
        path = project / name
        if path.is_symlink() or not path.is_file():
            result['errors'].append(f'{name}: missing or not a regular file')
            continue
        if path.stat().st_size > 1_000_000:
            result['errors'].append(f'{name}: exceeds bounded artifact size')
            continue
        try:
            content = path.read_text(encoding='utf-8')
        except UnicodeError:
            result['errors'].append(f'{name}: not UTF-8')
            continue
        if name.endswith('.md'):
            # This is a reproducible estimate; parent checks unusual formatting.
            body = re.split(r'(?im)^#{1,6}\s+(?:sources|references)\s*$', content)[0]
            words = len(re.findall(r"\b[\w]+(?:['’\-][\w]+)*\b", body))
            sources = sorted(set(re.findall(r'\[S[1-7]\]', content)))
            result['artifacts'][name] = {'estimated_body_words': words,
                                         'source_markers': sources}
            if not 1080 <= words <= 1760:
                result['errors'].append(f'{name}: length delivery defect ({words} words)')
            if not sources:
                result['errors'].append(f'{name}: missing source markers')
        else:
            try:
                review = json.loads(content)
                if not isinstance(review, dict):
                    raise ValueError('object required')
                if review.get('verdict') not in ('publish', 'revise', 'reject'):
                    raise ValueError('invalid verdict')
                if not isinstance(review.get('summary'), str) or not review['summary'].strip():
                    raise ValueError('nonempty summary required')
                if not isinstance(review.get('findings'), list):
                    raise ValueError('findings list required')
                for finding in review['findings']:
                    if not isinstance(finding, dict):
                        raise ValueError('finding must be an object')
                    if finding.get('severity') not in ('critical', 'major', 'minor'):
                        raise ValueError('invalid severity')
                    for key in ('location', 'problem', 'evidence', 'suggested_change'):
                        if not isinstance(finding.get(key), str) or not finding[key].strip():
                            raise ValueError(f'nonempty {key} required')
                result['artifacts'][name] = {'findings': len(review['findings'])}
            except (ValueError, TypeError) as exc:
                result['errors'].append(f'{name}: {exc}')
    result['structural_green'] = not result['errors']
    result['editorial_quality'] = 'requires independent parent assessment'
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', type=Path, required=True)
    parser.add_argument('--stage', type=int, choices=range(1, 5), default=4)
    args = parser.parse_args()
    result = inspect(args.project.resolve(strict=True), args.stage)
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return 0 if result['structural_green'] else 1


if __name__ == '__main__':
    sys.exit(main())
