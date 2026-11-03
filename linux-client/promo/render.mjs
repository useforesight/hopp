#!/usr/bin/env node
// Renders film.html frame by frame through headless Chromium (DevTools protocol, no npm
// dependencies) and encodes the frames with ffmpeg.
//
//   node render.mjs --shots 8.2,15,27.5        # PNG stills into build/shots/
//   node render.mjs --fps 30 --workers 4       # full film → build/video.mp4 (no audio)
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir, cpus } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const args = Object.fromEntries(process.argv.slice(2).reduce((acc, a, i, all) =>
  a.startsWith('--') ? [...acc, [a.slice(2), all[i + 1]?.startsWith('--') || all[i + 1] === undefined ? '1' : all[i + 1]]] : acc, []));
const FPS = Number(args.fps ?? 30);
const WORKERS = Number(args.workers ?? Math.max(1, Math.min(6, Math.floor(cpus().length / 2))));
const CHROME = args.chrome ?? 'chromium';
// --film and --build pick another film and its scratch directory (default: this one's).
const FILM = args.film ? join(process.cwd(), args.film) : join(here, 'film.html');
const BUILD = args.build ? join(process.cwd(), args.build) : join(here, 'build');
const URL_ = pathToFileURL(FILM).href;

async function browser() {
  const profile = mkdtempSync(join(tmpdir(), 'hopp-promo-'));
  const proc = spawn(CHROME, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--hide-scrollbars',
    '--allow-file-access-from-files', '--force-color-profile=srgb', '--disable-background-timer-throttling',
    '--disable-renderer-backgrounding', '--window-size=1920,1080', 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
  const port = await new Promise((resolve, reject) => {
    let buf = '';
    proc.stderr.on('data', d => { buf += d; const m = buf.match(/DevTools listening on ws:\/\/[^:]+:(\d+)\//); if (m) resolve(m[1]); });
    proc.on('exit', c => reject(new Error(`chromium exited ${c}: ${buf}`)));
  });
  const { webSocketDebuggerUrl } = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  const ws = new WebSocket(webSocketDebuggerUrl);
  await new Promise(r => ws.addEventListener('open', r, { once: true }));
  let id = 0; const pending = new Map(), waiters = [];
  ws.addEventListener('message', ev => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) { const { resolve, reject } = pending.get(msg.id); pending.delete(msg.id); msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result); }
    else if (msg.method) for (const w of [...waiters]) if (w.method === msg.method) { waiters.splice(waiters.indexOf(w), 1); w.resolve(msg.params); }
  });
  const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const mid = ++id; pending.set(mid, { resolve, reject }); ws.send(JSON.stringify({ id: mid, method, params, sessionId }));
  });
  const once = method => new Promise(resolve => waiters.push({ method, resolve }));
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  const s = (m, p) => send(m, p, sessionId);
  await s('Page.enable');
  await s('Runtime.enable');
  await s('Emulation.setDeviceMetricsOverride', { width: 1920, height: 1080, deviceScaleFactor: 1, mobile: false });
  const loaded = once('Page.loadEventFired');
  await s('Page.navigate', { url: URL_ });
  await loaded;
  const ready = await s('Runtime.evaluate', { expression: 'window.ready.then(() => window.DUR)', awaitPromise: true, returnByValue: true });
  if (ready.exceptionDetails) throw new Error(JSON.stringify(ready.exceptionDetails));
  const dur = ready.result.value;
  return {
    dur,
    async frame(t, frame, format = 'png') {
      const r = await s('Runtime.evaluate', { expression: `render(${t}, ${frame})`, returnByValue: true });
      if (r.exceptionDetails) throw new Error(`t=${t}: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
      const shot = await s('Page.captureScreenshot', { format, ...(format === 'jpeg' ? { quality: 95 } : {}), optimizeForSpeed: true });
      return Buffer.from(shot.data, 'base64');
    },
    close() { ws.close(); proc.kill(); setTimeout(() => rmSync(profile, { recursive: true, force: true }), 500); },
  };
}

async function shots(list) {
  const out = join(BUILD, 'shots'); mkdirSync(out, { recursive: true });
  const b = await browser();
  for (const t of list) {
    const png = await b.frame(t, Math.round(t * FPS));
    const file = join(out, `t${t.toFixed(2).padStart(6, '0')}.png`); writeFileSync(file, png); console.log(file);
  }
  b.close();
}

async function film() {
  const probe = await browser(); const dur = probe.dur; probe.close();
  const total = Math.round(dur * FPS);
  const per = Math.ceil(total / WORKERS);
  const segs = [];
  const started = Date.now(); let done = 0;
  await Promise.all(Array.from({ length: WORKERS }, async (_, w) => {
    const from = w * per, to = Math.min(total, from + per);
    if (from >= to) return;
    const seg = join(BUILD, `seg${w}.mp4`); segs[w] = seg;
    const ff = spawn('ffmpeg', ['-loglevel', 'error', '-y', '-f', 'image2pipe', '-framerate', String(FPS), '-c:v', 'png', '-i', '-',
      '-c:v', 'libx264', '-preset', 'slow', '-crf', '17', '-tune', 'film', '-pix_fmt', 'yuv420p', '-r', String(FPS), seg], { stdio: ['pipe', 'inherit', 'inherit'] });
    const b = await browser();
    for (let f = from; f < to; f++) {
      const png = await b.frame(f / FPS, f);
      if (!ff.stdin.write(png)) await new Promise(r => ff.stdin.once('drain', r));
      if (++done % 30 === 0) {
        const el = (Date.now() - started) / 1000;
        process.stdout.write(`\r${done}/${total} frames · ${(done / el).toFixed(1)} fps · eta ${Math.round((total - done) / (done / el))}s   `);
      }
    }
    b.close();
    ff.stdin.end();
    await new Promise(r => ff.on('exit', r));
  }));
  const list = join(BUILD, 'segs.txt');
  writeFileSync(list, segs.filter(Boolean).map(s => `file '${s}'`).join('\n'));
  const out = join(BUILD, 'video.mp4');
  await new Promise((resolve, reject) => spawn('ffmpeg', ['-loglevel', 'error', '-y', '-f', 'concat', '-safe', '0', '-i', list, '-c', 'copy', out], { stdio: 'inherit' })
    .on('exit', c => c ? reject(new Error('concat failed')) : resolve()));
  console.log(`\n${out} (${total} frames in ${Math.round((Date.now() - started) / 1000)}s)`);
}

if (args.shots) await shots(args.shots.split(',').map(Number));
else await film();
process.exit(0);
