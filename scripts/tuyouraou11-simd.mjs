// P6: build and compare actual scalar/SIMD Wasm against the frozen P1/P2 corpus.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
const root = resolve(import.meta.dirname, "..");
const mini = process.argv.includes("--mini");
const out = join(root, `target/tuyouraou11/${mini ? "p6-mini" : "p6"}`);
await mkdir(out, { recursive: true });
const backends = ["scalar", "simd"];
if (process.argv.includes("--build")) {
  for (const backend of backends) {
    const env = {
      ...process.env,
      RUSTFLAGS: `-C target-feature=${backend === "simd" ? "+" : "-"}simd128`,
    };
    delete env.CARGO_ENCODED_RUSTFLAGS;
    execFileSync(
      "cargo",
      [
        "build",
        "-p",
        "haitaka_wasm",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
        "--features",
        "sfnn-diagnostics",
        "--target-dir",
        join(out, "build"),
      ],
      { cwd: root, env, stdio: "inherit" }
    );
    execFileSync(
      "wasm-bindgen",
      [
        "--target",
        "web",
        "--out-dir",
        join(out, `pkg-${backend}`),
        "--out-name",
        "haitaka_wasm",
        join(out, "build/wasm32-unknown-unknown/release/haitaka_wasm.wasm"),
      ],
      { cwd: root, stdio: "inherit" }
    );
  }
}
const fixture = JSON.parse(
  await readFile(
    join(
      root,
      `plans/tuyouraou11/${
        mini ? "mini-reference-fixture.json" : "reference-fixture.json"
      }`
    )
  )
);
const corpus = JSON.parse(
  await readFile(
    join(
      root,
      `target/tuyouraou11/${mini ? "mini-corpus.json" : "p2-corpus.json"}`
    )
  )
);
const positions = JSON.parse(
  await readFile(join(root, "plans/tuyouraou11/p4-positions.json"))
);
const nn = await readFile(
  join(root, `target/tuyouraou11/${mini ? "mini-model" : "model"}/nn.bin`)
);
const progress = await readFile(
  join(root, `target/tuyouraou11/${mini ? "mini-model" : "model"}/progress.bin`)
);
const cases = [
  ...fixture.cases.map((c) => ({
    sfen: c.sfen,
    moves: "",
    traces: [
      { progress: c.progress, evalCp: c.evalCp, fullRefreshMatches: true },
    ],
  })),
  ...corpus.map((c) => ({
    sfen: c.startSfen,
    moves: c.moves.join(" "),
    traces: c.traces,
  })),
];
const hashes = {};
const snap = (result) => {
  const value = {
    bestMove: result.bestMove,
    score: result.lastCompletedIterationValue,
    states: result.states,
  };
  result.free();
  return value;
};
const searches = {};
for (const backend of backends) {
  const pkg = join(out, `pkg-${backend}`);
  const wasm = await readFile(join(pkg, "haitaka_wasm_bg.wasm"));
  hashes[backend] = createHash("sha256").update(wasm).digest("hex");
  const engine = await import(pathToFileURL(join(pkg, "haitaka_wasm.js")));
  await engine.default({ module_or_path: wasm });
  engine.set_hash_size_mb(1);
  engine.load_sfnn(nn, progress, fixture.arch);
  for (const c of cases)
    assert.deepEqual(JSON.parse(engine.sfnn_trace(c.sfen, c.moves)), c.traces);
  searches[backend] = positions.map((p) => snap(engine.search(p.sfen, 2)));
}
assert.deepEqual(
  searches.simd,
  searches.scalar,
  "same-depth search scores, moves and nodes must match"
);
const report = {
  fixturePositions: fixture.cases.length,
  incrementalMoves: corpus.reduce((n, c) => n + c.moves.length, 0),
  fixedDepthSearches: positions.length,
  hashes,
  searches,
  result: "PASS",
};
await writeFile(
  join(out, "wasm-comparison.json"),
  JSON.stringify(report, null, 2)
);
console.log(JSON.stringify({ ...report, searches: undefined }));
if (process.argv.includes("--browser")) {
  const { createRequire } = await import("node:module");
  const { createServer } = await import("node:http");
  const require = createRequire(
    join(
      resolve(root, process.env.SHOGITTER_DIR ?? "../shogitter-3"),
      "package.json"
    )
  );
  const { chromium, firefox } = require("@playwright/test");
  const worker = `import * as engine from './engine.js';
self.onmessage = async ({data}) => { try {
 await engine.default(); engine.set_hash_size_mb(1);
 const nn = new Uint8Array(await (await fetch('/nn.bin')).arrayBuffer());
 const progress = new Uint8Array(await (await fetch('/progress.bin')).arrayBuffer());
 engine.load_sfnn(nn, progress, data.arch);
 const traces = [];
 for (let i = 0; i < data.cases.length; i++) {
   const c = data.cases[i]; traces.push(JSON.parse(engine.sfnn_trace(c.sfen, c.moves)));
   if (i % 32 === 0) self.postMessage({ progress: i });
 }
 const searches = data.positions.map(p => { const r = engine.search(p.sfen,2); const s={ bestMove:r.bestMove,score:r.lastCompletedIterationValue,states:r.states }; r.free();return s; });
 self.postMessage({traces, searches});
} catch(e) { self.postMessage({error:String(e)}); } };`;
  const resources = new Map([
    [
      "/",
      ["text/html", "<!doctype html><title>P6 exact SIMD comparison</title>"],
    ],
    ["/nn.bin", ["application/octet-stream", nn]],
    ["/progress.bin", ["application/octet-stream", progress]],
  ]);
  for (const backend of backends) {
    const pkg = join(out, `pkg-${backend}`);
    resources.set(`/${backend}/worker.js`, ["text/javascript", worker]);
    resources.set(`/${backend}/engine.js`, [
      "text/javascript",
      await readFile(join(pkg, "haitaka_wasm.js")),
    ]);
    resources.set(`/${backend}/haitaka_wasm_bg.wasm`, [
      "application/wasm",
      await readFile(join(pkg, "haitaka_wasm_bg.wasm")),
    ]);
  }
  const server = createServer((req, res) => {
    const r = resources.get(req.url);
    if (!r) {
      res.writeHead(404);
      res.end();
      return;
    }
    res.writeHead(200, { "Content-Type": r[0] });
    res.end(r[1]);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const browsers = [];
  try {
    for (const [name, type] of [
      ["chromium", chromium],
      ["firefox", firefox],
    ]) {
      if (
        process.env.HAITAKA_BROWSERS &&
        !process.env.HAITAKA_BROWSERS.split(",").includes(name)
      )
        continue;
      const browser = await type.launch({
        headless: true,
        ...(name === "chromium" && process.env.HAITAKA_CHROMIUM_PATH
          ? { executablePath: process.env.HAITAKA_CHROMIUM_PATH }
          : {}),
      });
      try {
        const page = await browser.newPage();
        page.on("console", (message) =>
          console.log(name + ": " + message.text())
        );
        await page.goto(`http://127.0.0.1:${server.address().port}/`);
        for (const backend of backends) {
          const actual = await page.evaluate(
            ({ backend, cases, positions, arch }) =>
              new Promise((resolve, reject) => {
                const worker = new Worker(`/${backend}/worker.js`, {
                  type: "module",
                });
                const timeout = setTimeout(() => {
                  worker.terminate();
                  reject(new Error("P6 timeout"));
                }, 600000);
                worker.onerror = (e) => {
                  clearTimeout(timeout);
                  worker.terminate();
                  reject(new Error(e.message));
                };
                worker.onmessage = ({ data }) => {
                  if (data.progress !== undefined) {
                    console.log(backend + " reference cases: " + data.progress);
                    return;
                  }
                  clearTimeout(timeout);
                  worker.terminate();
                  resolve(data);
                };
                worker.postMessage({ cases, positions, arch });
              }),
            { backend, cases, positions, arch: fixture.arch }
          );
          assert.equal(actual.error, undefined);
          assert.deepEqual(
            actual.traces,
            cases.map((c) => c.traces)
          );
          assert.deepEqual(actual.searches, searches.scalar);
          browsers.push({
            browser: name,
            version: browser.version(),
            backend,
            result: "PASS",
          });
          console.log(JSON.stringify(browsers.at(-1)));
        }
      } finally {
        await browser.close();
      }
    }
    await writeFile(
      join(out, "browser-comparison.json"),
      JSON.stringify({ ...report, browsers }, null, 2)
    );
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}
