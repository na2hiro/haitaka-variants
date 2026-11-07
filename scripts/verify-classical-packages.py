#!/usr/bin/env python3
"""Check the four stage-4 CLI/WASM archives and capability guards."""
import argparse
import json
from pathlib import Path
import subprocess
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('directory', type=Path, nargs='?', default=Path('target'))
args = parser.parse_args()
report = []
for name, rule_id in [('torishogi', 16), ('washogi', 50), ('heian-dai', 15), ('daishogi', 49)]:
    cli = str((args.directory / ('haitaka-' + name)).resolve())
    archive = args.directory / ('haitaka-' + name + '.tgz')
    subprocess.run([cli, 'package', '--wasm-dir', str(args.directory / ('wasm-' + name)),
                    '--output', str(archive)], check=True, capture_output=True)
    with tarfile.open(archive) as bundle:
        manifest = json.load(bundle.extractfile(next(n for n in bundle.getnames()
                                                     if n.endswith('shogitter-engine.json'))))
    assert manifest['capabilities']['protocols'] == ['haitaka-json-v3']
    assert manifest['profiles'][0]['nnue'] is None
    rule = manifest['profiles'][0]['rules'][0]
    assert rule['ruleId'] == rule_id and rule['variant'] == name
    assert rule['positionFormat'] == rule['moveFormat'] == 'haitaka-json-v3'
    for command in [['package', '--rule-id', '0', '--allow-missing-wasm'],
                    ['package', '--nnue', '/nonexistent', '--allow-missing-wasm'], ['usi']]:
        assert subprocess.run([cli, *command], capture_output=True).returncode != 0
    report.append(dict(ruleset=name, ruleId=rule_id, protocol='haitaka-json-v3',
                       packageBytes=archive.stat().st_size, rejectsWrongRuleId=True,
                       rejectsNNUE=True, rejectsUSI=True))
(args.directory / 'classical-package-verification.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report))
