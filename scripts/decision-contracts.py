#!/usr/bin/env python3
"""Project current v1 schemas and Desktop examples from the maintained decision examples."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
API = ROOT / 'decision-extensions/api'

def ref(name): return {'$ref': '#/components/schemas/' + name}
def obj(properties, required=None):
    return {'type': 'object', 'additionalProperties': False, 'required': list(properties) if required is None else required, 'properties': properties}
def array(items, minimum=0, maximum=None):
    return {'type': 'array', 'minItems': minimum, **({'maxItems': maximum} if maximum else {}), 'items': items}

def main():
    examples = json.loads((API / 'decision-examples.json').read_text())['cases']
    text = {'type': 'string', 'minLength': 1}
    identifier = {'type': 'string', 'minLength': 1, 'maxLength': 128}
    probability = {'type': 'number', 'minimum': 0, 'maximum': 1}
    schemas = {
        'OrdinalDefinition': obj({'kind': {'const': 'ordinal'}, 'instructions': text, 'levels': array(obj({'id': identifier, 'criterion': text}), 2, 16)}),
        'CategoricalDefinition': obj({'kind': {'const': 'categorical'}, 'instructions': text, 'options': array(obj({'id': identifier, 'criterion': text, 'refinement': ref('OrdinalDefinition')}, ['id', 'criterion']), 2, 16)}),
        'DecisionDefinition': {'oneOf': [ref('OrdinalDefinition'), ref('CategoricalDefinition')]},
        'OrdinalResult': obj({'kind': {'const': 'ordinal'}, 'probabilities': {'type': 'object', 'minProperties': 2, 'additionalProperties': probability, 'description': 'Exact requested level IDs, finite probabilities, sum within 1e-6 of 1. Duplicate keys are forbidden.'}}),
        'CategoricalResult': obj({'kind': {'const': 'categorical'}, 'choice': identifier, 'refinement': ref('OrdinalResult')}, ['kind', 'choice']),
        'ContentPart': {'oneOf': [obj({'kind': {'const': 'text'}, 'text': {'type': 'string'}}), obj({'kind': {'const': 'unavailable'}, 'source_kind': identifier})]},
        'StepPart': {'oneOf': [ref('ContentPart'), obj({'kind': {'const': 'tool_activity'}, 'tool': identifier, 'status': {'enum': ['completed', 'failed', 'unknown']}})]},
        'VisibleTurn': obj({'user': array(ref('ContentPart')), 'status': {'enum': ['completed', 'failed', 'interrupted', 'unknown']}, 'steps': array(array(ref('StepPart')))}),
        'AssessmentTarget': obj({'from': {'type': 'integer', 'minimum': 0, 'description': 'Zero-based index in visible_conversation; the suffix is one actual prior stage.'}, 'instructions': text, 'criteria': {'type': 'array', 'minItems': 3, 'maxItems': 3, 'prefixItems': [obj({'score': {'const': score}, 'criterion': text}) for score in [0, .5, 1]], 'items': False}}),
        'Assessment': obj({'score': probability, 'partial': {'type': 'boolean'}, 'reason': {'type': 'string', 'minLength': 1, 'maxLength': 1024}}, ['score', 'partial']),
        'ClassifierRequest': obj({'decision': ref('DecisionDefinition'), 'latest_user': array(ref('ContentPart'), 1), 'visible_conversation': array(ref('VisibleTurn')), 'history_partial': {'type': 'boolean'}, 'assessment_target': {'oneOf': [{'type': 'null'}, ref('AssessmentTarget')]}}),
        'ClassifierResponse': obj({'decision': {'oneOf': [ref('OrdinalResult'), ref('CategoricalResult')]}, 'assessment': ref('Assessment')}, ['decision']),
    }
    content = lambda direction: {'schema': ref('ClassifierRequest' if direction == 'request' else 'ClassifierResponse'), 'examples': {case['name']: {'value': case[direction]} for case in examples if case['scope'] != 'future-docs-only'}}
    document = {'openapi': '3.1.0', 'info': {'title': 'HiRoute Decision API', 'version': '1.0.0', 'description': 'Current v1: categorical task choices, ordinal degree distributions and optional assessment of a preceding actual stage. Tool subset selection is future design only.'}, 'servers': [{'url': 'https://classifier.example', 'description': 'Example only; HiRoute posts to the configured complete endpoint.'}], 'paths': {'/v1/decisions': {'post': {'operationId': 'decideAgentTurn', 'summary': 'Decide the current task and optionally assess a preceding stage', 'description': 'Every new user turn decides again. Tool continuations inherit only while same-turn history is continuous and the frozen decision remains reusable. Exact allowed IDs and selected refinement are required. No model IDs or routing thresholds cross this boundary. Invalid selected degree preserves the category and uses its primary group; invalid optional assessment is discarded independently. Return strict JSON, no duplicate or unknown fields, at most 64 KiB.', 'requestBody': {'required': True, 'content': {'application/json': content('request')}}, 'responses': {'200': {'description': 'Decision with optional prior-stage assessment.', 'content': {'application/json': content('response')}}}}}}, 'components': {'schemas': schemas}}
    (API / 'decision.openapi.json').write_text(json.dumps(document, ensure_ascii=False, indent=2) + '\n')
    values = {name: {key: case[key] for key in ['request', 'response']} for name, case in [('first', next(case for case in examples if case['name'] == 'smart_saving')), ('assessment', next(case for case in examples if case['name'] == 'writing_review'))]}
    source = '// Generated by scripts/decision-contracts.py from the canonical v1 examples.\nexport const protocolExamples = ' + json.dumps(values, ensure_ascii=False, indent=2) + ';\n'
    source += '''export function protocolCurl(example: keyof typeof protocolExamples): string {
  return `curl --request POST 'https://classifier.example/v1/decisions' \\\\\\n  --header 'Content-Type: application/json' \\\\\\n  --data-raw '${JSON.stringify(protocolExamples[example].request, null, 2).replaceAll("'", "'\\\\''")}'`;
}
'''
    (ROOT / 'apps/desktop/src/features/decision-services/protocol-examples.ts').write_text(source)

if __name__ == '__main__': main()
