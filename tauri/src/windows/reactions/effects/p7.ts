// @ts-nocheck
// 7. Fireworks: press and hold to charge a fuse, release to launch. The longer the hold, the bigger the show (1 to 10 rockets).
(() => {
  let S = null;
  const FULL = 3000,
    R = 19,
    CIRC = 2 * Math.PI * R;
  const later = (fn, ms) => {
    const st = S,
      id = setTimeout(() => {
        st.timers.delete(id);
        if (!st.dead) fn();
      }, ms);
    st.timers.add(id);
    return id;
  };
  const frames = (fn) => {
    const st = S;
    const loop = (t) => {
      if (st.dead) return;
      if (fn(t) !== false) requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  };
  const W = () => innerWidth,
    V = () => innerHeight;
  const PALETTES = [
    ["#ff4d6d", "#ff9eb5", "#ffffff"],
    ["#4cc9f0", "#a8f0ff", "#ffffff"],
    ["#ffd166", "#fff1b8", "#ff9f1c"],
    ["#b26bff", "#e6c3ff", "#ffffff"],
    ["#06d6a0", "#b9ffe6", "#ffffff"],
    ["#ff7a3d", "#ffd0a8", "#fff3c4"],
  ];
  const GOLD = ["#ffcf70", "#ffb347", "#ffe8b0", "#fff6dc"];

  // ---------- Audio ----------
  function pan(x, send = 0.35) {
    const a = H.ac(),
      p = a.createStereoPanner(),
      s = a.createGain();
    p.pan.value = H.clamp((x / W()) * 2 - 1, -1, 1) * 0.75;
    s.gain.value = send;
    p.connect(S.out);
    p.connect(s);
    s.connect(S.far);
    return p;
  }
  // The rising fuse hiss while you hold. Returns {set(k), stop()}.
  function fuseHiss(level = 1) {
    const a = H.ac(),
      t = a.currentTime;
    const src = a.createBufferSource();
    src.buffer = H.noiseBuffer();
    src.loop = true;
    const bp = a.createBiquadFilter();
    bp.type = "bandpass";
    bp.Q.value = 1.4;
    bp.frequency.value = 1800;
    const g = a.createGain();
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(0.03 * level, t + 0.08);
    const o = a.createOscillator(),
      og = a.createGain();
    o.type = "triangle";
    o.frequency.value = 180;
    og.gain.value = 0;
    src.connect(bp);
    bp.connect(g);
    g.connect(S.out);
    o.connect(og);
    og.connect(S.out);
    src.start(t, Math.random() * 2);
    o.start(t);
    const h = {
      set(k) {
        const n = a.currentTime;
        bp.frequency.setTargetAtTime(1800 + k * 5200, n, 0.05);
        g.gain.setTargetAtTime((0.03 + k * 0.13) * level, n, 0.05);
        o.frequency.setTargetAtTime(180 + k * 420, n, 0.05);
        og.gain.setTargetAtTime(k * 0.025 * level, n, 0.05);
      },
      stop() {
        if (h.done) return;
        h.done = true;
        const n = a.currentTime;
        g.gain.setTargetAtTime(0, n, 0.03);
        og.gain.setTargetAtTime(0, n, 0.03);
        src.stop(n + 0.2);
        o.stop(n + 0.2);
        S.hisses.delete(h);
      },
    };
    S.hisses.add(h);
    return h;
  }
  const fizz = (to, g = 0.06) =>
    H.noise({
      dur: H.rand(0.008, 0.02),
      gain: g * H.rand(0.4, 1),
      filter: "highpass",
      freq: H.rand(3500, 7000),
      attack: 0.001,
      to,
    });
  function launchSound(x, flight, whistle) {
    const a = H.ac(),
      t = a.currentTime,
      p = pan(x, 0.2);
    H.noise({ dur: 0.14, gain: 0.45, filter: "lowpass", freq: 500, q: 0.8, attack: 0.002, to: p });
    H.tone({ freq: 150, glideTo: 55, dur: 0.16, gain: 0.35, to: p });
    H.noise({
      when: 0.02,
      dur: flight,
      gain: 0.06,
      filter: "bandpass",
      freq: 2500,
      sweepTo: 6500,
      q: 1.2,
      attack: 0.03,
      to: p,
    });
    if (!whistle) return;
    const o = a.createOscillator(),
      g = a.createGain(),
      v = a.createOscillator(),
      vg = a.createGain(),
      f0 = H.rand(650, 900);
    o.frequency.setValueAtTime(f0, t + 0.04);
    o.frequency.exponentialRampToValueAtTime(f0 * H.rand(2.6, 3.2), t + flight);
    v.frequency.value = 9;
    vg.gain.value = 18;
    v.connect(vg);
    vg.connect(o.frequency);
    g.gain.setValueAtTime(0, t);
    g.gain.linearRampToValueAtTime(0.05, t + 0.12);
    g.gain.setValueAtTime(0.05, t + flight * 0.8);
    g.gain.linearRampToValueAtTime(0, t + flight);
    o.connect(g);
    g.connect(p);
    o.start(t);
    v.start(t);
    o.stop(t + flight + 0.05);
    v.stop(t + flight + 0.05);
  }
  function boomSound(x, size, crackles) {
    // Light travels faster than sound: the boom lands a beat after the flash.
    const d = H.rand(0.08, 0.2),
      p = pan(x, 0.6);
    H.noise({
      when: d,
      dur: 0.6 + size * 0.3,
      gain: 0.7 + size * 0.25,
      filter: "lowpass",
      freq: 220,
      q: 0.7,
      attack: 0.004,
      to: p,
    });
    H.tone({ when: d, freq: 78, glideTo: 30, dur: 0.7, gain: 0.55 * size, to: p });
    H.noise({ when: d, dur: 0.05, gain: 0.25, filter: "bandpass", freq: 1200, q: 0.8, to: p });
    H.noise({
      when: d + 0.05,
      dur: 1.8,
      gain: 0.18,
      filter: "lowpass",
      freq: 700,
      sweepTo: 140,
      attack: 0.02,
      to: S.far,
    });
    for (let i = 0; i < Math.min(crackles, 14); i++)
      H.noise({
        when: d + 0.25 + H.rand(0, 1.3),
        dur: 0.012,
        gain: H.rand(0.03, 0.12),
        filter: "highpass",
        freq: H.rand(3000, 6000),
        attack: 0.001,
        to: p,
      });
  }

  // ---------- Visuals ----------
  function darken(ms) {
    if (!S.dark) {
      S.dark = H.el(`<div class="p7-dark"></div>`);
      H.screen.appendChild(S.dark);
      requestAnimationFrame(() => S.dark && S.dark.classList.add("on"));
    } else S.dark.classList.add("on");
    clearTimeout(S.darkT);
    S.darkT = later(() => {
      S.dark.classList.remove("on");
      S.darkT = later(() => {
        S.dark.remove();
        S.dark = null;
      }, 1300);
    }, ms);
  }
  // A spherical shell seen from the front: particles bunch towards the rim, like the real thing.
  function sphere(x, y, n, v, o) {
    for (let i = 0; i < n; i++) {
      const u = Math.random(),
        s = v * Math.sqrt(1 - u * u) * H.rand(0.96, 1.04);
      H.emit({
        x,
        y,
        count: 1,
        angle: H.rand(0, 360),
        spread: 0,
        speed: [s, s],
        gravity: 0.05,
        drag: 0.95,
        life: [70, 110],
        size: [3, 5.5],
        shape: "circle",
        glow: true,
        spin: false,
        wobble: false,
        ...o,
      });
    }
  }
  const glowAt = (x, y, r, color) => {
    const g = H.add(
      `<div class="p7-glow" style="left:${x}px;top:${y}px;width:${r * 2}px;height:${r * 2}px;--c:${color}"></div>`,
      900,
    );
    g.animate(
      [
        { transform: "translate(-50%,-50%) scale(.3)", opacity: 1 },
        { transform: "translate(-50%,-50%) scale(1)", opacity: 0 },
      ],
      { duration: 850, easing: "cubic-bezier(.1,.7,.3,1)", fill: "forwards" },
    );
  };
  function burst(x, y, kind, big) {
    const pal = kind === "willow" ? GOLD : H.pick(PALETTES),
      v = 9.5 * big;
    glowAt(x, y, 160 * big, pal[0]);
    H.emit({
      x,
      y,
      count: 18,
      angle: 0,
      spread: 360,
      speed: [0.5, 3],
      gravity: 0,
      drag: 0.9,
      life: [8, 14],
      size: [8, 14],
      shape: "circle",
      colors: ["#fff"],
      glow: true,
    });
    if (kind === "peony") {
      sphere(x, y, Math.round(115 * big), v, { colors: pal });
      sphere(x, y, 32, v * 0.45, { colors: ["#fff"], size: [2, 3.5] });
    }
    if (kind === "ring") {
      const n = Math.round(70 * big),
        off = H.rand(0, 360);
      for (let i = 0; i < n; i++)
        H.emit({
          x,
          y,
          count: 1,
          angle: off + (i * 360) / n,
          spread: 2,
          speed: [v * 0.98, v],
          gravity: 0.04,
          drag: 0.95,
          life: [80, 110],
          size: [3.5, 5.5],
          shape: "circle",
          colors: pal.slice(0, 2),
          glow: true,
          spin: false,
          wobble: false,
        });
      sphere(x, y, 26, v * 0.35, { colors: [pal[2]], size: [2.5, 4] });
    }
    if (kind === "crackle") {
      sphere(x, y, Math.round(60 * big), v * 0.85, { colors: GOLD, life: [40, 60], size: [2.5, 4] });
      // Delayed pops scattered where the sparks ended up, each with its own tick.
      const pops = Math.round(34 * big),
        p = pan(x, 0.25);
      for (let i = 0; i < pops; i++) {
        const ms = H.rand(500, 1200),
          a = H.rand(0, Math.PI * 2),
          r = v * 18 * Math.sqrt(Math.random());
        later(() => {
          H.emit({
            x: x + Math.cos(a) * r,
            y: y + Math.sin(a) * r + ms / 40,
            count: 5,
            angle: 0,
            spread: 360,
            speed: [0.8, 2.6],
            gravity: 0.02,
            drag: 0.9,
            life: [6, 14],
            size: [2, 4],
            shape: "circle",
            colors: ["#fff", "#fff6c8"],
            glow: true,
          });
          if (i % 4 === 0) fizz(p, 0.14);
        }, ms);
      }
    }
    if (kind === "willow") {
      // Simulate the stars ourselves so they can leave long, drooping gold trails.
      const stars = [];
      for (let i = 0; i < Math.round(60 * big); i++) {
        const u = Math.random(),
          s = v * 0.8 * Math.sqrt(1 - u * u),
          a = H.rand(0, Math.PI * 2);
        stars.push({ x, y, vx: Math.cos(a) * s, vy: Math.sin(a) * s });
      }
      let f = 0;
      frames(() => {
        f++;
        for (const s of stars) {
          s.vx *= 0.955;
          s.vy = s.vy * 0.955 + 0.045;
          s.x += s.vx;
          s.y += s.vy;
          if (f % 2 === 0 && f < 150)
            H.emit({
              x: s.x,
              y: s.y,
              count: 1,
              angle: 90,
              spread: 30,
              speed: [0, 0.25],
              gravity: 0.012,
              drag: 1,
              life: f < 30 ? [40, 60] : [26, 44],
              size: [1.8, 3.2],
              shape: "circle",
              colors: GOLD,
              spin: false,
              wobble: false,
            });
          if (f % 6 === 0 && f < 110)
            H.emit({
              x: s.x,
              y: s.y,
              count: 1,
              angle: 0,
              spread: 0,
              speed: [0, 0],
              gravity: 0,
              life: [5, 7],
              size: [4, 5],
              shape: "circle",
              colors: ["#fff3c4"],
              glow: true,
            });
        }
        return f < 150;
      });
    }
  }
  function rocket(x0, kind, big, flight) {
    const xa = x0 + H.rand(-70, 70),
      ya = V() * H.rand(0.14, 0.4),
      y0 = V() + 10,
      t0 = performance.now(),
      wob = H.rand(0, 6);
    launchSound(x0, flight / 1000, Math.random() < 0.75);
    H.emit({
      x: x0,
      y: V() - 4,
      count: 14,
      angle: -90,
      spread: 70,
      speed: [1, 4],
      gravity: 0.15,
      life: [14, 26],
      size: [2, 4],
      shape: "circle",
      colors: ["#ffd9a0", "#fff"],
      spin: false,
    });
    frames((t) => {
      const p = Math.min(1, (t - t0) / flight),
        e = 1 - (1 - p) * (1 - p);
      const x = x0 + (xa - x0) * p + Math.sin(p * 9 + wob) * 3 * (1 - p),
        y = y0 + (ya - y0) * e;
      H.emit({
        x,
        y,
        count: 2,
        angle: 90,
        spread: 40,
        speed: [0.2, 1.4],
        gravity: 0.06,
        life: [14, 30],
        size: [1.6, 3.4],
        shape: "circle",
        colors: ["#ffcf8a", "#ff9f5a", "#fff3c4"],
        spin: false,
        wobble: false,
      });
      H.emit({
        x,
        y,
        count: 1,
        angle: 0,
        spread: 0,
        speed: [0, 0],
        gravity: 0,
        life: [2, 3],
        size: [6, 7],
        shape: "circle",
        colors: ["#fff"],
        glow: true,
      });
      if (p < 1) return true;
      burst(x, y, kind, big);
      boomSound(x, big, kind === "crackle" ? 4 : 14);
      return false;
    });
  }

  // ---------- Button ----------
  function mountButton(H) {
    const wrap = H.el(
      `<div class="p7-wrap"><svg class="p7-ring" viewBox="0 0 48 48"><circle class="t" cx="24" cy="24" r="${R}"/><circle class="f" cx="24" cy="24" r="${R}" stroke-dasharray="${CIRC}" stroke-dashoffset="${CIRC}"/></svg><div class="p7-count"></div></div>`,
    );
    const b = H.makeButton("🎆", "Hold to charge fireworks");
    wrap.prepend(b);
    const fuse = wrap.querySelector(".f"),
      count = wrap.querySelector(".p7-count");
    let c = null; // the current charge
    const start = () => {
      if (c) return;
      H.ac();
      c = { t0: performance.now(), hiss: fuseHiss(), full: false };
      wrap.classList.add("on");
      const me = c;
      frames((t) => {
        if (c !== me) return false;
        const held = t - me.t0,
          k = H.clamp(held / FULL, 0, 1),
          n = shots(held);
        me.hiss.set(k);
        fuse.style.strokeDashoffset = CIRC * (1 - k);
        count.textContent = n > 1 ? `×${n}` : "";
        count.classList.toggle("vis", n > 1);
        const ctr = H.centerOf(b),
          a = -Math.PI / 2 + k * Math.PI * 2,
          tx = ctr.x + Math.cos(a) * R * 0.85,
          ty = ctr.y + Math.sin(a) * R * 0.85;
        H.emit({
          x: tx,
          y: ty,
          count: 1 + Math.round(k * 3),
          angle: (a * 180) / Math.PI,
          spread: 200,
          speed: [1, 2.5 + k * 4],
          gravity: 0.18,
          drag: 0.97,
          life: [10, 22 + k * 12],
          size: [1.6, 3.4],
          shape: "circle",
          colors: ["#fff3c4", "#ffb347", "#ff7a3d"],
          spin: false,
          wobble: false,
          glow: Math.random() < 0.25,
        });
        if (Math.random() < 0.15 + k * 0.4) fizz(S.out, 0.03 + k * 0.05);
        wrap.style.translate = `${H.rand(-1, 1) * k * 2}px ${H.rand(-1, 1) * k * 2}px`;
        if (k >= 1 && !me.full) {
          me.full = true;
          wrap.classList.add("full");
          [1568, 2093, 2637].forEach((f, i) => H.tone({ when: i * 0.05, freq: f, dur: 0.5, gain: 0.05, to: S.out }));
        }
        return true;
      });
    };
    const release = () => {
      if (!c) return;
      const held = performance.now() - c.t0;
      c.hiss.stop();
      c = null;
      wrap.classList.remove("on", "full");
      wrap.style.translate = "";
      fuse.style.strokeDashoffset = CIRC;
      count.classList.remove("vis");
      wrap.animate([{ scale: 0.8 }, { scale: 1.15 }, { scale: 1 }], {
        duration: 380,
        easing: "cubic-bezier(.2,1.6,.4,1)",
      });
      P.play(H, H.me, shots(held));
    };
    b.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      try {
        b.setPointerCapture(e.pointerId);
      } catch {}
      start();
    });
    b.addEventListener("pointerup", release);
    b.addEventListener("pointercancel", release);
    b.addEventListener("lostpointercapture", release);
    b.addEventListener("keydown", (e) => {
      if ((e.key === " " || e.key === "Enter") && !e.repeat) {
        e.preventDefault();
        start();
      }
    });
    b.addEventListener("keyup", (e) => {
      if (e.key === " " || e.key === "Enter") {
        e.preventDefault();
        release();
      }
    });
    H.slot.appendChild(wrap);
    S.release = () => {
      if (c) {
        c.hiss.stop();
        c = null;
      }
    };
  }
  const shots = (held) => (held < 250 ? 1 : 1 + Math.round(H.clamp((held - 250) / (FULL - 250), 0, 1) * 9));

  const CSS = `
  .p7-wrap{position:relative;width:30px;height:30px;flex:none}
  .p7-wrap .h-react-btn{position:relative;z-index:1;background:radial-gradient(circle at 50% 40%,#3a2d6b,#140f2b);box-shadow:0 2px 10px rgba(255,160,80,.35);touch-action:none}
  .p7-wrap.on .h-react-btn{transform:scale(.9);box-shadow:0 0 18px rgba(255,170,80,.8);transition:transform .25s cubic-bezier(.2,1.6,.4,1)}
  .p7-ring{position:absolute;inset:-9px;width:48px;height:48px;transform:rotate(-90deg);pointer-events:none;overflow:visible}
  .p7-ring circle{fill:none;stroke-width:3.5}
  .p7-ring .t{stroke:rgba(255,255,255,0);transition:stroke .2s}
  .p7-wrap.on .p7-ring .t{stroke:rgba(255,255,255,.18)}
  .p7-ring .f{stroke:#ffb347;stroke-linecap:round;filter:drop-shadow(0 0 3px #ff9f1c)}
  .p7-wrap.full .p7-ring .f{stroke:#fff6dc;animation:p7-pulse .35s ease-in-out infinite alternate}
  @keyframes p7-pulse{to{filter:drop-shadow(0 0 8px #fff) drop-shadow(0 0 14px #ffb347)}}
  .p7-count{position:absolute;left:50%;bottom:calc(100% + 12px);transform:translate(-50%,6px) scale(.6);opacity:0;padding:3px 8px;border-radius:999px;
    background:rgba(15,10,28,.9);border:1px solid rgba(255,190,110,.5);font:800 12px/1 system-ui,sans-serif;color:#ffd9a0;white-space:nowrap;
    transition:transform .3s cubic-bezier(.2,1.6,.4,1),opacity .2s;pointer-events:none}
  .p7-count.vis{opacity:1;transform:translate(-50%,0) scale(1)}
  .p7-dark{position:fixed;inset:0;z-index:9;pointer-events:none;opacity:0;transition:opacity 1.2s ease;background:radial-gradient(130% 100% at 50% 100%,rgba(6,4,22,.4),rgba(2,2,12,.68))}
  .p7-dark.on{opacity:1;transition-duration:.5s}
  .p7-glow{position:fixed;border-radius:50%;mix-blend-mode:screen;background:radial-gradient(circle,color-mix(in srgb,var(--c) 45%,transparent),transparent 65%)}
  .p7-tilefuse{position:fixed;padding:5px 10px;border-radius:999px;background:rgba(15,10,28,.88);border:1px solid rgba(255,190,110,.5);font:700 12px/1 system-ui,sans-serif;color:#ffd9a0;white-space:nowrap}`;

  const P = {
    n: 7,
    title: "Fireworks (hold to charge)",
    pitch:
      "Press and hold to light the fuse, let go to launch. A tap sends one rocket; a full 3 s charge sends a 10-rocket finale across everyone's screen.",
    how: "Hold 🎆 (or Space on it) and watch the fuse ring fill, then release. Longer hold, bigger show.",
    mount(H) {
      S = { timers: new Set(), hisses: new Set(), dead: false, dark: null };
      H.css("p7", CSS);
      S.out = H.dest(0);
      S.far = H.dest(0.9);
      S.far.gain.value = 0.6;
      mountButton(H);
      const st = S;
      H.onLeave(() => {
        st.dead = true;
        st.timers.forEach(clearTimeout);
        st.release && st.release();
        st.hisses.forEach((h) => h.stop());
        if (st.dark) st.dark.remove();
        const t = H.now();
        [st.out, st.far].forEach((g) => {
          g.gain.setTargetAtTime(0, t, 0.05);
          setTimeout(() => g.disconnect(), 500);
        });
      });
    },
    play(H, who, n = 1) {
      n = H.clamp(Math.round(n) || 1, 1, 10);
      const finale = n >= 7,
        gap = n <= 3 ? 520 : 330;
      const times = Array.from({ length: n }, (_, i) =>
        finale && i >= n - 3 ? (n - 3) * gap + (i - (n - 3)) * 90 : i * gap,
      );
      const xs = Array.from({ length: n }, (_, i) => W() * (0.1 + (0.8 * (i + 0.5)) / n) + H.rand(-40, 40)).sort(
        () => Math.random() - 0.5,
      );
      const kinds = ["peony", "ring", "willow", "crackle"].sort(() => Math.random() - 0.5);
      const last = times[n - 1] + 1300;
      darken(last + 1800);
      H.banner(
        who,
        n === 1 ? "launched a firework"
        : finale ? `launched a ${n}-rocket finale`
        : `launched ${n} fireworks`,
        "🎆",
        Math.max(2600, last),
      );
      times.forEach((t, i) =>
        later(() => {
          const isFinal = finale && i >= n - 3,
            kind = n === 1 ? H.pick(["peony", "ring", "willow"]) : kinds[i % kinds.length];
          rocket(
            xs[i],
            isFinal && i === n - 1 ? "peony" : kind,
            (isFinal ? 1.35
            : n === 1 ? 1.2
            : 1) * H.rand(0.9, 1.1),
            H.rand(1000, 1300),
          );
          if (isFinal && i === n - 1)
            later(() => {
              H.shake(6, 600);
              H.flash("#fff1d6", 500, 0.18);
            }, 1250);
        }, t),
      );
    },
  };
  H.register(P);
})();
