"""Native answer shape conversion, unchanged from the frozen research adapter."""
import math

def normalize(response, request):
    """Convert native arrays by name/value; never replace score by a modal grade."""
    expected = {q['name']: q for q in request['questions']}
    if not isinstance(response.get('answers'), list):
        raise ValueError('answers_not_array')
    answers = {}
    for a in response['answers']:
        name = a.get('name')
        if name not in expected or name in answers:
            raise ValueError('unknown_or_duplicate_answer_name')
        if a.get('type') == 'refusal':
            answers[name] = {'type': 'refusal'}
            continue
        q = expected[name]
        if a.get('type') != q['type'] or not isinstance(a.get('probabilities'), list):
            raise ValueError('wrong_answer_type')
        allowed = ([x['value'] for x in q['choices']] if q['type'] == 'choice'
                   else list(range(len(q['levels']))))
        probabilities = {}
        for p in a['probabilities']:
            value = p.get('value')
            if type(value) not in (str, int) or isinstance(value, bool) or value not in allowed:
                raise ValueError('invalid_probability_value')
            key = str(value)
            if key in probabilities:
                raise ValueError('duplicate_probability_value')
            v = p.get('probability')
            if type(v) not in (int, float) or not math.isfinite(v) or not 0 <= v <= 1:
                raise ValueError('invalid_probability')
            probabilities[key] = v
        if set(probabilities) != {str(v) for v in allowed}:
            raise ValueError('incomplete_probability_values')
        # Restore declared option order for the frozen tie convention.
        out = {'type': a['type'], 'probabilities': {str(v): probabilities[str(v)] for v in allowed}}
        if q['type'] == 'score':
            out['score'] = a['score']
        else:
            out['choice'] = a['choice']
        answers[name] = out
    return {'answers': answers}
