async function verifyProvenance(config) {
  const fetched = {};
  const verified = {};
  const fetchTimingsMs = {};
  for (const [name, route] of Object.entries(config.provenance.routes)) {
    const started = performance.now();
    const response = await fetch(route, { cache: 'no-store' });
    if (!response.ok) throw new Error(`provenance fetch failed for ${name}: ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const digestBytes = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const digest = Array.from(digestBytes, (value) => value.toString(16).padStart(2, '0')).join('');
    const expected = config.provenance.files[name];
    if (!expected || expected.bytes !== bytes.byteLength || expected.sha256 !== digest) throw new Error(`provenance mismatch for ${name}`);
    fetched[name] = bytes;
    verified[name] = { bytes: bytes.byteLength, sha256: digest, route };
    fetchTimingsMs[name] = performance.now() - started;
  }
  return { fetched, verified, fetchTimingsMs };
}

async function run(config) {
  const started = performance.now();
  const verified = await verifyProvenance(config);
  const acknowledgement = { schema: 'haitaka-anhoku-strength-first-browser-provenance-ack-v1', envelopeId: config.provenance.envelopeId, verifiedBeforePlay: true, verifiedFiles: verified.verified };
  const ack = await fetch('/provenance', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(acknowledgement) });
  if (!ack.ok) throw new Error(`provenance acknowledgement rejected: ${ack.status}`);
  const importStarted = performance.now();
  const wasm = await import('/pkg/haitaka_wasm.js');
  const glueImportMs = performance.now() - importStarted;
  const wasmInitStarted = performance.now();
  await wasm.default('/pkg/haitaka_wasm_bg.wasm');
  const wasmInitMs = performance.now() - wasmInitStarted;
  const modelLoadStarted = performance.now();
  const modelDescription = wasm.load_nnue(verified.fetched.modelA);
  const modelLoadMs = performance.now() - modelLoadStarted;
  const positions = config.positions.map((position) => ({ id: position.id, sfen: position.sfen, score: wasm.evaluate_nnue(position.sfen) }));
  return { schema: 'haitaka-anhoku-strength-first-browser-parity-v1', status: 'passed', provenance: config.provenance, execution: { runtime: 'desktop-google-chrome-headless-dedicated-module-worker', workerCount: 1, concurrentGames: 1, evaluation: 'web-target-WASM-evaluate_nnue-full-refresh', positions: positions.length }, load: { workerStartedAt: started, fetchTimingsMs: verified.fetchTimingsMs, glueImportMs, wasmInitMs, modelDescription, modelLoadMs, coldLoadMs: performance.now() - started }, positions };
}

self.onmessage = async ({ data }) => {
  try {
    self.postMessage({ type: 'result', result: await run(data) });
  } catch (error) {
    self.postMessage({ type: 'error', error: error?.stack ?? String(error) });
  }
};
