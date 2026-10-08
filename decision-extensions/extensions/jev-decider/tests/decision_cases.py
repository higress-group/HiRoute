"""Current wire examples shared by pure and real HTTP-handler tests."""
from pathlib import Path
import json

EXAMPLES = json.loads((Path(__file__).resolve().parents[3] / 'api/decision-examples.json').read_text())['cases']

def request(index=0):
    return json.loads(json.dumps(EXAMPLES[index]['request']))

def answers(category=False, score=None):
    values = {'q0': {'type': 'score', 'score': 0.1, 'probabilities': {'0': 0.8, '1': 0.2}}}
    if category:
        values = {'q0': {'type': 'choice', 'choice': 'review'}, 'q1': {'invalid': True},
                  'q2': {'type': 'score', 'probabilities': {'0': 0.8, '1': 0.2}}}
    if score is not None:
        values['q3' if category else 'q1'] = {'type': 'score', 'score': score, 'confidence': 0.99}
    return {'answers': values}
