// @ts-nocheck
// 6. Slow clap -> standing ovation: you start a slow clap, the others join in, it builds and erupts into a standing ovation.
(() => {
  let S = null;
  const PEAK = 8,
    FIZZLE_MS = 2000;
  const later = (fn, ms) => {
    const st = S,
      id = setTimeout(() => {
        st.timers.delete(id);
        if (!st.dead) fn();
      }, ms);
    st.timers.add(id);
    return id;
  };
  const track = (a) => (S.anims.push(a), a);
  // Overlays that sit under the particle canvas (and under the call window), so roses and confetti stay bright.
  const under = (html, ms) => {
    const n = H.el(html);
    H.screen.appendChild(n);
    S.nodes.add(n);
    if (ms) later(() => (n.remove(), S.nodes.delete(n)), ms);
    return n;
  };
  const SPRING = "cubic-bezier(.2,1.5,.4,1)";
  // Each person's hands sound a little different, and sit in a different place in the stereo field.
  const VOICES = [
    { f: 1650, q: 1.2, pan: 0 },
    { f: 1250, q: 1.0, pan: -0.55 },
    { f: 2250, q: 1.4, pan: 0.55 },
  ];
  const voiceOf = (key) =>
    VOICES[[...String(key)].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % VOICES.length];
  // The rest of the room, joining in a fraction late.
  const CROWD = [{ key: "crowd-b" }, { key: "crowd-c" }, { key: "crowd-a" }];

  // ---------- Audio ----------
  function room() {
    if (S.room) return S.room;
    const a = H.ac(),
      len = Math.floor(a.sampleRate * 0.45),
      ir = a.createBuffer(2, len, a.sampleRate);
    for (let c = 0; c < 2; c++) {
      const d = ir.getChannelData(c);
      for (let i = 0; i < len; i++) d[i] = (Math.random() * 2 - 1) * Math.pow(1 - i / len, 4) * (i < 300 ? i / 300 : 1);
    }
    const conv = a.createConvolver();
    conv.buffer = ir;
    const wet = a.createGain();
    wet.gain.value = 0.5;
    const out = H.dest(0.06); // the bus everything here goes through (so leaving can fade it)
    conv.connect(wet);
    wet.connect(out);
    S.room = { conv, out, bus: {} };
    return S.room;
  }
  function voiceBus(key) {
    const r = room();
    if (r.bus[key]) return r.bus[key];
    const a = H.ac(),
      g = a.createGain(),
      p = a.createStereoPanner(),
      send = a.createGain();
    p.pan.value = voiceOf(key).pan;
    send.gain.value = 0.7;
    g.connect(p);
    p.connect(r.out);
    p.connect(send);
    send.connect(r.conv);
    return (r.bus[key] = g);
  }
  // One crisp clap: a few micro-transients (palms never meet all at once), a bright body and a short tail.
  function clapSound(who, when = 0, level = 1, bright = 1) {
    const v = voiceOf(who.key),
      to = voiceBus(who.key),
      f = v.f * bright * H.rand(0.94, 1.06);
    for (let k = 0; k < 3; k++)
      H.noise({
        when: when + k * H.rand(0.006, 0.011),
        dur: 0.014,
        gain: 0.5 * level * (1 - k * 0.2),
        filter: "bandpass",
        freq: f * H.rand(0.9, 1.15),
        q: v.q,
        attack: 0.0008,
        to,
      });
    H.noise({
      when: when + 0.02,
      dur: 0.11,
      gain: 0.42 * level,
      filter: "bandpass",
      freq: f,
      q: v.q * 0.8,
      attack: 0.001,
      to,
    });
    H.noise({
      when: when + 0.02,
      dur: 0.05,
      gain: 0.18 * level,
      filter: "bandpass",
      freq: 850,
      q: 2,
      attack: 0.001,
      to,
    });
  }
  // A clap from somebody further away (the crowd in the ovation).
  const crowdClap = (when, level) =>
    H.noise({
      when,
      dur: H.rand(0.03, 0.08),
      gain: level,
      filter: "bandpass",
      freq: H.rand(1000, 2600),
      q: H.rand(0.8, 1.6),
      attack: 0.001,
      to: room().conv,
    });
  function roar(dur) {
    const a = H.ac(),
      t = a.currentTime,
      out = room().out;
    [
      [420, 0.7, 0.32],
      [900, 0.6, 0.26],
      [2200, 0.5, 0.12],
      [160, 0.9, 0.18],
    ].forEach(([f, q, lv], i) => {
      const src = a.createBufferSource();
      src.buffer = H.noiseBuffer();
      src.loop = true;
      const bp = a.createBiquadFilter();
      bp.type = "bandpass";
      bp.frequency.value = f;
      bp.Q.value = q;
      const g = a.createGain(),
        wob = a.createOscillator(),
        wg = a.createGain();
      wob.frequency.value = H.rand(0.3, 0.9);
      wg.gain.value = lv * 0.25;
      wob.connect(wg);
      wg.connect(g.gain);
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(lv, t + 0.35 + i * 0.1);
      g.gain.setValueAtTime(lv, t + dur * 0.55);
      g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
      src.connect(bp);
      bp.connect(g);
      g.connect(out);
      src.start(t, Math.random() * 2);
      src.stop(t + dur + 0.1);
      wob.start(t);
      wob.stop(t + dur + 0.1);
    });
    // "Whooo!" whistles from the crowd.
    for (let i = 0; i < 6; i++) {
      const w = H.rand(0.2, dur * 0.55),
        f = H.rand(1500, 2200);
      H.tone({
        when: w,
        freq: f,
        glideTo: f * H.rand(1.25, 1.5),
        dur: H.rand(0.35, 0.6),
        gain: 0.05,
        attack: 0.08,
        to: out,
      });
    }
    for (let i = 0; i < 4; i++) {
      const w = H.rand(0.3, dur * 0.5),
        f = H.rand(250, 340);
      H.tone({
        when: w,
        freq: f,
        glideTo: f * 0.75,
        type: "sawtooth",
        dur: 0.7,
        gain: 0.025,
        attack: 0.1,
        to: room().conv,
      });
    }
  }

  // ---------- Visuals ----------
  function hands(who, big = 1) {
    const t = H.tiles[who.key] || H.tiles.call,
      c = H.centerOf(t);
    const x = c.x + H.rand(-30, 30),
      y = c.y - 6;
    const el = H.add(
      `<div class="p6-hands" style="left:${x}px;top:${y}px;font-size:${38 * big}px"><span class="l">🫱</span><i class="p6-pow"></i><span class="r">🫲</span></div>`,
      1000,
    );
    const [l, pow, r] = el.children;
    l.animate(
      [
        { transform: "translateX(-22px) rotate(-22deg)" },
        { transform: "translateX(-3px) rotate(0)", offset: 0.3 },
        { transform: "translateX(-9px) rotate(-8deg)" },
      ],
      { duration: 520, easing: "cubic-bezier(.5,0,.3,1)", fill: "forwards" },
    );
    r.animate(
      [
        { transform: "translateX(22px) rotate(22deg)" },
        { transform: "translateX(3px) rotate(0)", offset: 0.3 },
        { transform: "translateX(9px) rotate(8deg)" },
      ],
      { duration: 520, easing: "cubic-bezier(.5,0,.3,1)", fill: "forwards" },
    );
    pow.animate(
      [
        { transform: "translate(-50%,-50%) scale(.2)", opacity: 0 },
        { transform: "translate(-50%,-50%) scale(.3)", opacity: 0, offset: 0.28 },
        { transform: "translate(-50%,-50%) scale(1.2)", opacity: 1, offset: 0.4 },
        { transform: "translate(-50%,-50%) scale(1.6)", opacity: 0 },
      ],
      { duration: 520, fill: "forwards" },
    );
    el.animate(
      [
        { transform: "translate(-50%,-50%) scale(.3)", opacity: 0 },
        { transform: "translate(-50%,-90%) scale(1.1)", opacity: 1, offset: 0.25 },
        { transform: "translate(-50%,-110%) scale(1)", opacity: 1, offset: 0.6 },
        { transform: "translate(-50%,-190%) scale(.85)", opacity: 0 },
      ],
      { duration: 950, easing: "cubic-bezier(.2,.9,.3,1)", fill: "forwards" },
    );
    track(
      t.animate([{ transform: "scale(1)" }, { transform: "scale(1.035)" }, { transform: "scale(1)" }], {
        duration: 220,
        easing: "ease-out",
      }),
    );
  }
  function setMeter(n, label) {
    const k = H.clamp(n / PEAK, 0, 1);
    S.fill.style.transform = `scaleX(${k})`;
    S.meter.classList.toggle("p6-live", n > 0);
    S.meter.style.setProperty("--heat", k);
    if (label != null) S.label.textContent = label;
    if (S.dim) S.dim.style.opacity = (k * k * 0.55).toFixed(3);
  }

  // ---------- The build ----------
  function step(who) {
    H.ac();
    const s = S.sess;
    if (s && s.erupted) {
      clapSound(who, 0, 0.8);
      hands(who);
      return;
    } // clapping along to the ovation
    if (!s) {
      S.sess = { n: 0, leader: who, joinAt: H.pick([2, 3]), last: 0, gap: 900, joiners: CROWD.slice(0, 2) };
      if (!S.dim) S.dim = under(`<div class="p6-dim"></div>`);
      H.banner(who, "started a slow clap", "👏");
    }
    const ss = S.sess,
      now = performance.now();
    if (ss.last) ss.gap = H.clamp(now - ss.last, 140, 1400);
    ss.last = now;
    ss.n++;
    const heat = ss.n / PEAK;
    clapSound(who, 0, 0.75 + heat * 0.35, 1 + heat * 0.06);
    hands(who, 1 + heat * 0.35);
    track(
      H.callWin.animate([{ scale: 1 }, { scale: 1 + 0.012 + heat * 0.02 }, { scale: 1 }], {
        duration: 200,
        easing: "ease-out",
      }),
    );
    // The others join in, a fraction late and not quite together. Once it gets going they double up.
    if (ss.n >= ss.joinAt)
      ss.joiners.forEach((p, i) => {
        const lag = H.rand(40, 110) + i * H.rand(30, 70);
        later(() => {
          clapSound(p, 0, 0.6 + heat * 0.3);
          hands(p, 0.9 + heat * 0.3);
        }, lag);
        if (ss.n >= 5 && ss.gap > 260) later(() => clapSound(p, 0, 0.45), lag + ss.gap / 2);
      });
    if (ss.n >= 4) for (let i = 0; i < (ss.n - 3) * 3; i++) crowdClap(H.rand(0, 0.3), 0.05 + heat * 0.08);
    const left = PEAK - ss.n;
    setMeter(
      ss.n,
      left <= 0 ? "BRAVO!"
      : ss.n < ss.joinAt ? "keep going…"
      : left <= 2 ? "almost there…"
      : "everyone's in",
    );
    clearTimeout(ss.fizzle);
    if (ss.n >= PEAK) {
      ss.erupted = true;
      later(() => P.play(H, ss.leader), 120);
    } else ss.fizzle = later(fizzle, FIZZLE_MS);
  }
  function fizzle() {
    const ss = S.sess;
    if (!ss || ss.erupted) return;
    // One last hopeful clap each, then it trails away.
    ss.joiners.forEach((p, i) => {
      if (ss.n >= ss.joinAt || i === 0)
        later(
          () => {
            clapSound(p, 0, 0.35);
            hands(p, 0.8);
          },
          150 + i * 520,
        );
    });
    S.meter.classList.add("p6-fizz");
    S.fill.style.transition = "transform 1.4s cubic-bezier(.45,0,.2,1)";
    setMeter(0, "…it fizzled");
    later(() => {
      S.meter.classList.remove("p6-fizz");
      S.fill.style.transition = "";
      S.label.textContent = "slow clap";
    }, 1800);
    endSession(1400);
  }
  function endSession(fadeMs) {
    const d = S.dim;
    S.dim = null;
    S.sess = null;
    if (d) {
      d.style.transition = `opacity ${fadeMs}ms ease`;
      d.style.opacity = 0;
      later(() => (d.remove(), S.nodes.delete(d)), fadeMs + 50);
    }
  }

  const CSS = `
  .p6-meter{position:absolute;left:50%;top:7px;width:140px;height:18px;transform:translateX(-50%);border-radius:9px;overflow:hidden;
    background:rgba(20,16,32,.55);box-shadow:inset 0 0 0 1px rgba(255,255,255,.08);opacity:.55;transition:opacity .3s,box-shadow .3s}
  .p6-meter.p6-live{opacity:1;box-shadow:inset 0 0 0 1px rgba(255,255,255,.15),0 0 calc(var(--heat,0)*22px) rgba(255,190,90,calc(var(--heat,0)*.9))}
  .p6-fill{position:absolute;inset:0;transform-origin:0 50%;transform:scaleX(0);transition:transform .45s cubic-bezier(.2,1.5,.4,1);
    background:linear-gradient(90deg,#ffd166,#ff9f43 50%,#ff4d6d)}
  .p6-meter.p6-fizz .p6-fill{filter:grayscale(1) brightness(.7)}
  .p6-label{position:absolute;inset:0;display:grid;place-items:center;font-size:9.5px;font-weight:800;letter-spacing:.06em;text-transform:uppercase;
    color:#fff;text-shadow:0 1px 2px rgba(0,0,0,.6);white-space:nowrap}
  .p6-hands{position:fixed;display:flex;align-items:center;line-height:1;filter:drop-shadow(0 6px 10px rgba(0,0,0,.35))}
  .p6-hands span{display:inline-block}
  .p6-pow{position:absolute;left:50%;top:45%;width:1.3em;height:1.3em;border-radius:50%;opacity:0;
    background:repeating-conic-gradient(from 0deg,#fff6c8 0 6deg,transparent 6deg 45deg);-webkit-mask:radial-gradient(circle,transparent 38%,#000 40%,#000 62%,transparent 64%);mask:radial-gradient(circle,transparent 38%,#000 40%,#000 62%,transparent 64%)}
  .p6-dim,.p6-dark,.p6-beam{pointer-events:none;z-index:9}
  .p6-dim{position:fixed;inset:0;opacity:0;transition:opacity .6s ease;background:radial-gradient(110% 90% at 50% 40%,rgba(10,6,24,.35),rgba(6,3,14,.85))}
  .p6-dark{position:fixed;inset:0;background:radial-gradient(120% 100% at 50% 30%,rgba(16,6,30,.55),rgba(4,2,10,.9))}
  .p6-beam{position:fixed;top:-6vh;width:46vw;height:140vh;margin-left:-23vw;transform-origin:50% 0;mix-blend-mode:screen;filter:blur(8px);
    clip-path:polygon(47% 0,53% 0,100% 100%,0 100%);background:linear-gradient(180deg,rgba(255,244,214,.75),rgba(255,226,160,.28) 55%,rgba(255,226,160,0))}
  .p6-bravo{position:fixed;left:50%;top:44%;transform:translate(-50%,-50%);display:flex;white-space:nowrap;
    font:italic 700 min(19vw,240px)/1 Didot,"Bodoni 72","Bodoni MT","Playfair Display",Georgia,"Times New Roman",serif;letter-spacing:.02em;
    color:#ffe6a3;text-shadow:0 2px 0 #b9802d,0 5px 0 #8a5a1a,0 0 40px rgba(255,200,90,.75),0 0 110px rgba(255,140,60,.5)}
  .p6-bravo span{display:inline-block}`;

  const P = {
    n: 6,
    title: "Slow clap → standing ovation",
    pitch:
      "Start a slow clap. The others join in, it speeds up, and at the peak the whole screen becomes a standing ovation: crowd roar, spotlights, roses, BRAVO.",
    how: "Click 👏 slowly, then faster. About 8 claps erupts into the ovation. Stop for 2 s and it fizzles out.",
    mount(H) {
      S = { timers: new Set(), anims: [], dead: false, sess: null, dim: null, nodes: new Set() };
      H.css("p6", CSS);
      const b = H.makeButton("👏", "Slow clap");
      b.addEventListener("pointerdown", (e) => {
        e.preventDefault();
        step(H.me);
      });
      b.addEventListener("keydown", (e) => {
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          if (!e.repeat) step(H.me);
        }
      });
      H.slot.appendChild(b);
      S.meter = H.el(`<div class="p6-meter"><div class="p6-fill"></div><div class="p6-label"></div></div>`);
      [S.fill, S.label] = S.meter.children;
      S.label.textContent = "slow clap";
      (H.meterHost || H.callWin).appendChild(S.meter);
      const st = S;
      H.onLeave(() => {
        st.dead = true;
        st.timers.forEach(clearTimeout);
        st.anims.forEach((a) => {
          try {
            a.cancel();
          } catch {}
        });
        st.meter.remove();
        st.nodes.forEach((n) => n.remove());
        if (st.room) {
          const t = H.now();
          st.room.out.gain.setTargetAtTime(0, t, 0.05);
          setTimeout(() => st.room.out.disconnect(), 500);
        }
      });
    },
    /** One clap from someone: enough of them, from anyone, and it erupts. */
    clap(H, who) {
      step(who);
    },
    play(H, who) {
      const T = 6.5,
        ms = T * 1000;
      if (!S.sess) S.sess = { n: PEAK, erupted: true, leader: who, joiners: [] };
      S.sess.erupted = true;
      H.banner(who, "got a standing ovation going", "🌹", 3600);
      setMeter(PEAK, "BRAVO!");
      // Sound: roar plus a few hundred claps, swelling then thinning out.
      roar(T);
      for (let i = 0; i < 420; i++) {
        const t = T * 0.85 * Math.pow(Math.random(), 1.4),
          e = Math.min(1, t / 0.4) * (1 - t / T);
        crowdClap(t, 0.06 + 0.22 * e * Math.random());
      }
      CROWD.forEach((p) => {
        for (let t = 0.05; t < T * 0.6; t += H.rand(0.2, 0.28)) clapSound(p, t, 0.5);
      });
      H.tone({ freq: 70, glideTo: 38, dur: 0.6, gain: 0.5, to: room().out });
      // Lights down, spotlights up.
      if (S.dim) {
        S.dim.style.transition = "opacity .2s";
        S.dim.style.opacity = 0;
      }
      const dark = under(`<div class="p6-dark"></div>`, ms + 100);
      dark.animate([{ opacity: 0 }, { opacity: 1, offset: 0.06 }, { opacity: 1, offset: 0.82 }, { opacity: 0 }], {
        duration: ms,
        fill: "forwards",
      });
      [0.12, 0.37, 0.63, 0.88].forEach((x, i) => {
        const b = under(`<div class="p6-beam" style="left:${x * 100}vw"></div>`, ms + 100),
          dir = i % 2 ? 1 : -1;
        b.animate([{ opacity: 0 }, { opacity: 1, offset: 0.08 }, { opacity: 1, offset: 0.8 }, { opacity: 0 }], {
          duration: ms,
          fill: "forwards",
        });
        b.animate([{ transform: `rotate(${dir * 32}deg)` }, { transform: `rotate(${-dir * 26}deg)` }], {
          duration: H.rand(1400, 1900),
          delay: i * 90,
          iterations: Infinity,
          direction: "alternate",
          easing: "cubic-bezier(.45,0,.55,1)",
        });
      });
      H.flash("#ffe2a8", 500, 0.5);
      H.shake(9, 600);
      // BRAVO, letter by letter.
      const bravo = H.add(
        `<div class="p6-bravo">${"BRAVO"
          .split("")
          .map((c) => `<span>${c}</span>`)
          .join("")}<span>!</span></div>`,
        ms,
      );
      [...bravo.children].forEach((s, i) =>
        s.animate(
          [
            { transform: `translateY(90px) scale(.2) rotate(${H.rand(-30, 30)}deg)`, opacity: 0 },
            { transform: "none", opacity: 1 },
          ],
          { duration: 700, delay: 180 + i * 85, easing: SPRING, fill: "backwards" },
        ),
      );
      bravo.animate(
        [
          { opacity: 1, transform: "translate(-50%,-50%) scale(1)" },
          { opacity: 1, transform: "translate(-50%,-50%) scale(1.06)", offset: 0.8 },
          { opacity: 0, transform: "translate(-50%,-62%) scale(1.1)" },
        ],
        { duration: ms - 200, easing: "ease-in-out", fill: "forwards" },
      );
      // Roses and confetti rain down; roses get thrown up from the front row too.
      for (let i = 0; i < 34; i++)
        later(
          () => {
            H.emit({
              x: H.rand(0, innerWidth),
              y: -40,
              count: 2,
              angle: 90,
              spread: 40,
              speed: [2, 6],
              gravity: 0.16,
              drag: 0.99,
              life: [200, 260],
              size: [30, 46],
              shape: "emoji",
              emoji: "🌹",
              wobble: false,
            });
            H.emit({
              x: H.rand(0, innerWidth),
              y: -20,
              count: 9,
              angle: 90,
              spread: 60,
              speed: [1, 5],
              gravity: 0.12,
              drag: 0.98,
              life: [200, 260],
              size: [7, 12],
              shape: H.pick(["rect", "tri"]),
            });
          },
          150 + i * 110,
        );
      [0, 500, 1100].forEach((d) =>
        later(
          () =>
            H.emit({
              x: innerWidth * H.rand(0.3, 0.7),
              y: innerHeight + 20,
              count: 6,
              angle: -90,
              spread: 50,
              speed: [15, 22],
              gravity: 0.32,
              life: [90, 130],
              size: [34, 46],
              shape: "emoji",
              emoji: "🌹",
            }),
          400 + d,
        ),
      );
      // Everyone on the call stands up and claps.
      Object.values(H.tiles).forEach((t, i) =>
        track(
          t.animate(
            [
              { transform: "translateY(0)" },
              { transform: "translateY(-14px) scale(1.03)" },
              { transform: "translateY(0)" },
            ],
            { duration: 520, delay: i * 120, iterations: 7, easing: "cubic-bezier(.3,1.4,.5,1)" },
          ),
        ),
      );
      for (let i = 0; i < 18; i++) later(() => hands(CROWD[i % 3], 1.2), i * 210);
      later(() => {
        S.fill.style.transition = "transform 1.2s cubic-bezier(.45,0,.2,1)";
        setMeter(0, "slow clap");
        endSession(300);
      }, ms - 600);
      later(() => (S.fill.style.transition = ""), ms + 800);
    },
  };
  H.register(P);
})();
