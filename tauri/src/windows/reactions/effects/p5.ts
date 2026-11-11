// @ts-nocheck
// 5. Soundboard drawer: an MPC-style pad drawer springs out from under the call window. 8 pads, each with its own sound and full-screen moment.
(() => {
  let S = null; // per-mount state
  const later = (fn, ms) => {
    const st = S,
      id = setTimeout(() => {
        st.timers.delete(id);
        if (!st.dead) fn();
      }, ms);
    st.timers.add(id);
    return id;
  };
  const out = (wet = 0.15) => S.buses[wet] || (S.buses[wet] = H.dest(wet));
  const anim = (el, frames, o) => el.animate(frames, { fill: "forwards", ...o });
  const SPRING = "cubic-bezier(.2,1.5,.4,1)";
  const W = () => innerWidth,
    V = () => innerHeight;
  // Looping noise, so long sounds never run off the end of the shared 3 s buffer.
  const noiseLoop = (o) => {
    const a = H.ac(),
      t = a.currentTime + (o.when || 0),
      dur = o.dur;
    const src = a.createBufferSource();
    src.buffer = H.noiseBuffer();
    src.loop = true;
    const f = a.createBiquadFilter();
    f.type = o.filter || "bandpass";
    f.frequency.setValueAtTime(o.freq ?? 1000, t);
    f.Q.value = o.q ?? 1;
    if (o.sweepTo) f.frequency.exponentialRampToValueAtTime(o.sweepTo, t + dur);
    const g = a.createGain();
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(o.gain, t + (o.attack ?? 0.01));
    if (o.hold) g.gain.setValueAtTime(o.gain, t + o.hold);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    src.connect(f);
    f.connect(g);
    g.connect(o.to || out(0.2));
    src.start(t, Math.random() * 2);
    src.stop(t + dur + 0.05);
  };
  const sat = (k) => {
    const n = 1024,
      c = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      const x = (i / (n - 1)) * 2 - 1;
      c[i] = Math.tanh(k * x) / Math.tanh(k);
    }
    return c;
  };

  // ---------- Shared visuals ----------
  function ring(x, y, color, ms) {
    const D = 2 * Math.hypot(Math.max(x, W() - x), Math.max(y, V() - y));
    const r = H.add(
      `<div class="p5-ring" style="left:${x - D / 2}px;top:${y - D / 2}px;width:${D}px;height:${D}px;--c:${color}"></div>`,
      ms + 60,
    );
    anim(
      r,
      [
        { transform: "scale(.02)", opacity: 1 },
        { transform: "scale(1)", opacity: 0 },
      ],
      { duration: ms, easing: "cubic-bezier(.1,.8,.3,1)" },
    );
  }
  function word(txt, x, y, o = {}) {
    const ms = o.ms || 1400,
      r = o.rot || 0;
    const w = H.add(
      `<div class="p5-word" style="left:${x}px;top:${y}px;font-size:${o.size || 90}px;color:${o.color || "#fff"}"><span>${txt}</span></div>`,
      ms + 80,
    );
    const T = (s, dy = 0, rr = r) => `translate(-50%,calc(-50% + ${dy}px)) rotate(${rr}deg) scale(${s})`;
    anim(
      w,
      [
        { transform: T(0), opacity: 0 },
        { transform: T(1.18), opacity: 1, offset: 0.14 },
        { transform: T(1), opacity: 1, offset: 0.26 },
        { transform: T(1), opacity: 1, offset: 0.75 },
        { transform: T(0.92, o.drop ?? 30, r + (o.droop || 0)), opacity: 0 },
      ],
      { duration: ms, easing: "cubic-bezier(.3,.7,.4,1)" },
    );
    if (o.wobble)
      w.firstChild.animate([{ transform: "rotate(-7deg) scaleY(1)" }, { transform: "rotate(7deg) scaleY(.9)" }], {
        duration: 180,
        iterations: Math.ceil(ms / 180),
        direction: "alternate",
        easing: "ease-in-out",
      });
    return w;
  }
  const kick = () =>
    H.callWin.animate([{ scale: 1 }, { scale: 1.05 }, { scale: 1 }], { duration: 260, easing: "ease-out" });
  const tileOf = (who) => H.centerOf(H.tiles[who.key] || H.tiles.call);

  // ---------- The eight pads ----------
  function airhorn(who) {
    const a = H.ac(),
      t0 = a.currentTime + 0.03;
    const sh = a.createWaveShaper();
    sh.curve = sat(2.2);
    const lp = a.createBiquadFilter();
    lp.type = "lowpass";
    lp.frequency.value = 3800;
    lp.Q.value = 1.2;
    const pk = a.createBiquadFilter();
    pk.type = "peaking";
    pk.frequency.value = 1250;
    pk.Q.value = 1.4;
    pk.gain.value = 7;
    sh.connect(lp);
    lp.connect(pk);
    pk.connect(out(0.22));
    const c = tileOf(who);
    [
      [0, 0.2],
      [0.27, 0.2],
      [0.54, 1.05],
    ].forEach(([w, d], bi) => {
      const t = t0 + w,
        g = a.createGain(),
        last = bi === 2;
      g.gain.setValueAtTime(0, t);
      g.gain.linearRampToValueAtTime(0.13, t + 0.015);
      g.gain.setValueAtTime(0.13, t + d);
      g.gain.linearRampToValueAtTime(0, t + d + 0.08);
      g.connect(sh);
      [
        [233, 0, 0.8],
        [466, -16, 0.6],
        [468, 11, 0.6],
        [698, 6, 0.35],
        [935, -9, 0.25],
      ].forEach(([f, det, lv]) => {
        const o = a.createOscillator(),
          og = a.createGain();
        o.type = "sawtooth";
        o.detune.value = det;
        o.frequency.setValueAtTime(f * 0.9, t);
        o.frequency.exponentialRampToValueAtTime(f, t + 0.06);
        if (last) {
          o.frequency.setValueAtTime(f, t + d - 0.15);
          o.frequency.exponentialRampToValueAtTime(f * 0.96, t + d + 0.08);
        }
        og.gain.value = lv;
        o.connect(og);
        og.connect(g);
        o.start(t);
        o.stop(t + d + 0.12);
      });
      later(
        () => {
          ring(c.x, c.y, last ? "rgba(255,209,102,.6)" : "rgba(255,110,140,.5)", last ? 1600 : 1000);
          if (last) {
            later(() => ring(c.x, c.y, "rgba(255,77,109,.5)", 1500), 170);
            later(() => ring(c.x, c.y, "rgba(255,255,255,.45)", 1500), 340);
          }
          H.shake(last ? 13 : 6, last ? 900 : 220);
          kick();
        },
        w * 1000 + 30,
      );
    });
  }

  function badum(who) {
    const tom = (w, f) => {
      H.tone({ when: w, freq: f, glideTo: f * 0.5, dur: 0.3, gain: 0.6, to: out(0.12) });
      H.noise({ when: w, dur: 0.04, gain: 0.25, filter: "lowpass", freq: 1800, to: out(0.1) });
    };
    tom(0, 200);
    tom(0.17, 140);
    noiseLoop({ when: 0.4, dur: 1.4, gain: 0.38, filter: "highpass", freq: 6500, q: 0.5, attack: 0.003, to: out(0.3) });
    H.noise({ when: 0.4, dur: 0.14, gain: 0.35, filter: "bandpass", freq: 2500, q: 0.9, to: out(0.2) });
    [3150, 4370, 5210, 6880].forEach((f) =>
      H.tone({ when: 0.4, freq: f, type: "square", dur: 0.9, gain: 0.014, to: out(0.3) }),
    );
    word("BA", W() * 0.3, V() * 0.46, { rot: -8, color: "#ffd166", size: 100, ms: 1300 });
    later(() => word("DUM", W() * 0.5, V() * 0.52, { rot: 4, color: "#ff7ab6", size: 110, ms: 1300 }), 170);
    later(() => {
      const x = W() * 0.72,
        y = V() * 0.42;
      word("TSS!", x, y, { rot: -6, color: "#7df9ff", size: 140, ms: 1500 });
      H.emit({
        x,
        y,
        count: 50,
        angle: 0,
        spread: 360,
        speed: [4, 13],
        gravity: 0.15,
        life: [40, 70],
        size: [8, 14],
        shape: "star",
        colors: ["#7df9ff", "#ffffff", "#ffd166"],
        glow: true,
      });
      H.flash("#e6fbff", 300, 0.35);
      H.shake(7, 320);
    }, 400);
  }

  function trombone(who) {
    const a = H.ac(),
      t0 = a.currentTime + 0.03;
    [
      [0, 293.7, 0.42],
      [0.48, 277.2, 0.42],
      [0.96, 261.6, 0.42],
      [1.44, 246.9, 1.5],
    ].forEach(([w, f, d], i) => {
      const t = t0 + w,
        last = i === 3,
        g = a.createGain(),
        lp = a.createBiquadFilter();
      lp.type = "lowpass";
      lp.Q.value = 7;
      lp.frequency.setValueAtTime(320, t);
      lp.frequency.exponentialRampToValueAtTime(1500, t + 0.14);
      lp.frequency.exponentialRampToValueAtTime(last ? 900 : 480, t + d);
      g.gain.setValueAtTime(0, t);
      g.gain.linearRampToValueAtTime(0.2, t + 0.05);
      g.gain.setValueAtTime(0.2, t + d - 0.12);
      g.gain.linearRampToValueAtTime(0, t + d);
      lp.connect(g);
      g.connect(out(0.25));
      [0, 9].forEach((det) => {
        const o = a.createOscillator();
        o.type = "sawtooth";
        o.detune.value = det;
        o.frequency.setValueAtTime(f * 1.03, t);
        o.frequency.exponentialRampToValueAtTime(f, t + 0.08);
        if (last) {
          o.frequency.setValueAtTime(f, t + 0.3);
          o.frequency.exponentialRampToValueAtTime(f * 0.93, t + d);
          const lfo = a.createOscillator(),
            lg = a.createGain(),
            lg2 = a.createGain();
          lfo.frequency.value = 5.5;
          lg.gain.value = 7;
          lg2.gain.value = 550;
          lfo.connect(lg);
          lg.connect(o.frequency);
          if (!det) {
            lfo.connect(lg2);
            lg2.connect(lp.frequency);
          }
          lfo.start(t + 0.25);
          lfo.stop(t + d);
        }
        o.connect(lp);
        o.start(t);
        o.stop(t + d + 0.05);
      });
    });
    const grey = H.add(`<div class="p5-grey"></div>`, 4300);
    anim(grey, [{ opacity: 0 }, { opacity: 1, offset: 0.2 }, { opacity: 1, offset: 0.8 }, { opacity: 0 }], {
      duration: 4200,
      easing: "ease-in-out",
    });
    const horn = H.add(`<div class="p5-horn">🎺</div>`, 4300);
    const pose = (r, x = 0, y = 0) => ({ transform: `translate(${x}px,${y}px) rotate(${r}deg)` });
    anim(horn, [pose(-30, -260, 60), pose(-12)], { duration: 450, easing: SPRING });
    [0, 480, 960].forEach((ms, i) =>
      later(() => anim(horn, [pose(-12 + i * 9), pose(-3 + i * 9)], { duration: 380, easing: SPRING }), ms + 40),
    );
    later(
      () =>
        horn.animate([pose(24, 0, 10), pose(30, 0, 18)], {
          duration: 180,
          iterations: 8,
          direction: "alternate",
          easing: "ease-in-out",
          fill: "forwards",
        }),
      1480,
    );
    later(
      () => anim(horn, [pose(30, 0, 18), pose(55, -40, V())], { duration: 700, easing: "cubic-bezier(.5,0,.8,.3)" }),
      2950,
    );
    const ws = [
      ["wah", 0.34, 0.3, 84],
      ["wah", 0.47, 0.38, 74],
      ["wah", 0.6, 0.47, 66],
      ["waaaah", 0.74, 0.6, 104],
    ];
    ws.forEach(([t, x, y, s], i) =>
      later(
        () =>
          word(t, W() * x, V() * y, {
            size: s,
            color: "#a9c1e6",
            rot: 4 + i * 5,
            droop: 18,
            drop: 70,
            ms: i === 3 ? 2200 : 1300,
            wobble: i === 3,
          }),
        [0, 480, 960, 1440][i] + 30,
      ),
    );
  }

  function crickets(who) {
    const chirp = (w, f) => {
      for (let k = 0; k < 3; k++)
        H.tone({ when: w + k * 0.045, freq: f, dur: 0.03, gain: 0.07, attack: 0.004, to: out(0.35) });
    };
    for (let t = 0.5; t < 3.7; t += H.rand(0.42, 0.8)) {
      chirp(t, 4400);
      if (Math.random() < 0.6) chirp(t + 0.19, 4750);
    }
    noiseLoop({ dur: 3.8, gain: 0.05, filter: "bandpass", freq: 450, sweepTo: 900, q: 0.6, attack: 1.2, to: out(0.3) });
    const dim = H.add(`<div class="p5-dim"></div>`, 4700);
    anim(dim, [{ opacity: 0 }, { opacity: 1, offset: 0.14 }, { opacity: 1, offset: 0.85 }, { opacity: 0 }], {
      duration: 4600,
      easing: "ease-in-out",
    });
    const bug = H.add(`<div class="p5-bug">🦗</div>`, 4600);
    anim(
      bug,
      [
        { opacity: 0, transform: "scale(.5)" },
        { opacity: 1, transform: "scale(1)", offset: 0.1 },
        { opacity: 1, offset: 0.85 },
        { opacity: 0, transform: "scale(1)" },
      ],
      { duration: 4500, easing: "ease-out" },
    );
    later(() => {
      let p = "",
        q = "";
      const pt = () => {
        const a = Math.random() * 6.283,
          r = Math.sqrt(Math.random()) * 42;
        return `${(50 + Math.cos(a) * r).toFixed(1)} ${(50 + Math.sin(a) * r).toFixed(1)}`;
      };
      for (let i = 0; i < 30; i++)
        i % 2 ?
          (p += `<path d="M${pt()} C${pt()} ${pt()} ${pt()}"/>`)
        : (q += `<path d="M${pt()} C${pt()} ${pt()} ${pt()}"/>`);
      const tw = H.add(
        `<div class="p5-tw" style="top:${V() * 0.74}px"><div><svg width="120" height="120" viewBox="0 0 100 100" fill="none" stroke-linecap="round"><g stroke="#a47a45" stroke-width="2.4">${p}</g><g stroke="#d9b27a" stroke-width="1.6">${q}</g></svg></div></div>`,
        3700,
      );
      anim(tw, [{ transform: "translateX(-150px)" }, { transform: `translateX(${W() + 150}px)` }], {
        duration: 3500,
        easing: "linear",
      });
      const up = "cubic-bezier(.25,.6,.45,1)",
        dn = "cubic-bezier(.55,0,.75,.4)",
        Y = (y) => `translateY(${y}px)`;
      const hops = [
        [0, 0],
        [0.12, -90],
        [0.26, 0],
        [0.38, -52],
        [0.5, 0],
        [0.61, -30],
        [0.72, 0],
        [0.82, -12],
        [0.9, 0],
        [1, 0],
      ];
      tw.firstChild.animate(
        hops.map(([o, y], i) => ({ transform: Y(y), offset: o, easing: i % 2 ? dn : up })),
        { duration: 3500 },
      );
      tw.querySelector("svg").animate([{ transform: "rotate(0)" }, { transform: "rotate(1000deg)" }], {
        duration: 3500,
      });
    }, 450);
  }

  function drumroll(who) {
    const snare = (w, g) => {
      H.noise({ when: w, dur: 0.06, gain: g, filter: "bandpass", freq: H.rand(2600, 3600), q: 0.8, to: out(0.15) });
      H.tone({ when: w, freq: 185, glideTo: 160, dur: 0.05, type: "triangle", gain: g * 0.5, to: out(0.1) });
    };
    for (let t = 0; t < 2.15; ) {
      const k = t / 2.15;
      snare(t, 0.05 + 0.3 * k * k);
      t += 0.075 - 0.043 * k;
    }
    H.tone({ when: 2.2, freq: 120, glideTo: 40, dur: 0.55, gain: 0.9, to: out(0.2) });
    noiseLoop({ when: 2.2, dur: 2.3, gain: 0.45, filter: "highpass", freq: 5000, q: 0.5, attack: 0.003, to: out(0.4) });
    H.noise({ when: 2.2, dur: 0.3, gain: 0.4, filter: "bandpass", freq: 2200, q: 0.7, to: out(0.3) });
    const c = tileOf(who);
    const spot = H.add(`<div class="p5-spot" style="left:${c.x}px;top:${c.y}px"></div>`, 2500);
    const T = (s) => `translate(-50%,-50%) scale(${s})`;
    anim(
      spot,
      [
        { transform: T(3.5), opacity: 0 },
        { transform: T(1.35), opacity: 1, offset: 0.25 },
        { transform: T(0.95), opacity: 1 },
      ],
      { duration: 2200, easing: "cubic-bezier(.3,.6,.4,1)" },
    );
    later(() => {
      anim(spot, [{ opacity: 1 }, { opacity: 0 }], { duration: 160 });
      H.flash("#fff", 550, 0.85);
      H.shake(14, 700);
      kick();
      H.emit({
        x: c.x,
        y: c.y,
        count: 170,
        angle: 0,
        spread: 360,
        speed: [8, 22],
        gravity: 0.3,
        life: [90, 150],
        size: [7, 13],
      });
      H.emit({
        x: c.x,
        y: c.y,
        count: 40,
        angle: 0,
        spread: 360,
        speed: [5, 16],
        gravity: 0.12,
        life: [40, 70],
        size: [10, 16],
        shape: "star",
        colors: ["#fff", "#ffd166"],
        glow: true,
      });
      word("TA-DA!", W() * 0.5, V() * 0.42, { size: 150, color: "#ffd166", rot: -4, ms: 1600 });
    }, 2200);
  }

  function chaching(who) {
    H.noise({ dur: 0.05, gain: 0.4, filter: "lowpass", freq: 2500, to: out(0.1) });
    H.tone({ freq: 620, type: "square", dur: 0.04, gain: 0.08, to: out(0.1) });
    H.noise({ when: 0.05, dur: 0.18, gain: 0.25, filter: "bandpass", freq: 900, q: 2, sweepTo: 400, to: out(0.1) });
    const bell = (w, f, g) =>
      [
        [1, 1],
        [2.76, 0.5],
        [5.4, 0.25],
        [8.93, 0.12],
      ].forEach(([m, l]) =>
        H.tone({ when: w, freq: f * m, dur: 1.7 / Math.pow(m, 0.3), gain: g * l, attack: 0.002, to: out(0.3) }),
      );
    bell(0.14, 1568, 0.22);
    bell(0.14, 2093, 0.16);
    for (let i = 0; i < 45; i++)
      H.tone({
        when: 0.2 + Math.random() * 1.5,
        freq: H.rand(3200, 7200),
        dur: H.rand(0.06, 0.18),
        gain: H.rand(0.015, 0.05),
        to: out(0.4),
      });
    const c = tileOf(who);
    H.emit({
      x: c.x,
      y: c.y,
      count: 34,
      angle: -90,
      spread: 70,
      speed: [10, 19],
      gravity: 0.45,
      drag: 0.99,
      life: [80, 130],
      size: [24, 34],
      shape: "emoji",
      emoji: "🪙",
      wobble: false,
    });
    later(() => {
      H.flash("#ffd166", 600, 0.28);
      word("CHA-CHING!", W() * 0.5, V() * 0.4, { size: 120, color: "#ffd166", rot: -5, ms: 1500 });
    }, 140);
    for (let i = 0; i < 20; i++)
      later(
        () => {
          for (let j = 0; j < 6; j++)
            H.emit({
              x: H.rand(0, W()),
              y: -40,
              count: 1,
              angle: 90,
              spread: 20,
              speed: [3, 8],
              gravity: 0.32,
              life: [150, 220],
              size: [22, 38],
              shape: "emoji",
              emoji: Math.random() < 0.85 ? "🪙" : "💰",
              wobble: false,
            });
          H.emit({
            x: H.rand(0, W()),
            y: H.rand(0, V() * 0.7),
            count: 3,
            angle: 0,
            spread: 360,
            speed: [0.5, 2],
            gravity: 0,
            life: [20, 35],
            size: [8, 14],
            shape: "star",
            colors: ["#fff6c8", "#ffd166"],
            glow: true,
          });
        },
        100 + i * 80,
      );
  }

  function wooow(who) {
    const a = H.ac(),
      t0 = a.currentTime + 0.02,
      o = out(0.4),
      T = 2.3;
    for (let i = 0; i < 9; i++) {
      const f = H.pick([110, 130, 150, 180, 210, 240, 260]) * H.rand(0.97, 1.03),
        t = t0 + H.rand(0, 0.18);
      const osc = a.createOscillator();
      osc.type = "sawtooth";
      osc.frequency.setValueAtTime(f * 0.8, t);
      osc.frequency.exponentialRampToValueAtTime(f * 1.3, t + 0.75);
      osc.frequency.exponentialRampToValueAtTime(f * 0.78, t + T);
      const vib = a.createOscillator(),
        vg = a.createGain();
      vib.frequency.value = H.rand(4.5, 6);
      vg.gain.value = f * 0.015;
      vib.connect(vg);
      vg.connect(osc.frequency);
      const g = a.createGain();
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(0.12, t + 0.35);
      g.gain.setValueAtTime(0.12, t + 1.2);
      g.gain.exponentialRampToValueAtTime(0.0001, t + T);
      const f1 = a.createBiquadFilter(),
        f2 = a.createBiquadFilter(),
        g2 = a.createGain();
      g2.gain.value = 0.6;
      f1.type = f2.type = "bandpass";
      f1.Q.value = 5;
      f2.Q.value = 6;
      f1.frequency.setValueAtTime(320, t);
      f1.frequency.exponentialRampToValueAtTime(650, t + 0.75);
      f1.frequency.exponentialRampToValueAtTime(380, t + T);
      f2.frequency.setValueAtTime(800, t);
      f2.frequency.exponentialRampToValueAtTime(1080, t + 0.75);
      f2.frequency.exponentialRampToValueAtTime(780, t + T);
      osc.connect(f1);
      f1.connect(g);
      osc.connect(f2);
      f2.connect(g2);
      g2.connect(g);
      g.connect(o);
      osc.start(t);
      osc.stop(t + T + 0.05);
      vib.start(t);
      vib.stop(t + T + 0.05);
    }
    const box = H.add(
      `<div class="p5-wow">${"WOOOOW"
        .split("")
        .map((ch, i) => `<span style="color:hsl(${190 - i * 34},100%,${68 + (i % 2) * 6}%)">${ch}</span>`)
        .join("")}</div>`,
      2500,
    );
    anim(
      box,
      [
        { transform: "translate(-50%,-50%) scale(.2)", opacity: 0 },
        { transform: "translate(-50%,-50%) scale(1)", opacity: 1, offset: 0.2 },
        { transform: "translate(-50%,-50%) scale(1.06)", opacity: 1, offset: 0.8 },
        { transform: "translate(-50%,-50%) scale(1.25)", opacity: 0 },
      ],
      { duration: 2400, easing: "cubic-bezier(.25,1.3,.4,1)" },
    );
    [...box.children].forEach((s, i) =>
      s.animate([{ transform: "scale(1)" }, { transform: "scale(1.45) translateY(-6%)" }, { transform: "scale(1)" }], {
        duration: 520,
        delay: 250 + i * 90,
        iterations: 2,
        easing: "ease-in-out",
      }),
    );
    H.screen.animate([{ scale: 1 }, { scale: 1.035, offset: 0.4 }, { scale: 1 }], {
      duration: 2300,
      easing: "ease-in-out",
    });
    H.emit({
      x: W() / 2,
      y: V() / 2,
      count: 60,
      angle: 0,
      spread: 360,
      speed: [4, 14],
      gravity: 0.05,
      life: [50, 90],
      size: [10, 16],
      shape: "star",
      colors: ["#fff", "#ffd166", "#7df9ff", "#ff7ab6"],
      glow: true,
    });
    Object.values(H.tiles).forEach((t, i) =>
      later(
        () => {
          const c = H.centerOf(t),
            e = H.add(`<div class="p5-oface" style="left:${c.x}px;top:${c.y}px">😮</div>`, 1900);
          anim(
            e,
            [
              { transform: "translate(-50%,-50%) scale(0)", opacity: 1 },
              { transform: "translate(-50%,-90%) scale(1.15)", opacity: 1, offset: 0.25 },
              { transform: "translate(-50%,-120%) scale(1)", opacity: 1, offset: 0.75 },
              { transform: "translate(-50%,-160%) scale(.9)", opacity: 0 },
            ],
            { duration: 1800, easing: SPRING },
          );
        },
        80 + i * 110,
      ),
    );
  }

  function applause(who) {
    const T = 3.2;
    noiseLoop({ dur: T, gain: 0.07, filter: "bandpass", freq: 1500, q: 0.4, attack: 1.2, hold: 1.8, to: out(0.3) });
    for (let i = 0; i < 200; i++) {
      const t = (T * (Math.random() + Math.random() + Math.random())) / 3,
        env = Math.sin((Math.PI * t) / T);
      H.noise({
        when: t,
        dur: H.rand(0.03, 0.07),
        gain: 0.04 + 0.2 * env * Math.random(),
        filter: "bandpass",
        freq: H.rand(900, 2600),
        q: H.rand(0.8, 1.6),
        to: out(0.25),
      });
    }
    for (let i = 0; i < 30; i++)
      later(() => {
        const env = Math.sin((Math.PI * i) / 30),
          n = 1 + Math.round(env * 4);
        for (let j = 0; j < n; j++)
          H.emit({
            x: H.rand(30, W() - 30),
            y: V() + 20,
            count: 1,
            angle: -90,
            spread: 24,
            speed: [10, 16],
            gravity: 0.25,
            life: [70, 110],
            size: [26, 42],
            shape: "emoji",
            emoji: "👏",
            wobble: false,
          });
      }, i * 95);
  }

  const PADS = [
    { e: "📯", label: "Airhorn", c: "#ff4d6d", verb: "blew the airhorn", run: airhorn },
    { e: "🥁", label: "Ba-dum-tss", c: "#ffb347", verb: "played a ba-dum-tss", run: badum },
    { e: "🎺", label: "Sad trom.", c: "#6c9bd2", verb: "played the sad trombone", run: trombone },
    { e: "🦗", label: "Crickets", c: "#5fbf6a", verb: "summoned the crickets", run: crickets },
    { e: "🥁", label: "Drumroll", c: "#c77dff", verb: "started a drumroll", run: drumroll },
    { e: "💰", label: "Cha-ching", c: "#ffd166", verb: "hit cha-ching", run: chaching },
    { e: "😮", label: "Wooow", c: "#4cc9f0", verb: "went wooow", run: wooow },
    { e: "👏", label: "Applause", c: "#ff7ab6", verb: "started the applause", run: applause },
  ];

  const CSS = `
  .p5-clip{position:absolute;left:0;right:0;top:100%;height:210px;overflow:hidden;pointer-events:none}
  .p5-drawer{position:relative;margin:0 12px;padding:8px 10px 12px;border-radius:0 0 14px 14px;pointer-events:auto;
    background:linear-gradient(180deg,#30323d,#1c1d25);transform:translateY(-104%);transition:transform .3s cubic-bezier(.55,0,.8,.2);
    box-shadow:inset 0 12px 14px -8px rgba(0,0,0,.85),inset 0 0 0 1px rgba(255,255,255,.05),0 16px 30px rgba(0,0,0,.45)}
  .p5-drawer::before{content:"";position:absolute;left:0;right:0;bottom:100%;height:40px;background:#30323d}
  .p5-open .p5-drawer{transform:translateY(0);transition:transform .62s cubic-bezier(.18,1.45,.38,1)}
  .p5-head{display:flex;align-items:center;gap:6px;font:800 8.5px/1 ui-monospace,Menlo,monospace;letter-spacing:.14em;color:rgba(255,255,255,.45);margin:2px 2px 8px}
  .p5-head i{width:6px;height:6px;border-radius:50%;background:#ff4d6d;box-shadow:0 0 6px #ff4d6d}
  .p5-head span{margin-left:auto;letter-spacing:.05em}
  .p5-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:7px}
  .p5-pad{all:unset;cursor:pointer;position:relative;height:54px;border-radius:7px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:3px;
    background:radial-gradient(120% 90% at 50% 30%,color-mix(in srgb,var(--c) 28%,#2b2d38),#17181e);
    box-shadow:inset 0 1px 0 rgba(255,255,255,.09),inset 0 -2px 0 rgba(0,0,0,.45),0 0 0 1px rgba(0,0,0,.55),0 0 10px -3px var(--c);
    transition:transform .3s cubic-bezier(.2,1.8,.4,1),filter .2s}
  .p5-pad:hover{filter:brightness(1.25)}
  .p5-pad::after{content:"";position:absolute;inset:0;border-radius:inherit;opacity:0;background:radial-gradient(circle at 50% 45%,#fff,var(--c) 45%,transparent 85%)}
  .p5-pad .e{font-size:19px;line-height:1;position:relative;z-index:1}
  .p5-pad .l{font-size:8px;font-weight:800;letter-spacing:.05em;text-transform:uppercase;opacity:.8;position:relative;z-index:1}
  .p5-pad .k{position:absolute;top:3px;left:5px;font:800 8px/1 ui-monospace,Menlo,monospace;opacity:.45;z-index:1}
  .p5-pad.hit{animation:p5-press .45s cubic-bezier(.2,1.4,.4,1)}
  .p5-pad.hit::after{animation:p5-glow .6s ease-out}
  @keyframes p5-press{0%{transform:scale(.88);box-shadow:0 0 0 1px #fff9,0 0 28px 6px var(--c)}100%{transform:scale(1)}}
  @keyframes p5-glow{0%{opacity:.95}100%{opacity:0}}
  .p5-by{position:absolute;top:3px;right:4px;z-index:2;font-size:7px;font-weight:800;padding:1px 3px;border-radius:4px;background:linear-gradient(135deg,#7c5cff,#3ec6ff)}
  .h-react-btn.p5-on{box-shadow:0 0 0 2px #fff,0 0 16px rgba(255,122,182,.9)}
  .p5-ring{position:fixed;border-radius:50%;background:radial-gradient(circle closest-side,transparent 80%,var(--c) 92%,transparent 100%)}
  .p5-word{position:fixed;font:italic 900 90px/1 Impact,"Arial Black",system-ui,sans-serif;white-space:nowrap;letter-spacing:.02em;
    -webkit-text-stroke:3px rgba(20,10,40,.85);paint-order:stroke fill;text-shadow:0 7px 0 rgba(20,10,40,.45),0 0 40px rgba(255,255,255,.3)}
  .p5-word span{display:inline-block}
  .p5-grey{position:fixed;inset:0;backdrop-filter:grayscale(.9) brightness(.8);-webkit-backdrop-filter:grayscale(.9) brightness(.8);background:rgba(40,60,95,.22)}
  .p5-horn{position:fixed;left:6vw;bottom:8vh;font-size:min(22vw,200px);line-height:1;transform-origin:30% 60%;filter:drop-shadow(0 20px 30px rgba(0,0,0,.4))}
  .p5-dim{position:fixed;inset:0;background:radial-gradient(120% 100% at 50% 60%,rgba(10,12,25,.5),rgba(4,5,12,.78))}
  .p5-bug{position:fixed;right:8vw;bottom:7vh;font-size:44px;animation:p5-chirp .5s steps(2) infinite}
  @keyframes p5-chirp{50%{translate:0 -2px;rotate:3deg}}
  .p5-tw{position:fixed;left:0;margin-top:-60px}
  .p5-tw svg{display:block;filter:drop-shadow(0 18px 10px rgba(0,0,0,.35))}
  .p5-spot{position:fixed;width:300px;height:300px;border-radius:50%;background:radial-gradient(circle,rgba(255,240,200,.14),transparent 70%);
    box-shadow:0 0 0 300vmax rgba(4,4,12,.76),inset 0 0 50px 28px rgba(4,4,12,.76)}
  .p5-wow{position:fixed;left:50%;top:45%;display:flex;font:900 min(16vw,190px)/1 "Arial Rounded MT Bold",system-ui,sans-serif;white-space:nowrap;
    -webkit-text-stroke:4px rgba(20,10,40,.8);paint-order:stroke fill;filter:drop-shadow(0 10px 30px rgba(76,201,240,.5))}
  .p5-wow span{display:inline-block}
  .p5-oface{position:fixed;font-size:54px;line-height:1}`;

  const P = {
    n: 5,
    title: "Soundboard drawer",
    pitch:
      "A drawer of glowing MPC pads springs out of the call window. Eight sounds, each with its own full-screen moment for everyone on the call.",
    how: "Click 🎛️ to open the drawer, then hit pads or press 1–8 (keys work even when it's closed). Esc closes it.",
    capturesDigits: true,
    mount(H) {
      S = { timers: new Set(), buses: {}, dead: false, open: false, shift: 0 };
      H.css("p5", CSS);
      const b = H.makeButton("🎛️", "Soundboard (1–8)");
      const wrap = H.el(
        `<div class="p5-clip"><div class="p5-drawer"><div class="p5-head"><i></i>HOPP·8 SOUNDBOARD<span>1–8</span></div><div class="p5-grid">${PADS.map((p, i) => `<button class="p5-pad" style="--c:${p.c}" title="${p.label}"><span class="k">${i + 1}</span><span class="e">${p.e}</span><span class="l">${p.label}</span></button>`).join("")}</div></div></div>`,
      );
      H.callWin.appendChild(wrap);
      Object.assign(S, { btn: b, wrap, drawer: wrap.firstChild, pads: [...wrap.querySelectorAll(".p5-pad")] });
      S.pads.forEach((el, i) =>
        el.addEventListener("pointerdown", (e) => {
          e.preventDefault();
          hit(H, i, H.me);
        }),
      );
      b.onclick = () => setOpen(H, !S.open);
      H.slot.appendChild(b);
      const onKey = (e) => {
        if (e.metaKey || e.ctrlKey || e.altKey || e.repeat || (e.target.closest && e.target.closest("input,textarea")))
          return;
        if (e.key === "Escape") return setOpen(H, false);
        const i = +e.key;
        if (i >= 1 && i <= 8) {
          e.preventDefault();
          if (!S.open) setOpen(H, true);
          hit(H, i - 1, H.me);
        }
      };
      addEventListener("keydown", onKey);
      const st = S;
      H.onLeave(() => {
        st.dead = true;
        st.timers.forEach(clearTimeout);
        removeEventListener("keydown", onKey);
        wrap.remove();
        const t = H.now();
        Object.values(st.buses).forEach((g) => {
          g.gain.setTargetAtTime(0, t, 0.04);
          setTimeout(() => g.disconnect(), 400);
        });
      });
    },
    play(H, who, i = 0) {
      const p = PADS[i] || PADS[0];
      H.banner(who, p.verb, p.e);
      p.run(who);
    },
  };

  function setOpen(H, v) {
    if (v === S.open) return;
    S.open = v;
    S.wrap.classList.toggle("p5-open", v);
    S.btn.classList.toggle("p5-on", v);
    const cw = H.callWin;
    cw.style.transition = "translate .6s cubic-bezier(.2,1.25,.4,1)";
    if (v) {
      const r = cw.getBoundingClientRect(),
        top = r.top + S.shift,
        bottom = r.bottom + S.shift;
      S.shift = H.clamp(bottom + S.drawer.offsetHeight + 16 - innerHeight, 0, Math.max(0, top - 32));
      S.pads.forEach((el, i) =>
        el.animate(
          [
            { transform: "translateY(-12px) scale(.8)", opacity: 0 },
            { transform: "none", opacity: 1 },
          ],
          { duration: 420, delay: 90 + i * 28, easing: SPRING, fill: "backwards" },
        ),
      );
    } else S.shift = 0;
    cw.style.translate = `0 ${-S.shift}px`;
    // A physical slide and a soft thunk when it lands.
    H.noise({ dur: v ? 0.26 : 0.16, gain: 0.12, filter: "lowpass", freq: v ? 1100 : 700, sweepTo: 250, to: out(0.05) });
    H.tone({ when: v ? 0.2 : 0.14, freq: 95, glideTo: 55, dur: 0.12, gain: 0.3, to: out(0.05) });
  }

  function hit(H, i, who) {
    if (S.open) {
      const el = S.pads[i];
      el.classList.remove("hit");
      void el.offsetWidth;
      el.classList.add("hit");
      if (who !== H.me) {
        const by = H.el(`<b class="p5-by">${who.initials}</b>`);
        el.appendChild(by);
        later(() => by.remove(), 1100);
      }
    }
    P.play(H, who, i);
  }

  H.register(P);
})();
