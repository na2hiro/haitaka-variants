self.onmessage = async (e) => {
  try {
    const { pkg, model, movetime } = e.data;
    const t0 = performance.now();
    const wasm = await import(`./${pkg}/haitaka_wasm.js`);
    await wasm.default(`./${pkg}/haitaka_wasm_bg.wasm`);
    const initMs = performance.now() - t0;
    const eng = new wasm.UsiEngine();
    let loadMs = 0, fetchMs = 0, desc = 'handcrafted', bytes = 0;
    if (model !== 'handcrafted') {
      const f0 = performance.now();
      const buf = new Uint8Array(await (await fetch(`./${model}`, { cache: 'no-store' })).arrayBuffer());
      fetchMs = performance.now() - f0; bytes = buf.byteLength;
      const l0 = performance.now(); desc = eng.load_nnue(buf); loadMs = performance.now() - l0;
    }
    const send = (c) => Array.from(eng.send(c), String);
    send('usi'); send('isready');
    const sfens = (await (await fetch('./positions.sfen')).text()).split('\n').filter(Boolean);
    const res = [];
    for (const sfen of sfens) {
      send('usinewgame'); send(`position sfen ${sfen}`);
      const t = performance.now(); const out = send(`go movetime ${movetime}`); const el = performance.now() - t;
      const last = out.filter(l => l.startsWith('info') && l.includes(' nodes ')).pop() || '';
      const tok = last.split(' '); const g = (k) => { const i = tok.indexOf(k); return i >= 0 ? Number(tok[i + 1]) : null; };
      res.push({ el, nodes: g('nodes'), depth: g('depth') });
    }
    const mean = (k) => res.reduce((a, r) => a + (r[k] || 0), 0) / res.length;
    self.postMessage({ pkg, model, desc, modelBytes: bytes, initMs, fetchMs, loadMs, positions: res.length,
      meanElapsedMs: mean('el'), maxElapsedMs: Math.max(...res.map(r => r.el)), meanNps: Math.round(mean('nodes') / (mean('el') / 1000)), meanDepth: mean('depth') });
  } catch (err) { self.postMessage({ error: String(err && err.stack || err) }); }
};
