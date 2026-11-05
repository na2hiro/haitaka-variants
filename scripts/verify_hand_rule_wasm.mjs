// Compare actual wasm-bindgen packages against native fixture evaluations.
import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';

const [fixtureDirectory, packageDirectory, networkPath] = process.argv.slice(2);
assert(fixtureDirectory && packageDirectory && networkPath,
  'Usage: node scripts/verify_hand_rule_wasm.mjs <fixtures> <packages> <smoke.nnue>');
const network = await readFile(networkPath);
const results = [];
for (const rule of ['nakamaware', 'fushiyo', 'share', 'zoshoku', 'babanuki', 'tensuu', 'suika']) {
  const directory = resolve(packageDirectory, rule);
  const api = await import(pathToFileURL(join(directory, 'haitaka_wasm.js')).href);
  api.initSync({ module: await readFile(join(directory, 'haitaka_wasm_bg.wasm')) });
  if (rule === 'zoshoku') {
    assert.throws(() => api.load_nnue(network), /zoshoku/);
  } else {
    api.load_nnue(network);
  }
  const fixtures = JSON.parse(await readFile(join(fixtureDirectory, `${rule}.json`), 'utf8'));
  for (const row of fixtures) {
    const perft = api.perft(row.sfen, 1);
    assert.equal(perft.nodes, row.nodes, `${rule}: move count for ${row.sfen}`);
    perft.free();
    if (row.eval !== null) {
      assert.equal(api.evaluate_nnue(row.sfen), row.eval, `${rule}: integer evaluation for ${row.sfen}`);
    }
    if (['nakamaware', 'suika'].includes(rule) && row.usi.length) {
      assert.throws(() => api.evaluate_nnue(row.sfen), /terminal King/);
    }
    const engine = new api.UsiEngine();
    assert.deepEqual(engine.send(`position sfen ${row.sfen}`), []);
    assert.deepEqual(engine.terminal_notifications(), row.usi, `${rule}: terminal notification`);
    engine.free();
  }
  results.push({ rule, positions: fixtures.length, nativeWasmAgreement: true });
}
console.log(JSON.stringify(results, null, 2));
