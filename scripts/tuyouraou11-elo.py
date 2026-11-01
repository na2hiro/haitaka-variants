#!/usr/bin/env python3
"""Convert saved match scores and paired bootstrap intervals to logistic Elo gaps."""
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
META = ROOT / 'plans/tuyouraou11'


def elo(score):
    if score == 0:
        return '-infinity'
    if score == 1:
        return '+infinity'
    if not 0 < score < 1:
        raise ValueError('score rate must be between 0 and 1')
    return 400 * math.log10(score / (1 - score))


def convert(row):
    result = {'scoreRate': row['scoreRate'], 'eloDifference': elo(row['scoreRate']),
              'ci95': [elo(value) for value in row['ci95']]}
    if 'openingBlockCi95' in row:
        result['openingBlockCi95'] = [elo(value) for value in row['openingBlockCi95']]
    return result


def main():
    strength_path = META / 'p6-strength-results/strength-summary.json'
    native_path = META / 'p4-games-summary.json'
    strength = json.loads(strength_path.read_text())
    native = json.loads(native_path.read_text())['summary']
    assert strength['games'] == 400 and strength['result'] == 'PASS'
    records = []
    for row in strength['strata']:
        records.append({'comparison': 'mini minus micro', 'runtime': 'browser SIMD Wasm',
                        'browser': row['browser'], 'movetimeMs': row['movetimeMs'], 'games': row['games'],
                        'wins': row['miniWins'], 'losses': row['microWins'], 'draws': row['draws'], **convert(row)})
    records.append({'comparison': 'mini minus micro', 'runtime': 'browser SIMD Wasm',
                    'browser': 'equal-weight aggregate of Chromium and Firefox', 'movetimeMs': [200, 1000],
                    'games': 400, 'wins': 248, 'losses': 147, 'draws': 5, **convert(strength)})
    for ms, row in native['strata'].items():
        records.append({'comparison': 'micro minus handcrafted', 'runtime': 'P4 native, before P6 SIMD',
                        'movetimeMs': int(ms), 'games': row['wins'] + row['losses'] + row['draws'],
                        'wins': row['wins'], 'losses': row['losses'], 'draws': row['draws'], **convert(row)})
    records.append({'comparison': 'micro minus handcrafted', 'runtime': 'P4 native, before P6 SIMD',
                    'movetimeMs': [200, 1000], 'games': 200, 'wins': 188, 'losses': 12, 'draws': 0, **convert(native)})
    result = {'schemaVersion': 1, 'formula': '400 * log10(p / (1 - p))',
              'p': '(wins + 0.5 * draws) / games',
              'intervalMethod': 'Monotone transformation of the saved opening-pair bootstrap score interval; not an independent Elo standard-error estimate',
              'interpretation': 'Relative logistic Elo equivalents in the measured match conditions, not absolute human/platform ratings',
              'limitations': 'Native P4 and browser P6 conditions differ; do not add these gaps or infer mini versus handcrafted from them. Direct browser handcrafted matches have not been measured here.',
              'sources': {str(path.relative_to(META)): hashlib.sha256(path.read_bytes()).hexdigest() for path in [strength_path, native_path]},
              'records': records}
    target = META / 'p6-elo-summary.json'
    target.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
