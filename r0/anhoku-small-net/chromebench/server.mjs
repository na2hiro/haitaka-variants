// usage: node server.mjs <root> <port> <query> ; launches headless Chrome, samples Chrome RSS, prints the posted result
import http from 'http'; import fs from 'fs'; import path from 'path'; import { spawn, execSync } from 'child_process';
const [root, port, query] = process.argv.slice(2);
const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm' };
let chrome, peakKb = 0, timer;
const userDir = fs.mkdtempSync(path.join(process.env.TMPDIR || '/tmp', 'cb-'));
const server = http.createServer((req, res) => {
  if (req.method === 'POST' && req.url === '/result') {
    let body = ''; req.on('data', (d) => body += d); req.on('end', () => {
      res.end('ok'); const r = JSON.parse(body); r.chromePeakRssMB = +(peakKb / 1024).toFixed(1);
      console.log(JSON.stringify(r)); clearInterval(timer); chrome.kill('SIGTERM'); server.close(); setTimeout(() => { fs.rmSync(userDir, { recursive: true, force: true }); process.exit(0); }, 500);
    }); return;
  }
  const p = path.join(root, decodeURIComponent(req.url.split('?')[0]));
  fs.readFile(p, (err, data) => { if (err) { res.statusCode = 404; return res.end(); } res.setHeader('Content-Type', types[path.extname(p)] || 'application/octet-stream'); res.end(data); });
});
server.listen(+port, '127.0.0.1', () => {
  chrome = spawn('google-chrome', ['--headless=new', '--no-first-run', '--disable-gpu', `--user-data-dir=${userDir}`, `http://127.0.0.1:${port}/page.html?${query}`], { stdio: 'ignore' });
  timer = setInterval(() => {
    try { const kb = execSync(`ps -o rss= --ppid ${chrome.pid} -p ${chrome.pid} 2>/dev/null; pgrep -f ${userDir} | xargs -r ps -o rss= -p`).toString().split('\n').map(Number).filter(Boolean);
      // sum distinct processes' RSS for this Chrome instance
      const total = execSync(`pgrep -f ${userDir} | xargs -r ps -o rss= -p`).toString().split('\n').map(Number).filter(Boolean).reduce((a, b) => a + b, 0);
      peakKb = Math.max(peakKb, total); } catch {}
  }, 250);
  setTimeout(() => { console.log(JSON.stringify({ error: 'timeout' })); chrome.kill('SIGTERM'); process.exit(1); }, 120000);
});
