// P2: execute generated --target web glue and actual scalar Wasm (no mocks).
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
const pkg = resolve(root, process.argv.slice(2).find(a => !a.startsWith('--')) ?? 'target/tuyouraou11/pkg-diagnostics');
const [nn, progress, wasm] = await Promise.all(['nn.bin', 'progress.bin'].map(n => readFile(join(root, 'target/tuyouraou11/model', n))).concat(readFile(join(pkg, 'haitaka_wasm_bg.wasm'))));
const fixture = JSON.parse(await readFile(join(root, 'plans/tuyouraou11/reference-fixture.json'), 'utf8'));
const corpus = JSON.parse(await readFile(join(root, 'target/tuyouraou11/p2-corpus.json'), 'utf8'));
const mod = await import(pathToFileURL(join(pkg, 'haitaka_wasm.js')).href);
await mod.default({ module_or_path: wasm });
mod.set_hash_size_mb(1);
const arch = fixture.arch;
const start = corpus[0].startSfen;
const snap = result => {
  const value = { bestMove: result.bestMove, score: result.lastCompletedIterationValue, states: result.states };
  result.free(); return value;
};
const scoreOfSession = session => {
  const lines = session.send('go depth 1');
  const info = lines.find(s => s.includes('lastCompletedIterationValue'));
  return Number(/lastCompletedIterationValue (-?\d+)/.exec(info)[1]);
};
const baseline = snap(mod.search(start, 1));
const unloadedSession = new mod.UsiEngine();
unloadedSession.send('setoption name Hash value 1');
assert.equal(scoreOfSession(unloadedSession), baseline.score);
assert.throws(() => mod.load_sfnn(nn, new Uint8Array(), arch), /SFNN_INVALID_PROGRESS:/);
assert.deepEqual(snap(mod.search(start, 1)), baseline);
assert.equal(mod.load_sfnn(nn, progress, arch), arch);
const active = snap(mod.search(start, 1));
assert.ok(active.bestMove && active.states > 0);
assert.equal(scoreOfSession(unloadedSession), baseline.score, 'global load must not affect existing session');
assert.equal(unloadedSession.load_sfnn(nn, progress, arch), arch);
assert.equal(scoreOfSession(unloadedSession), active.score);
assert.throws(() => unloadedSession.load_sfnn(nn, new Uint8Array(), arch), /SFNN_INVALID_PROGRESS:/);
assert.equal(scoreOfSession(unloadedSession), active.score);
assert.throws(() => mod.load_sfnn(nn, progress, 'mini'), /SFNN_UNSUPPORTED_ARCH:/);
assert.throws(() => mod.load_nnue(new Uint8Array()), /failed to load NNUE/);
assert.deepEqual(snap(mod.search(start, 1)), active);
let comparisons = 0;
if (typeof mod.sfnn_trace === 'function') {
  for (const c of fixture.cases) {
    const [trace] = JSON.parse(mod.sfnn_trace(c.sfen, ''));
    assert.deepEqual(trace, { progress: c.progress, evalCp: c.evalCp, fullRefreshMatches: true }, c.id);
  }
  for (const game of corpus) {
    const actual = JSON.parse(mod.sfnn_trace(game.startSfen, game.moves.join(' ')));
    assert.deepEqual(actual, game.traces, 'native/Wasm incremental trace mismatch');
    comparisons += game.moves.length;
  }
  assert.equal(comparisons, 2048);
} else {
  assert.ok(process.argv.includes('--production'), 'diagnostic export required for exact trace comparisons');
}
const iterative = mod.search_iterative_deepening(start, 64, 50);
assert.ok(iterative.bestMove);
iterative.free();
unloadedSession.free();
console.log(JSON.stringify({ runtime: 'Node WebAssembly / web glue', fixturePositions: comparisons ? fixture.cases.length : 0, incrementalMoves: comparisons, baseline, sfnn: active, production: process.argv.includes('--production'), result: 'PASS' }));

if (process.argv.includes('--browser')) {
  const { createRequire } = await import('node:module');
  const { createServer } = await import('node:http');
  const require = createRequire(join(resolve(root, process.env.SHOGITTER_DIR ?? '../shogitter-3'), 'package.json'));
  const { chromium } = require('@playwright/test');
  const workerSource = `
    import * as engine from '/engine.js';
    self.onmessage = async () => {
      try {
        const [wasm, nn, progress, fixture, corpus] = await Promise.all([
          fetch('/engine.wasm').then(r => r.arrayBuffer()),
          fetch('/nn.bin').then(r => r.arrayBuffer()), fetch('/progress.bin').then(r => r.arrayBuffer()),
          fetch('/fixture.json').then(r => r.json()), fetch('/corpus.json').then(r => r.json())]);
        await engine.default({ module_or_path: wasm });
        engine.set_hash_size_mb(1);
        const arch = engine.load_sfnn(new Uint8Array(nn), new Uint8Array(progress), fixture.arch);
        const fixtureTraces = engine.sfnn_trace ? fixture.cases.map(c => JSON.parse(engine.sfnn_trace(c.sfen, ''))[0]) : [];
        const corpusTraces = engine.sfnn_trace ? corpus.map(c => JSON.parse(engine.sfnn_trace(c.startSfen, c.moves.join(' ')))) : [];
        const result = engine.search(corpus[0].startSfen, 1);
        const search = { bestMove: result.bestMove, score: result.lastCompletedIterationValue, states: result.states };
        result.free();
        const session = new engine.UsiEngine();
        const sessionArch = session.load_sfnn(new Uint8Array(nn), new Uint8Array(progress), fixture.arch);
        const lines = session.send('go movetime 30');
        session.free();
        self.postMessage({ arch, sessionArch, fixtureTraces, corpusTraces, search, lines });
      } catch(e) { self.postMessage({ error: String(e) }); }
    };
  `;
  const resources = new Map([
    ['/', ['text/html', '<!doctype html><title>P2 Wasm Worker test</title>']],
    ['/worker.js', ['text/javascript', workerSource]],
    ['/engine.js', ['text/javascript', await readFile(join(pkg, 'haitaka_wasm.js'))]],
    ['/engine.wasm', ['application/wasm', wasm]], ['/nn.bin', ['application/octet-stream', nn]],
    ['/progress.bin', ['application/octet-stream', progress]],
    ['/fixture.json', ['application/json', JSON.stringify(fixture)]],
    ['/corpus.json', ['application/json', JSON.stringify(corpus)]],
  ]);
  const server = createServer((req, res) => {
    const resource = resources.get(req.url);
    if (!resource) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { 'Content-Type': resource[0] }); res.end(resource[1]);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch({ headless: true, ...(process.env.HAITAKA_CHROMIUM_PATH ? { executablePath: process.env.HAITAKA_CHROMIUM_PATH } : {}) });
    const page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    const report = await page.evaluate(() => new Promise((resolve, reject) => {
      let ticks = 0;
      const interval = setInterval(() => ticks++, 10);
      const worker = new Worker('/worker.js', { type: 'module' });
      const timeout = setTimeout(() => { worker.terminate(); clearInterval(interval); reject(new Error('worker timeout')); }, 60000);
      worker.onerror = e => { worker.terminate(); clearInterval(interval); clearTimeout(timeout); reject(new Error(e.message)); };
      worker.onmessage = e => { worker.terminate(); clearInterval(interval); clearTimeout(timeout); resolve({ ...e.data, ticks }); };
      worker.postMessage('run');
    }));
    assert.equal(report.error, undefined);
    assert.equal(report.arch, arch);
    assert.equal(report.sessionArch, arch);
    assert.ok(report.ticks > 0, 'main page must keep processing tasks while worker evaluates');
    assert.deepEqual(report.search, active);
    assert.ok(report.lines.at(-1).startsWith('bestmove '));
    if (comparisons) {
      assert.deepEqual(report.fixtureTraces, fixture.cases.map(c => ({ progress: c.progress, evalCp: c.evalCp, fullRefreshMatches: true })));
      assert.deepEqual(report.corpusTraces, corpus.map(c => c.traces));
    }
    console.log(JSON.stringify({ runtime: 'Chromium module Web Worker', version: browser.version(), fixturePositions: report.fixtureTraces.length, incrementalMoves: comparisons, mainThreadTicks: report.ticks, result: 'PASS' }));
  } finally {
    await browser?.close();
    await new Promise(resolve => server.close(resolve));
  }
}
