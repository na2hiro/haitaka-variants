from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import select
import statistics
import subprocess
import time

counter_fields = ('pvsProbes', 'pvsResearches', 'lmrReductions', 'lmrResearches')
movetime = 1000
max_depth = 64

def pin(worker, cpu):
    os.sched_setaffinity(worker.process.pid, {cpu})

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def make_usi(mv):
    squares = [mv['from'], *mv['path']]
    return ''.join(f'{file}{chr(96 + rank)}' for file, rank in squares) + ('+' if mv['promotion'] else '')

class Usi:
    def __init__(self, binary, log, cpu):
        self.log = log.open('wb')
        self.process = subprocess.Popen(
            ['taskset', '-c', str(cpu), str(binary), 'usi', '--eval', 'handcrafted',
             '--movetime-max-depth', str(max_depth)], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=self.log, bufsize=0,
        )
        self.buffer = b''
        self.send('usi\nisready')
        self.read_until('usiok', 10)
        self.read_until('readyok', 10)

    def send(self, text):
        pending = memoryview((text + '\n').encode())
        while pending:
            pending = pending[self.process.stdin.write(pending):]

    def read_until(self, prefix, timeout):
        deadline = time.monotonic() + timeout
        lines = []
        while True:
            while b'\n' not in self.buffer:
                left = deadline - time.monotonic()
                if left <= 0 or not select.select([self.process.stdout], [], [], left)[0]:
                    raise RuntimeError(f'USI watchdog waiting for {prefix}: {lines}')
                data = os.read(self.process.stdout.fileno(), 65536)
                if not data:
                    raise RuntimeError(f'USI exited waiting for {prefix}: {self.process.poll()}')
                self.buffer += data
            raw, self.buffer = self.buffer.split(b'\n', 1)
            line = raw.decode()
            if line.startswith(('info string invalid', 'info string unsupported')):
                raise RuntimeError(line)
            lines.append(line)
            if line.startswith(prefix):
                return lines

    def search(self, history):
        command = 'position startpos'
        if history:
            command += ' moves ' + ' '.join(history)
        began = time.monotonic()
        self.send(f'{command}\ngo movetime {movetime} depth {max_depth}')
        lines = self.read_until('bestmove ', 15)
        wall_ms = 1000 * (time.monotonic() - began)
        infos = [line for line in lines if line.startswith('info depth ')]
        assert len(infos) == 1, lines
        words = infos[0].split(' pv ', 1)[0].replace(' score cp ', ' score ').split()
        stats = {k: int(v) for k, v in zip(words[1::2], words[2::2], strict=True)}
        for key in ('depth', 'time', 'nodes', 'score', *counter_fields):
            assert key in stats, infos
        return dict(best=lines[-1].split()[1], **stats, rpcWallMs=wall_ms)

    def close(self):
        try:
            if self.process.poll() is None:
                self.send('quit')
                try:
                    self.process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait()
        finally:
            self.process.stdin.close()
            self.process.stdout.close()
            self.log.close()

