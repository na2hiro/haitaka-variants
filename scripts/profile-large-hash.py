#!/usr/bin/env python3
"""Count/time full FNV hashes in an isolated source snapshot, never in production."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tomllib
ROOT = Path(__file__).resolve().parents[1]
RULES = ['tenjiku-dai', 'dai-dai', 'maka-dai-dai', 'taishogi', 'taikyoku']
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--revision', help='Git revision; omit for working tree engine sources')
parser.add_argument('--label', required=True)
parser.add_argument('--cpu', type=int, default=7)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if not re.fullmatch('[a-z][a-z0-9-]*', args.label):
    parser.error('label must be lowercase letters/digits/hyphens')
dest = ROOT / 'target' / ('hash-profile-' + args.label)
if dest.exists():
    parser.error('snapshot already exists; use a new label')
dest.mkdir(parents=True)
if args.revision:
    archive = subprocess.check_output(['git', 'archive', args.revision, 'haitaka', 'haitaka_types'], cwd=ROOT)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(dest, filter='data')
else:
    for name in ['haitaka', 'haitaka_types']:
        shutil.copytree(ROOT / name, dest / name)
fingerprint = hashlib.sha256()
for path in sorted((dest / 'haitaka/src').rglob('*.rs')):
    fingerprint.update(str(path.relative_to(dest)).encode())
    fingerprint.update(path.read_bytes())
source_hash = fingerprint.hexdigest()
lib = dest / 'haitaka/src/lib.rs'
lib.write_text(lib.read_text() + '\npub mod hash_probe;\n')
p = dest / 'haitaka/src/extended.rs'
source = p.read_text()
needle = 'pub fn hash(&self) -> u64 {'
assert source.count(needle) == 1
p.write_text(source.replace(needle, needle + '\nlet _probe = crate::hash_probe::Span::new();'))
(dest / 'haitaka/src/hash_probe.rs').write_text('''use std::{cell::Cell,time::Instant};
thread_local! {static STATS:Cell<(u64,u128)>=const {Cell::new((0,0))};}
pub fn reset(){STATS.with(|s|s.set((0,0)));}
pub fn stats()->(u64,u128){STATS.with(Cell::get)}
pub struct Span(Instant);
impl Span {pub fn new()->Self{Self(Instant::now())}}
impl Drop for Span{fn drop(&mut self){let ns=self.0.elapsed().as_nanos();STATS.with(|s|{let(n,t)=s.get();s.set((n+1,t+ns));});}}
''')
bench = dest / 'bench'
(bench / 'src').mkdir(parents=True)
(bench / 'src/main.rs').write_text(Path(__file__).with_suffix('.rs').read_text())
(bench / 'Cargo.lock').write_bytes((ROOT / 'Cargo.lock').read_bytes())
version = next(p['version'] for p in tomllib.loads((ROOT/'Cargo.lock').read_text())['package'] if p['name'] == 'serde_json')
manifest = '''[package]
name = "haitaka-hash-profile"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
haitaka = { path = "../haitaka", default-features = false }
serde_json = "={serde_version}"
[profile.release]
opt-level = 3
codegen-units = 1
lto = "fat"
[features]
'''
for rule in RULES:
    manifest += rule + ' = ["haitaka/' + rule + '"]\n'
(bench / 'Cargo.toml').write_text(manifest.replace('{serde_version}', version))
(dest / 'bins').mkdir()
for rule in RULES:
    with (dest / (rule + '-build.log')).open('w') as log:
        subprocess.run(['cargo', 'build', '--offline', '--release', '--manifest-path', str(bench/'Cargo.toml'), '--target-dir', str(ROOT/'target'), '--features', rule], cwd=ROOT, stdout=log, stderr=log, check=True)
    shutil.copy2(ROOT/'target/release/haitaka-hash-profile', dest/'bins'/rule)
    print('built profile', rule, flush=True)
baseline = json.loads((ROOT/'plans/historical-shogi-performance/baseline/perft.json').read_text())
report = {
    'revision': args.revision or 'working-tree', 'label': args.label,
    'engineSnapshotSha256': source_hash, 'cpuAffinity': args.cpu,
    'compiler': subprocess.check_output(['rustc', '--version'], text=True).strip(),
    'baselineSha256': hashlib.sha256((ROOT/'plans/historical-shogi-performance/baseline/perft.json').read_bytes()).hexdigest(),
    'method': 'Isolated source; RAII timer only in full FNV hash. Warmup excluded, >=3 seconds x3. Inclusive hash fraction includes timer perturbation; diagnostic, not production NPS.',
    'rules': [],
}
for rule in RULES:
    expected = next(r['result'] for r in baseline['rules'] if r['name'] == rule)
    result = subprocess.check_output(['taskset', '-c', str(args.cpu), str(dest/'bins'/rule), rule, str(expected['depth'])], text=True, cwd=ROOT)
    measured = json.loads(result)
    assert all(s['nodesPerPerft'] == expected['nodesPerPerft'] for s in measured['samples'])
    report['rules'].append(measured)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2)+'\n')
    print('profiled', rule, flush=True)
