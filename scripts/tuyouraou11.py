#!/usr/bin/env python3
"""Pinned micro artifacts, external reference build/fixtures, explicit verification.

No source from the GPL reference is linked into Haitaka. The external diagnostic
binary remains GPLv3 and is built under ignored target/ only.
"""
import argparse
import ctypes
import ctypes.util
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
META = ROOT / 'plans/tuyouraou11'
WORK = ROOT / 'target/tuyouraou11'
MODEL = WORK / 'model'
PROV = json.loads((META / 'provenance.json').read_text())
COMMIT = PROV['reference']['commit']
SOURCE = WORK / 'reference' / ('yaneurao-YaneuraOu-' + COMMIT[:7])


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def download(url, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url, timeout=60) as response:
        path.write_bytes(response.read())


def check_model():
    for item in PROV['files'].values():
        path = MODEL / item['path']
        if path.stat().st_size != item['bytes'] or digest(path) != item['sha256']:
            raise ValueError('model size/hash mismatch: ' + str(path))
    print('PASS: both model sizes and SHA-256', flush=True)


def unpack_7z(path):
    """Use system libarchive, keep just the two licensed parameter files."""
    library = ctypes.util.find_library('archive')
    if not library:
        raise RuntimeError('libarchive shared library required (or extract the two bins manually)')
    a = ctypes.CDLL(library)
    ptr = ctypes.c_void_p
    a.archive_read_new.restype = ptr
    for name in ['archive_read_support_filter_all', 'archive_read_support_format_all', 'archive_read_free']:
        getattr(a, name).argtypes = [ptr]
    a.archive_read_open_filename.argtypes = [ptr, ctypes.c_char_p, ctypes.c_size_t]
    a.archive_read_next_header.argtypes = [ptr, ctypes.POINTER(ptr)]
    a.archive_entry_pathname.argtypes = [ptr]
    a.archive_entry_pathname.restype = ctypes.c_char_p
    a.archive_entry_size.argtypes = [ptr]
    a.archive_entry_size.restype = ctypes.c_int64
    a.archive_read_data.argtypes = [ptr, ptr, ctypes.c_size_t]
    a.archive_read_data.restype = ctypes.c_ssize_t
    reader = a.archive_read_new()
    try:
        a.archive_read_support_filter_all(reader)
        a.archive_read_support_format_all(reader)
        if a.archive_read_open_filename(reader, os.fsencode(path), 65536) != 0:
            raise ValueError('cannot open 7z')
        while True:
            entry = ptr()
            status = a.archive_read_next_header(reader, ctypes.byref(entry))
            if status == 1:
                break
            if status != 0:
                raise ValueError('archive header error')
            name = Path(a.archive_entry_pathname(entry).decode()).name
            if name not in ['nn.bin', 'progress.bin']:
                continue
            size = a.archive_entry_size(entry)
            if size != next(v['bytes'] for v in PROV['files'].values() if v['path'] == name):
                raise ValueError('unexpected model size in archive')
            data = bytearray()
            buffer = ctypes.create_string_buffer(65536)
            while len(data) < size:
                n = a.archive_read_data(reader, buffer, min(65536, size - len(data)))
                if n <= 0:
                    raise ValueError('truncated archive entry')
                data.extend(buffer.raw[:n])
            (MODEL / name).write_bytes(data)
    finally:
        a.archive_read_free(reader)


def fetch():
    archive = MODEL / 'micro.7z'
    if not archive.exists():
        download(PROV['archiveUrl'], archive)
    if digest(archive) != PROV['archiveSha256']:
        raise ValueError('distribution archive SHA-256 mismatch')
    unpack_7z(archive)
    check_model()
    shutil.copyfile(META / 'provenance.json', MODEL / 'provenance.json')
    shutil.copyfile(META / 'MODEL-LICENSE.txt', MODEL / 'LICENSE.txt')


def reference():
    archive = WORK / 'reference' / ('source-' + COMMIT + '.tar.gz')
    if not SOURCE.exists():
        download('https://api.github.com/repos/yaneurao/YaneuraOu/tarball/' + COMMIT, archive)
        with tarfile.open(archive) as stream:
            stream.extractall(archive.parent, filter='data')
    # A diagnostic-only extension. Upstream evaluation functions are unchanged.
    path = SOURCE / 'source/eval/nnue/nnue_test_command.cpp'
    text = path.read_text(encoding='utf-8-sig')
    if 'sub_command == "p1ref"' not in text:
        marker = 'if (sub_command == "test_features") {'
        if text.count(marker) != 1:
            raise ValueError('pinned reference insertion point changed')
        snippet = (ROOT / 'scripts/tuyouraou11-reference-command.inc').read_text()
        text = text.replace(marker, snippet + marker)
        path.write_text(text)
    command = ['make', '-C', str(SOURCE / 'source'), '-j4', 'normal', 'COMPILER=g++',
               'PYTHON=python3', 'TARGET_CPU=OTHER',
               'YANEURAOU_EDITION=YANEURAOU_ENGINE_SFNN_ka2_2048_8_64_progress8']
    with (WORK / 'reference/build-instrumented.log').open('w') as log:
        subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    print('PASS: external reference built at', COMMIT, flush=True)


def fixtures():
    check_model()
    command = ['cargo', 'run', '-p', 'haitaka_wasm', '--example', 'sfnn_positions', '--release']
    positions = subprocess.check_output(command, cwd=ROOT, text=True)
    (WORK / 'positions.tsv').write_text(positions)
    rows = [line.split('\t') for line in positions.splitlines()]
    commands = ['usi', 'setoption name Threads value 1', 'setoption name USI_Hash value 16',
                'setoption name BookFile value no_book', 'setoption name FV_SCALE value 24',
                'setoption name EvalDir value ' + str(MODEL), 'isready']
    for _, sfen in rows:
        commands.extend(['position sfen ' + sfen, 'test nn p1ref'])
    commands.append('quit')
    binary = SOURCE / 'source/YaneuraOu-by-gcc'
    result = subprocess.run([str(binary)], input='\n'.join(commands)+'\n', text=True,
                            capture_output=True, timeout=120, check=True, cwd=ROOT)
    (WORK / 'reference/fixture-output.log').write_text(result.stdout + result.stderr)
    if 'hash mismatch' in result.stdout or 'params read failed' in result.stdout:
        raise ValueError('reference model load mismatch')
    values = [line.split()[1:] for line in result.stdout.splitlines() if line.startswith('P1REF ')]
    if len(values) != len(rows) or len(rows) < 64:
        raise ValueError(f'incomplete reference output: {len(values)}/{len(rows)}')
    cases = []
    for (ident, sfen), parts in zip(rows, values):
        black, white, selected, bucket, score = map(int, parts)
        ply = int(ident.split(':')[1])
        tags = ['black-to-move' if sfen.split()[1] == 'b' else 'white-to-move',
                'opening' if ply < 32 else ('middlegame' if ply < 128 else 'late-legal-play')]
        if '+' in sfen.split()[0]: tags.append('promoted-piece')
        if sfen.split()[2] != '-': tags.append('hand')
        tags.append('king-position')
        cases.append({'id': ident, 'sfen': sfen, 'tags': tags,
                      'progress': {'sumsQ16': {'black': str(black), 'white': str(white)},
                                   'selectedSumQ16': str(selected), 'bucket': bucket}, 'evalCp': score})
    fixture = {'schemaVersion': 1, 'arch': PROV['arch'],
               'model': {'id': PROV['id'], 'nnSha256': PROV['files']['nn']['sha256'],
                         'progressSha256': PROV['files']['progress']['sha256']},
               'reference': {'repository': PROV['reference']['repository'], 'commit': COMMIT,
                             'command': ['python3', 'scripts/tuyouraou11.py', 'fixtures'],
                             'buildFlags': ['TARGET_CPU=OTHER', 'COMPILER=g++', 'PYTHON=python3',
                                            'YANEURAOU_EDITION=YANEURAOU_ENGINE_SFNN_ka2_2048_8_64_progress8'],
                             'fvScale': 24, 'generator': 'haitaka_wasm/examples/sfnn_positions.rs; LCG64 seed=1; sha256=' + digest(ROOT / 'haitaka_wasm/examples/sfnn_positions.rs')},
               'cases': cases}
    import jsonschema
    jsonschema.Draft202012Validator(json.loads((META / 'reference-fixture.schema.json').read_text())).validate(fixture)
    (META / 'reference-fixture.json').write_text(json.dumps(fixture, ensure_ascii=False, indent=2)+'\n')
    print('PASS: generated', len(cases), 'external reference positions', flush=True)


def verify():
    check_model()
    import jsonschema
    fixture = json.loads((META / 'reference-fixture.json').read_text())
    jsonschema.Draft202012Validator(json.loads((META / 'reference-fixture.schema.json').read_text())).validate(fixture)
    if fixture['reference']['commit'] != COMMIT or fixture['model']['id'] != PROV['id']:
        raise ValueError('reference/provenance metadata mismatch')
    env = dict(os.environ, HAITAKA_SFNN_MODEL_DIR=str(MODEL),
               HAITAKA_SFNN_CORPUS=str(WORK / 'p2-corpus.json'))
    subprocess.run(['cargo', 'test', '-p', 'haitaka_wasm', '--release', '--test', 'sfnn_reference',
                    '--', '--ignored', '--nocapture'], cwd=ROOT, env=env, check=True)
    subprocess.run(['cargo', 'test', '-p', 'haitaka_wasm', '--release', '--lib', 'sfnn_',
                    '--', '--ignored', '--nocapture', '--test-threads=1'], cwd=ROOT, env=env, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['fetch', 'reference', 'fixtures', 'verify'])
    args = parser.parse_args()
    WORK.mkdir(parents=True, exist_ok=True)
    globals()[args.action]()


if __name__ == '__main__':
    main()
