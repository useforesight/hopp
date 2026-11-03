#!/usr/bin/env node
// Synthesises the promo soundtrack (128 BPM, A minor) straight to build/music.wav.
// Every hit lines up with film.html's beat map: bar(n, beat) is the same T(n, beat).
import { writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const SR = 48000, BPM = 128, BEAT = 60 / BPM, BAR = 4 * BEAT, DUR = 25 * BAR, TAIL = 0.0;
const N = Math.ceil((DUR + TAIL) * SR);
const T = (bar, beat = 0) => bar * BAR + beat * BEAT;
const L = new Float32Array(N), R = new Float32Array(N);
const sendL = new Float32Array(N), sendR = new Float32Array(N);   // reverb bus
const duckBus = { L: new Float32Array(N), R: new Float32Array(N) }; // sidechained bus
const TAU = Math.PI * 2;
const mtof = m => 440 * Math.pow(2, (m - 69) / 12);
let seed = 0x9e3779b9;
const rnd = () => { seed ^= seed << 13; seed >>>= 0; seed ^= seed >>> 17; seed ^= seed << 5; seed >>>= 0; return seed / 4294967296 * 2 - 1; };

class Biquad {
  constructor(type, f, q = .707) { this.type = type; this.x1 = this.x2 = this.y1 = this.y2 = 0; this.set(f, q); }
  set(f, q = this.q) {
    this.q = q; f = Math.min(Math.max(f, 20), SR * .45);
    const w = TAU * f / SR, c = Math.cos(w), s = Math.sin(w), a = s / (2 * q);
    let b0, b1, b2, a0 = 1 + a, a1 = -2 * c, a2 = 1 - a;
    if (this.type === 'lp') { b0 = (1 - c) / 2; b1 = 1 - c; b2 = (1 - c) / 2; }
    else if (this.type === 'hp') { b0 = (1 + c) / 2; b1 = -(1 + c); b2 = (1 + c) / 2; }
    else { b0 = a; b1 = 0; b2 = -a; } // band-pass
    this.b0 = b0 / a0; this.b1 = b1 / a0; this.b2 = b2 / a0; this.a1 = a1 / a0; this.a2 = a2 / a0;
  }
  run(x) {
    const y = this.b0 * x + this.b1 * this.x1 + this.b2 * this.x2 - this.a1 * this.y1 - this.a2 * this.y2;
    this.x2 = this.x1; this.x1 = x; this.y2 = this.y1; this.y1 = y; return y;
  }
}
function put(bus, t0, i, v, pan = 0, send = 0) {
  const n = Math.round(t0 * SR) + i; if (n < 0 || n >= N) return;
  const gl = Math.cos((pan + 1) * Math.PI / 4), gr = Math.sin((pan + 1) * Math.PI / 4);
  bus.L[n] += v * gl * 1.414; bus.R[n] += v * gr * 1.414;
  if (send) { sendL[n] += v * gl * send; sendR[n] += v * gr * send; }
}
const DRY = { L, R };

// ---------------------------------------------------------------- drums
function kick(t0, amp = 1) {
  let ph = 0; const len = .5 * SR;
  for (let i = 0; i < len; i++) {
    const t = i / SR, f = 44 + 120 * Math.exp(-t * 30) + 40 * Math.exp(-t * 200);
    ph += TAU * f / SR;
    let v = Math.sin(ph) * Math.exp(-t * 6.5) * (t < .002 ? t / .002 : 1);
    v += (i < 120 ? rnd() * .5 * (1 - i / 120) : 0);
    put(DRY, t0, i, Math.tanh(v * 1.8) * .9 * amp);
  }
}
function clap(t0, amp = 1, send = .35) {
  const bp = new Biquad('bp', 1300, .9), hp = new Biquad('hp', 600);
  const len = .32 * SR;
  for (let i = 0; i < len; i++) {
    const t = i / SR;
    const env = [0, .011, .022].reduce((a, d) => a + (t >= d ? Math.exp(-(t - d) * (d === .022 ? 16 : 120)) : 0), 0);
    put(DRY, t0, i, hp.run(bp.run(rnd())) * env * .9 * amp, .05, send);
  }
}
function snare(t0, amp = 1, tone = 200) {
  const hp = new Biquad('hp', 1500); let ph = 0; const len = .2 * SR;
  for (let i = 0; i < len; i++) {
    const t = i / SR; ph += TAU * tone / SR;
    put(DRY, t0, i, (hp.run(rnd()) * .7 * Math.exp(-t * 22) + Math.sin(ph) * .4 * Math.exp(-t * 30)) * amp, 0, .25);
  }
}
function hat(t0, amp = 1, open = false, pan = .25) {
  const hp = new Biquad('hp', 8000, .9), hp2 = new Biquad('hp', 6000); const len = (open ? .3 : .06) * SR;
  for (let i = 0; i < len; i++) { const t = i / SR; put(DRY, t0, i, hp2.run(hp.run(rnd())) * Math.exp(-t * (open ? 11 : 70)) * .45 * amp, pan, open ? .1 : 0); }
}
function crash(t0, amp = 1) {
  const hp = new Biquad('hp', 4500); const len = 2.4 * SR;
  for (let i = 0; i < len; i++) { const t = i / SR; const v = hp.run(rnd()) * Math.exp(-t * 2.1) * .35 * amp; put(DRY, t0, i, v, (i % 2 ? .3 : -.3), .3); }
}
function impact(t0, amp = 1) {
  let ph = 0; const len = 2.6 * SR, lp = new Biquad('lp', 900);
  for (let i = 0; i < len; i++) {
    const t = i / SR, f = 28 + 70 * Math.exp(-t * 5); ph += TAU * f / SR;
    const v = Math.sin(ph) * Math.exp(-t * 1.6) * 1.0 + lp.run(rnd()) * Math.exp(-t * 4) * .5;
    put(DRY, t0, i, Math.tanh(v * 1.4) * .8 * amp, 0, .45);
  }
  crash(t0, amp * .9);
}
function riser(t0, t1, amp = 1) {
  const bp = new Biquad('bp', 300, 2.2), len = (t1 - t0) * SR; let ph = 0;
  for (let i = 0; i < len; i++) {
    const p = i / len, f = 250 * Math.pow(40, p * p);
    if (i % 32 === 0) bp.set(f, 2.2);
    ph += TAU * (110 * Math.pow(8, p * p)) / SR;
    const v = (bp.run(rnd()) * 1.2 + Math.sin(ph) * .12 * p) * Math.pow(p, 1.6);
    put(DRY, t0, i, v * .8 * amp, Math.sin(p * 9) * .4, .35);
  }
}
function whoosh(t0, len = .5, up = true, amp = 1) {
  const bp = new Biquad('bp', 500, 1.4), n = len * SR;
  for (let i = 0; i < n; i++) {
    const p = i / n, f = up ? 300 * Math.pow(20, p) : 6000 * Math.pow(1 / 20, p);
    if (i % 32 === 0) bp.set(f, 1.4);
    const env = Math.sin(Math.PI * Math.pow(p, up ? 1.4 : .7));
    put(DRY, t0, i, bp.run(rnd()) * env * .9 * amp, (p - .5) * (up ? 1.2 : -1.2), .25);
  }
}
function tick(t0, amp = 1, f = 2600) {
  const hp = new Biquad('hp', 2000); let ph = 0; const len = .05 * SR;
  for (let i = 0; i < len; i++) { const t = i / SR; ph += TAU * f / SR; put(DRY, t0, i, (Math.sin(ph) * .5 + hp.run(rnd()) * .6) * Math.exp(-t * 90) * amp * .6, .1, .15); }
}
function key(t0, amp = 1) { tick(t0, amp * .9, 3200); tick(t0 + .012, amp * .5, 1800); }

// ---------------------------------------------------------------- tonal
const CHORDS = [[57, 60, 64], [53, 57, 60], [55, 60, 64], [55, 59, 62]]; // Am F C G
const ROOTS = [45, 41, 48, 43];
const chordAt = t => Math.floor(t / BAR) % 4;
function saw(ph) { return 2 * (ph - Math.floor(ph + .5)); }
function supersaw(bus, t0, len, notes, amp, cutoff, pan = 0, send = .3, attack = .02, release = .3) {
  const n = Math.round(len * SR), lpL = new Biquad('lp', 800, .9), lpR = new Biquad('lp', 800, .9);
  const det = [-.19, -.08, 0, .08, .19], phases = notes.flatMap(() => det.map(() => (rnd() + 1) / 2));
  for (let i = 0; i < n; i++) {
    const t = i / SR;
    if (i % 64 === 0) { const c = cutoff(t0 + t); lpL.set(c, .9); lpR.set(c * 1.04, .9); }
    let l = 0, r = 0, k = 0;
    for (const m of notes) for (const d of det) {
      const f = mtof(m + d * .5); phases[k] += f / SR; const s = saw(phases[k]);
      if (k % 2) l += s; else r += s; k++;
    }
    const env = Math.min(1, t / attack) * (t > len - release ? Math.max(0, (len - t) / release) : 1);
    const vL = lpL.run(l) * env * amp / notes.length, vR = lpR.run(r) * env * amp / notes.length;
    const idx = Math.round(t0 * SR) + i; if (idx < 0 || idx >= N) continue;
    bus.L[idx] += vL * (1 - pan * .5); bus.R[idx] += vR * (1 + pan * .5);
    sendL[idx] += vL * send; sendR[idx] += vR * send;
  }
}
function bassNote(t0, len, midi, amp = 1) {
  const n = len * SR, lp = new Biquad('lp', 300, 1.2); let p1 = 0, p2 = 0, ps = 0;
  for (let i = 0; i < n; i++) {
    const t = i / SR, f = mtof(midi);
    if (i % 32 === 0) lp.set(180 + 1400 * Math.exp(-t * 18), 1.3);
    p1 += f / SR; p2 += f * 1.007 / SR; ps += TAU * f / 2 / SR;
    const env = Math.min(1, t / .004) * Math.min(1, (len - t) / .02);
    const v = lp.run(saw(p1) + saw(p2)) * .5 + Math.sin(ps) * .32;
    put(duckBus, t0, i, Math.tanh(v * 1.3) * env * .55 * amp);
  }
}
function pluck(t0, midi, amp = 1, pan = 0) {
  const n = .35 * SR, lp = new Biquad('lp', 3000, 1.5); let ph = 0;
  for (let i = 0; i < n; i++) {
    const t = i / SR; ph += mtof(midi) / SR;
    if (i % 32 === 0) lp.set(600 + 5000 * Math.exp(-t * 22), 1.5);
    const sq = (ph % 1) < .5 ? 1 : -1;
    put(duckBus, t0, i, lp.run(sq * .6 + saw(ph * 1.003) * .4) * Math.exp(-t * 11) * .22 * amp, pan, .25);
  }
}
function stab(t0, chord, amp = 1) {
  supersaw(DRY, t0, .55, [...chord, chord[0] + 12], .55 * amp, t => 600 + 7000 * Math.exp(-(t - t0) * 9), 0, .5, .003, .35);
}

// ---------------------------------------------------------------- arrangement
const groove = (a, b) => [a, b];
const GROOVE = [groove(4, 10), groove(10, 20), groove(22, 24)];
const inGroove = bar => GROOVE.some(([a, b]) => bar >= a && bar < b);
const kicks = [];

// Intro: heartbeat kicks, word ticks, pad opening.
for (const b of [0, 1]) { kick(T(b), .55); kicks.push(T(b)); kick(T(b, 2.5), .3); }
for (const at of [T(0), T(0, 1), T(1), T(1, 1), T(1, 2), T(1, 2.5)]) { tick(at, .8, 1900); whoosh(at - .18, .2, true, .25); }
for (let b = 0; b < 2; b++) supersaw(DRY, T(b), BAR + .05, CHORDS[b % 4], .5, t => 300 + 900 * (t / T(2)), 0, .5, .4, .1);
// Bar 2: a hit on every word.
for (let k = 0; k < 4; k++) { const at = T(2, k); kick(at, 1); clap(at, .7); stab(at, CHORDS[k % 4], .9); kicks.push(at); }
// Bar 3: riser + roll, cut on the last eighth.
riser(T(3), T(4) - .02, 1);
supersaw(DRY, T(3), BAR, CHORDS[3], .45, t => 400 + 4000 * Math.pow((t - T(3)) / BAR, 2), 0, .4, .05, .05);
for (let k = 0; k < 28; k++) {
  const p = k / 28, at = T(3) + (BAR - BEAT / 2) * (1 - Math.pow(1 - p, 1.6));
  if (at < T(3, 3.5)) snare(at, .25 + .6 * p, 180 + 200 * p);
}
// Drops.
impact(T(4), 1.1); impact(T(10), .7); impact(T(22), 1.25);
crash(T(14), .6); crash(T(17), .7); crash(T(18), .5);

for (let bar = 4; bar < 24; bar++) {
  if (!inGroove(bar)) continue;
  const c = chordAt(T(bar)), chord = CHORDS[c];
  for (let k = 0; k < 4; k++) { kick(T(bar, k), 1); kicks.push(T(bar, k)); }
  clap(T(bar, 1), .8); clap(T(bar, 3), .8);
  for (let k = 0; k < 8; k++) hat(T(bar, k / 2 + .25), k % 2 ? .9 : .5, bar >= 10 && k % 2 === 1 && k === 7, .25);
  for (let k = 0; k < 16; k++) if (k % 4 !== 0) hat(T(bar, k / 4), .25, false, -.3);
  for (let k = 0; k < 8; k++) bassNote(T(bar, k / 2 + .5 * (k % 2 ? 0 : 0)), BEAT / 2 * .85, ROOTS[c] + (k === 7 ? 12 : 0), k % 2 ? 1 : .85);
  const open = bar < 10 ? 900 + (bar - 4) * 350 : bar < 20 ? 3200 : 4200;
  supersaw(duckBus, T(bar), BAR + .02, chord, .42, t => open + 900 * Math.sin(t * .8), 0, .35, .01, .05);
  if (bar >= 7) {
    const tones = [...chord, chord[0] + 12, chord[1] + 12, chord[2] + 12];
    const pattern = [0, 2, 4, 3, 1, 5, 4, 2, 0, 3, 5, 4, 2, 1, 3, 5];
    for (let k = 0; k < 16; k++) pluck(T(bar, k / 4), tones[pattern[k]] + 12, bar >= 11 ? 1 : .7, (k % 2 ? .45 : -.45));
  }
}
// Fills.
for (let k = 0; k < 4; k++) snare(T(9, 3 + k / 4), .35 + .15 * k, 220 + 40 * k);
for (let k = 0; k < 4; k++) snare(T(19, 3 + k / 4), .35 + .15 * k, 220 + 40 * k);
// Theme stabs, feature-card hits.
for (let k = 0; k < 4; k++) stab(T(17, k), CHORDS[k], .9);
for (const at of [T(18), T(18, 2), T(19), T(19, 2)]) stab(at, CHORDS[chordAt(at)], .8);
// Break: words, then the final riser.
for (let b = 20; b < 22; b++) supersaw(DRY, T(b), BAR + .05, CHORDS[b % 4], .55, t => 700 + 500 * Math.sin(t), 0, .6, .25, .1);
for (const at of [T(20), T(20, 2), T(21)]) { kick(at, .9); stab(at, CHORDS[chordAt(at)], 1); whoosh(at - .25, .25, true, .4); }
riser(T(21, 1), T(22) - .02, 1.1);
for (let k = 0; k < 20; k++) { const p = k / 20, at = T(21, 1) + (BAR - BEAT * 1.5) * (1 - Math.pow(1 - p, 1.5)); if (at < T(21, 3.5)) snare(at, .3 + .6 * p, 200 + 250 * p); }
// End: final chord, long tail.
supersaw(DRY, T(24), DUR - T(24), [45, 57, 60, 64, 69], .7, t => 500 + 5000 * Math.exp(-(t - T(24)) * 1.4), 0, .6, .003, 1.5);
impact(T(24), .9);
// Interface sounds.
for (const at of [T(5), T(6), T(8, 2), T(14), T(16), T(16, 2)]) key(at, 1);
key(T(5) - .06, .6); key(T(5) - .03, .7); key(T(14) - .04, .6); key(T(16) - .04, .6); key(T(16, 2) - .04, .6);
for (const k of [2, 2.5, 3, 3.5]) tick(T(5, k), .45, 2200);
whoosh(T(4), .7, false, .7);                     // pull back from the bar icon
whoosh(T(7, 3.5) - .05, .45, true, .55);         // whip up to the toast
whoosh(T(8, 3) + .1, .45, false, .55);           // whip back down
whoosh(T(9, 3.6), .4, false, .4);                // out to the desktop
whoosh(T(14) - .05, .5, false, .6);              // peek drops in
whoosh(T(16) - .05, .45, true, .5);              // peek leaves
whoosh(T(16, 2), .5, true, .7);                  // workspace slide
for (let k = 0; k < 4; k++) whoosh(T(9, k) - .08, .22, k % 2 === 0, .3); // faces morph

// ---------------------------------------------------------------- sidechain, reverb, master
const sc = new Float32Array(N).fill(1);
for (const k of kicks) { const s = Math.round(k * SR); for (let i = 0; i < .3 * SR && s + i < N; i++) sc[s + i] = Math.min(sc[s + i], 1 - .75 * Math.exp(-i / SR / .09)); }
for (let i = 0; i < N; i++) { L[i] += duckBus.L[i] * sc[i]; R[i] += duckBus.R[i] * sc[i]; }

function reverb(inp, out, offset) {
  const combs = [1557, 1617, 1491, 1422, 1277, 1356].map(d => ({ b: new Float32Array(d + offset), i: 0, f: 0 }));
  const aps = [556, 441, 341].map(d => ({ b: new Float32Array(d + offset), i: 0 }));
  for (let n = 0; n < N; n++) {
    let acc = 0;
    for (const c of combs) { const y = c.b[c.i]; c.f = y * .78 + c.f * .22; c.b[c.i] = inp[n] * .5 + c.f * .86; c.i = (c.i + 1) % c.b.length; acc += y; }
    for (const a of aps) { const y = a.b[a.i]; const v = -acc + y; a.b[a.i] = acc + y * .5; a.i = (a.i + 1) % a.b.length; acc = v; }
    out[n] += acc * .14;
  }
}
reverb(sendL, L, 0); reverb(sendR, R, 23);

// Gentle fade with the picture, then limit and normalise.
const fadeA = T(24, 1);
// Drive the soft clipper moderately: scale so the loudest 0.1% of samples sit near 1.2.
const mags = []; for (let i = 0; i < N; i += 7) mags.push(Math.max(Math.abs(L[i]), Math.abs(R[i])));
mags.sort((a, b) => a - b); const p999 = mags[Math.floor(mags.length * .999)];
const pre = 1.2 / p999;
for (let i = 0; i < N; i++) { L[i] *= pre; R[i] *= pre; }
console.log(`pre-master p99.9 ${p999.toFixed(2)} → drive ${pre.toFixed(2)}`);
let peak = 0;
for (let i = 0; i < N; i++) {
  const t = i / SR, fade = t < fadeA ? 1 : Math.max(0, 1 - (t - fadeA) / (DUR - fadeA)) ** 1.5;
  const fin = Math.min(1, t / .05);
  L[i] = Math.tanh(L[i]) * fade * fin; R[i] = Math.tanh(R[i]) * fade * fin;
  peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i]));
}
const gain = .89 / peak;
const pcm = Buffer.alloc(44 + N * 4);
pcm.write('RIFF', 0); pcm.writeUInt32LE(36 + N * 4, 4); pcm.write('WAVE', 8); pcm.write('fmt ', 12);
pcm.writeUInt32LE(16, 16); pcm.writeUInt16LE(1, 20); pcm.writeUInt16LE(2, 22); pcm.writeUInt32LE(SR, 24);
pcm.writeUInt32LE(SR * 4, 28); pcm.writeUInt16LE(4, 32); pcm.writeUInt16LE(16, 34); pcm.write('data', 36); pcm.writeUInt32LE(N * 4, 40);
for (let i = 0; i < N; i++) {
  pcm.writeInt16LE(Math.round(Math.max(-1, Math.min(1, L[i] * gain)) * 32767), 44 + i * 4);
  pcm.writeInt16LE(Math.round(Math.max(-1, Math.min(1, R[i] * gain)) * 32767), 46 + i * 4);
}
const here = dirname(fileURLToPath(import.meta.url));
mkdirSync(join(here, 'build'), { recursive: true });
writeFileSync(join(here, 'build', 'music.wav'), pcm);
console.log(`build/music.wav · ${(N / SR).toFixed(2)}s · peak gain ${gain.toFixed(2)}`);
