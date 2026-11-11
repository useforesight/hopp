// @ts-nocheck
// 4. Reaction wheel: hold the button, a ring of 8 reactions springs out; drag to one and let go.
// The emoji flies HUGE from the sender's tile, then bursts into a shower of itself with its own sound.
(() => {
  let alive = false,
    gen = 0,
    last = "party";
  const R = 96,
    HOLD_MS = 220;

  // ---------- Sound bits ----------
  const ac = () => H.ac();
  const voice = (freq, when, dur, gain, type, filt) => {
    // osc -> env -> (filter) -> dest; returns {osc, filter}
    const a = ac(),
      t = a.currentTime + when,
      o = a.createOscillator(),
      g = a.createGain();
    o.type = type;
    o.frequency.setValueAtTime(freq, t);
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(gain, t + 0.015);
    g.gain.setValueAtTime(gain, t + dur * 0.6);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    o.connect(g);
    let out = g,
      f = null;
    if (filt) {
      f = a.createBiquadFilter();
      f.type = filt.type || "lowpass";
      f.frequency.setValueAtTime(filt.freq, t);
      f.Q.value = filt.q ?? 1;
      g.connect(f);
      out = f;
    }
    out.connect(H.dest(filt?.wet ?? 0.25));
    o.start(t);
    o.stop(t + dur + 0.05);
    return { o, f, t };
  };
  const pop = (when, g = 0.5) => {
    H.noise({ when, filter: "highpass", freq: 1100, dur: 0.07, gain: g, wet: 0.15 });
    H.tone({ when, freq: 240, glideTo: 60, dur: 0.1, gain: g * 0.8, wet: 0.05 });
  };
  const brass = (f, when, dur, g) => {
    [-12, 0, 9].forEach((d) => {
      const v = voice(f * Math.pow(2, d / 1200), when, dur, g, "sawtooth", { freq: 300, q: 2, wet: 0.45 });
      v.f.frequency.exponentialRampToValueAtTime(2600, v.t + 0.05);
      v.f.frequency.exponentialRampToValueAtTime(700, v.t + dur);
      if (dur > 1) {
        const l = ac().createOscillator(),
          lg = ac().createGain();
        l.frequency.value = 5.5;
        lg.gain.value = f * 0.012;
        l.connect(lg);
        lg.connect(v.o.frequency);
        l.start(v.t + 0.3);
        l.stop(v.t + dur);
      }
    });
    H.tone({ when, freq: f / 2, dur, gain: g * 1.4, attack: 0.02, wet: 0.2 });
  };

  const SFX = {
    party() {
      const v = voice(400, 0, 0.5, 0.14, "sawtooth", { type: "bandpass", freq: 1500, q: 3, wet: 0.2 }); // party horn
      v.o.frequency.linearRampToValueAtTime(520, v.t + 0.12);
      for (let i = 0; i < 7; i++) pop(0.32 + i * H.rand(0.05, 0.09), H.rand(0.3, 0.55));
      H.noise({ when: 0.4, dur: 1.4, gain: 0.07, freq: 5200, sweepTo: 2600, q: 0.6, attack: 0.08 });
    },
    fire() {
      H.noise({ filter: "lowpass", freq: 250, sweepTo: 3200, dur: 0.6, attack: 0.2, gain: 0.55, wet: 0.35 });
      H.noise({ when: 0.3, filter: "lowpass", freq: 700, sweepTo: 300, dur: 1.5, attack: 0.1, gain: 0.25, wet: 0.3 });
      for (let i = 0; i < 46; i++)
        H.noise({
          when: 0.15 + Math.pow(Math.random(), 1.4) * 1.7,
          filter: "highpass",
          freq: H.rand(2500, 6000),
          dur: H.rand(0.006, 0.02),
          gain: H.rand(0.08, 0.4),
          wet: 0.1,
        });
    },
    applause() {
      for (let i = 0; i < 80; i++) {
        const t = Math.pow(Math.random(), 1.3) * 2.3;
        H.noise({
          when: t,
          filter: "bandpass",
          freq: H.rand(900, 2400),
          q: 1.3,
          dur: H.rand(0.025, 0.05),
          gain: 0.32 * (1 - t / 2.7),
          attack: 0.001,
          wet: 0.35,
        });
      }
    },
    lol() {
      const a = ac();
      [232, 214, 200, 186, 170].forEach((f, i) => {
        const when = i * 0.17,
          dur = 0.14,
          t = a.currentTime + when;
        H.noise({
          when: Math.max(0, when - 0.03),
          filter: "bandpass",
          freq: 1600,
          q: 0.8,
          dur: 0.05,
          gain: 0.12,
          attack: 0.01,
        }); // the "h"
        const o = a.createOscillator(),
          g = a.createGain(),
          out = H.dest(0.15);
        o.type = "sawtooth";
        o.frequency.setValueAtTime(f * 1.08, t);
        o.frequency.exponentialRampToValueAtTime(f * 0.88, t + dur);
        g.gain.setValueAtTime(0.0001, t);
        g.gain.exponentialRampToValueAtTime(0.5, t + 0.02);
        g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
        o.connect(g);
        [
          [680, 6, 1],
          [1080, 7, 0.6],
          [2400, 8, 0.15],
        ].forEach(([ff, q, lv]) => {
          // formants of a deep "ah"
          const bp = a.createBiquadFilter(),
            lg = a.createGain();
          bp.type = "bandpass";
          bp.frequency.value = ff;
          bp.Q.value = q;
          lg.gain.value = lv * 1.6;
          g.connect(bp);
          bp.connect(lg);
          lg.connect(out);
        });
        o.start(t);
        o.stop(t + dur + 0.05);
      });
    },
    rocketLaunch(sec) {
      H.noise({
        filter: "lowpass",
        freq: 160,
        sweepTo: 2800,
        q: 0.9,
        dur: sec + 0.15,
        attack: 0.25,
        gain: 0.45,
        wet: 0.25,
      });
      const v = voice(55, 0, sec + 0.1, 0.12, "sawtooth", { freq: 700, wet: 0.2 });
      v.o.frequency.exponentialRampToValueAtTime(340, v.t + sec);
    },
    rocket() {
      H.tone({ freq: 130, glideTo: 32, dur: 0.7, gain: 0.85, wet: 0.3 });
      H.noise({ filter: "lowpass", freq: 3000, sweepTo: 300, dur: 0.6, gain: 0.6, wet: 0.4 });
      for (let i = 0; i < 16; i++)
        H.noise({
          when: 0.25 + H.rand(0, 1.1),
          filter: "highpass",
          freq: 4000,
          dur: 0.02,
          gain: H.rand(0.1, 0.3),
          wet: 0.5,
        });
    },
    hundred() {
      H.tone({ freq: 988, dur: 0.09, gain: 0.13, type: "square", attack: 0.002, wet: 0.1 });
      H.tone({ when: 0.08, freq: 1319, dur: 0.55, gain: 0.13, type: "square", attack: 0.002, wet: 0.15 });
      [1319, 1568, 1976, 2637, 3136].forEach((f, i) =>
        H.tone({ when: 0.3 + i * 0.055, freq: f, dur: 0.3, gain: 0.08, type: "triangle", wet: 0.3 }),
      );
    },
    love() {
      H.tone({ freq: 261.6, dur: 1.8, attack: 0.25, gain: 0.1, wet: 0.5 });
      [523.3, 659.3, 784, 987.8, 1318.5].forEach((f, i) => {
        H.tone({ when: i * 0.07, freq: f, dur: 2.2, gain: 0.11, attack: 0.008, wet: 0.65 });
        H.tone({ when: i * 0.07, freq: f * 2.76, dur: 0.6, gain: 0.02, wet: 0.65 });
      });
    },
    shock() {
      brass(196, 0, 0.2, 0.09);
      brass(185, 0.27, 0.2, 0.09);
      brass(155.6, 0.56, 1.6, 0.1);
      H.tone({ when: 0.56, freq: 82, glideTo: 50, dur: 1.3, gain: 0.7, wet: 0.3 });
      H.noise({ when: 0.56, filter: "lowpass", freq: 400, dur: 0.6, gain: 0.4, wet: 0.3 });
    },
  };

  // ---------- The reactions ----------
  const sparks = (colors) => ({ shape: "spark", glow: true, colors, size: [4, 8], gravity: 0.05 });
  const R8 = [
    {
      k: "party",
      e: "🎉",
      label: "party",
      verb: "started a party",
      hue: "#ffd166",
      trail: { shape: "rect", size: [5, 9] },
      extra: { shape: "rect", count: 90, size: [7, 12] },
    },
    {
      k: "fire",
      e: "🔥",
      label: "fire",
      verb: "says that's fire",
      hue: "#ff6a00",
      grav: -0.14,
      trail: sparks(["#ffb000", "#ff6a00", "#ffdd55"]),
      extra: { ...sparks(["#ffb000", "#ff5a00", "#ffe066"]), gravity: -0.12, count: 70 },
    },
    {
      k: "applause",
      e: "👏",
      label: "applause",
      verb: "is applauding",
      hue: "#ffcf8a",
      trail: sparks(["#fff", "#ffe2a8"]),
      extra: { shape: "star", count: 30, colors: ["#ffd166", "#fff"] },
    },
    {
      k: "lol",
      e: "😂",
      label: "lol",
      verb: "is crying laughing",
      hue: "#7fd3ff",
      trail: { shape: "circle", colors: ["#7fd3ff", "#bfe9ff"], size: [5, 9], gravity: 0.2 },
      extra: { shape: "circle", count: 50, colors: ["#7fd3ff", "#bfe9ff", "#4cc9f0"], size: [6, 12] },
    },
    {
      k: "rocket",
      e: "🚀",
      label: "ship it",
      verb: "says ship it",
      hue: "#8b5cf6",
      tilt: true,
      trail: { shape: "circle", colors: ["rgba(230,230,240,.8)", "#ffb347", "#ff6a00"], size: [8, 16], gravity: -0.03 },
      extra: { ...sparks(H.confettiColors), count: 120, speed: [6, 20], gravity: 0.2 },
    },
    {
      k: "hundred",
      e: "💯",
      label: "hundred",
      verb: "says 100",
      hue: "#ff3b5c",
      trail: sparks(["#ffd166", "#fff3b0"]),
      extra: { shape: "star", count: 50, colors: ["#ffd166", "#ffe066", "#fff"], size: [8, 14] },
    },
    {
      k: "love",
      e: "❤️",
      label: "love",
      verb: "sent love",
      hue: "#ff5d8f",
      grav: -0.07,
      trail: { shape: "emoji", emoji: "💗", size: [12, 20], gravity: -0.05 },
      extra: { shape: "emoji", emoji: ["💖", "💕", "💗"], count: 26, size: [16, 30], gravity: -0.06 },
    },
    {
      k: "shock",
      e: "😱",
      label: "shock",
      verb: "is shocked",
      hue: "#a78bfa",
      trail: sparks(["#c4b5fd", "#fff"]),
      extra: { shape: "tri", count: 50, colors: ["#a78bfa", "#6d28d9", "#fff"] },
    },
  ];
  const byKey = (k) => R8.find((r) => r.k === k) || R8[0];

  const css = () =>
    H.css(
      "p4",
      `
    .h-react-btn.p4-btn { position: relative; font-size: 16px !important; }
    .h-react-btn.p4-btn::after { content: ""; position: absolute; inset: -4px; border-radius: 50%; border: 2.5px solid rgba(255,255,255,.95);
      opacity: 0; transform: scale(.7); transition: opacity .1s, transform .1s; }
    .h-react-btn.p4-btn.holding::after { opacity: 1; transform: scale(1); transition: opacity ${HOLD_MS}ms, transform ${HOLD_MS}ms cubic-bezier(.2,1.5,.4,1); }
    .h-react-btn.p4-btn::before { content: "•••"; position: absolute; bottom: -9px; left: 50%; transform: translateX(-50%); font-size: 7px; letter-spacing: 1px; opacity: .7; color: #fff; }
    .p4-wheel { position: fixed; width: 0; height: 0; }
    .p4-disc { position: absolute; left: ${-R - 46}px; top: ${-R - 46}px; width: ${2 * R + 92}px; height: ${2 * R + 92}px; border-radius: 50%;
      background: radial-gradient(circle, rgba(20,12,36,.25) 0 22%, rgba(20,12,36,.78) 40%, rgba(20,12,36,.82) 70%, rgba(20,12,36,0) 72%);
      backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px); }
    .p4-item { position: absolute; width: 58px; height: 58px; margin: -29px 0 0 -29px; cursor: pointer; }
    .p4-bub { width: 100%; height: 100%; border-radius: 50%; display: grid; place-items: center; font-size: 32px;
      background: rgba(255,255,255,.12); box-shadow: inset 0 0 0 1px rgba(255,255,255,.18); transition: transform .28s cubic-bezier(.2,1.8,.4,1), background .15s, box-shadow .15s; }
    .p4-item.on .p4-bub { transform: scale(1.34); background: var(--hue); box-shadow: 0 0 0 3px #fff, 0 10px 30px var(--hue); }
    .p4-lab { position: absolute; left: 50%; top: 100%; margin-top: 8px; transform: translate(-50%, -4px) scale(.8); white-space: nowrap; padding: 3px 9px; border-radius: 999px;
      font-size: 12px; font-weight: 700; background: #fff; color: #2a1640; opacity: 0; transition: opacity .12s, transform .25s cubic-bezier(.2,1.6,.4,1); z-index: 2; }
    .p4-item.on .p4-lab { opacity: 1; transform: translate(-50%, 6px) scale(1); }
    .p4-fly { position: fixed; left: 0; top: 0; font-size: 240px; line-height: 1; width: 1.2em; height: 1.2em; margin: -.6em 0 0 -.6em;
      display: grid; place-items: center; filter: drop-shadow(0 18px 30px rgba(0,0,0,.35)); will-change: transform; }
    .p4-shock { position: fixed; border-radius: 50%; border: 5px solid var(--hue); box-shadow: 0 0 30px var(--hue); }
  `,
    );

  // ---------- Effect ----------
  function play(who, key) {
    const r = byKey(key),
      g = gen;
    const from = H.centerOf(H.tiles[who.key] || H.tiles.call);
    const to = { x: innerWidth * H.rand(0.36, 0.52), y: innerHeight * H.rand(0.34, 0.46) };
    const ctrl = { x: (from.x + to.x) / 2, y: Math.min(from.y, to.y) - innerHeight * 0.45 };
    const el = H.add(`<div class="p4-fly">${r.e}</div>`),
      FLY = 900,
      WIND = 230;
    H.banner(who, r.verb, r.e);
    H.tone({ freq: 320, glideTo: 720, dur: 0.12, gain: 0.25, wet: 0.1 });
    H.noise({
      filter: "bandpass",
      freq: 400,
      sweepTo: 2400,
      q: 1.2,
      dur: FLY / 1000,
      attack: 0.5,
      gain: 0.12,
      wet: 0.25,
    });
    if (r.k === "rocket") SFX.rocketLaunch(FLY / 1000);
    const t0 = performance.now();
    let frame = 0,
      px = from.x,
      py = from.y,
      rot0 = 0;
    const step = (now) => {
      if (!alive || g !== gen) return el.remove();
      const t = now - t0;
      if (t < FLY) {
        const k = 1 - Math.pow(1 - t / FLY, 2.4),
          u = 1 - k; // decelerate into the burst point
        const x = u * u * from.x + 2 * u * k * ctrl.x + k * k * to.x,
          y = u * u * from.y + 2 * u * k * ctrl.y + k * k * to.y;
        const s = 0.22 + 0.78 * Math.pow(k, 0.7);
        const rot = r.tilt ? (Math.atan2(y - py, x - px) * 180) / Math.PI + 45 : Math.sin(t / 90) * 14 * (1 - k);
        el.style.transform = `translate(${x}px,${y}px) rotate(${rot}deg) scale(${s})`;
        rot0 = rot;
        if (frame++ % 2 === 0 && t > 60)
          H.emit({
            x,
            y,
            count: 2,
            angle: 0,
            spread: 360,
            speed: [0.5, 2.5],
            life: [25, 45],
            drag: 0.95,
            gravity: 0.1,
            ...r.trail,
          });
        px = x;
        py = y;
      } else if (t < FLY + WIND) {
        // anticipation: swell and quiver before the burst
        const k = (t - FLY) / WIND;
        el.style.transform = `translate(${to.x + Math.sin(t / 18) * 5 * k}px,${to.y}px) rotate(${r.tilt ? rot0 : Math.sin(t / 30) * 6 * k}deg) scale(${1 + 0.28 * k * k}, ${1 + 0.2 * k * k})`;
      } else return burst(r, to, el);
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  function burst(r, p, el) {
    el.animate(
      [
        { transform: `translate(${p.x}px,${p.y}px) scale(1.3)`, opacity: 1 },
        { transform: `translate(${p.x}px,${p.y}px) scale(1.9)`, opacity: 0 },
      ],
      { duration: 180, easing: "ease-out", fill: "forwards" },
    ).onfinish = () => el.remove();
    (SFX[r.k] || SFX.party)();
    H.emit({
      x: p.x,
      y: p.y,
      count: 34,
      angle: -90,
      spread: 360,
      speed: [5, 16],
      gravity: r.grav ?? 0.32,
      drag: 0.975,
      life: [70, 120],
      size: [30, 66],
      shape: "emoji",
      emoji: r.e,
      spin: true,
    });
    H.emit({ x: p.x, y: p.y, angle: -90, spread: 360, speed: [4, 15], life: [50, 100], drag: 0.97, ...r.extra });
    for (let i = 0; i < 2; i++) {
      const ring = H.add(
        `<div class="p4-shock" style="--hue:${r.hue};left:${p.x - 60}px;top:${p.y - 60}px;width:120px;height:120px"></div>`,
      );
      ring.animate(
        [
          { transform: "scale(.4)", opacity: 1 },
          { transform: `scale(${7 + i * 4})`, opacity: 0 },
        ],
        { duration: 800 + i * 300, delay: i * 90, easing: "cubic-bezier(.1,.7,.3,1)", fill: "backwards" },
      ).onfinish = () => ring.remove();
    }
    H.flash(r.hue, 380, 0.16);
    H.shake(r.k === "rocket" ? 14 : 7, 380);
    if (r.k === "shock")
      setTimeout(() => {
        if (alive) {
          H.flash("#000", 1100, 0.4);
          H.shake(12, 600);
        }
      }, 560);
  }

  // ---------- The wheel ----------
  let state = "idle",
    wheel = null,
    hover = -1,
    holdT = 0,
    down = null,
    btn = null;
  const centre = () => H.centerOf(btn);
  function openWheel() {
    clearTimeout(holdT);
    btn.classList.remove("holding");
    const c = centre();
    wheel = H.add(`<div class="p4-wheel" style="left:${c.x}px;top:${c.y}px"><div class="p4-disc"></div></div>`);
    wheel.c = c;
    wheel.querySelector(".p4-disc").animate(
      [
        { transform: "scale(.3)", opacity: 0 },
        { transform: "scale(1)", opacity: 1 },
      ],
      { duration: 320, easing: "cubic-bezier(.2,1.3,.4,1)" },
    );
    wheel.items = R8.map((r, i) => {
      const a = ((-90 + i * 45) * Math.PI) / 180,
        x = Math.cos(a) * R,
        y = Math.sin(a) * R;
      const it = H.el(
        `<div class="p4-item interactive" style="left:${x}px;top:${y}px;--hue:${r.hue}"><div class="p4-bub">${r.e}</div><div class="p4-lab">${r.label}</div></div>`,
      );
      wheel.appendChild(it);
      it.animate(
        [
          { transform: `translate(${-x}px,${-y}px) scale(.15) rotate(-90deg)`, opacity: 0 },
          { transform: "none", opacity: 1 },
        ],
        { duration: 460, delay: i * 28, easing: "cubic-bezier(.2,1.6,.35,1)", fill: "backwards" },
      );
      H.tone({
        when: i * 0.028,
        freq: [523, 587, 659, 784, 880, 1047, 1175, 1319][i],
        dur: 0.09,
        gain: 0.04,
        wet: 0.3,
      });
      return it;
    });
    H.noise({ filter: "bandpass", freq: 600, sweepTo: 2600, q: 1, dur: 0.25, attack: 0.12, gain: 0.07 });
    setHover(-1);
  }
  function pickAt(x, y) {
    if (!wheel) return -1;
    const dx = x - wheel.c.x,
      dy = y - wheel.c.y,
      d = Math.hypot(dx, dy);
    if (d < 34 || d > R + 90) return -1;
    return ((Math.round(((Math.atan2(dy, dx) * 180) / Math.PI + 90) / 45) % 8) + 8) % 8;
  }
  function setHover(i) {
    if (!wheel || i === hover) return;
    hover = i;
    wheel.items.forEach((it, j) => it.classList.toggle("on", j === i));
    if (i >= 0) H.tone({ freq: 900 + i * 90, dur: 0.045, gain: 0.06, type: "triangle", wet: 0.1 });
  }
  function closeWheel(chosen = -1) {
    state = "idle";
    hover = -1;
    if (!wheel) return;
    const w = wheel;
    wheel = null;
    w.items.forEach((it, j) => {
      const kf =
        j === chosen ?
          [
            { transform: "scale(1)", opacity: 1 },
            { transform: "scale(1.8)", opacity: 0 },
          ]
        : [
            { transform: "none", opacity: 1 },
            {
              transform: `translate(${-parseFloat(it.style.left)}px,${-parseFloat(it.style.top)}px) scale(.2)`,
              opacity: 0,
            },
          ];
      it.classList.remove("interactive");
      it.animate(kf, {
        duration: 200,
        delay: j === chosen ? 0 : (7 - j) * 12,
        easing: "cubic-bezier(.5,0,.8,.4)",
        fill: "forwards",
      });
    });
    w.querySelector(".p4-disc").animate([{ opacity: 1 }, { opacity: 0, transform: "scale(.5)" }], {
      duration: 260,
      fill: "forwards",
    });
    setTimeout(() => w.remove(), 320);
  }
  function choose(i) {
    const r = R8[i];
    closeWheel(i);
    last = r.k;
    btn.querySelector("span").textContent = r.e;
    btn.animate([{ scale: 1 }, { scale: 1.35 }, { scale: 1 }], { duration: 300, easing: "cubic-bezier(.2,1.6,.4,1)" });
    play(H.me, r.k);
  }

  function onDown(e) {
    if (state === "open") {
      const i = pickAt(e.clientX, e.clientY);
      if (i >= 0) choose(i);
      else closeWheel();
      if (btn.contains(e.target)) e.preventDefault();
      return;
    }
    if (!btn.contains(e.target) || e.button !== 0) return;
    e.preventDefault();
    H.ac();
    state = "pressing";
    down = { x: e.clientX, y: e.clientY };
    btn.classList.add("holding");
    holdT = setTimeout(() => {
      if (state === "pressing") {
        state = "dragging";
        openWheel();
      }
    }, HOLD_MS);
  }
  function onMove(e) {
    if (state === "pressing" && Math.hypot(e.clientX - down.x, e.clientY - down.y) > 14) {
      state = "dragging";
      openWheel();
    }
    if (state === "dragging" || state === "open") setHover(pickAt(e.clientX, e.clientY));
  }
  function onUp() {
    if (state === "pressing") {
      clearTimeout(holdT);
      btn.classList.remove("holding");
      state = "idle";
      btn.animate([{ scale: 1 }, { scale: 0.85 }, { scale: 1.15 }, { scale: 1 }], {
        duration: 280,
        easing: "ease-out",
      });
      play(H.me, last);
    } else if (state === "dragging") {
      if (hover >= 0) choose(hover);
      else state = "open"; // let go in the middle: wheel stays open for a click
    }
  }
  const onKey = (e) => {
    if (e.key === "Escape" && wheel) closeWheel();
  };

  H.register({
    n: 4,
    title: "Reaction wheel",
    pitch:
      "Hold the button and a wheel of 8 reactions springs out. Pick one and it flies HUGE across everyone's screen, then bursts into a shower of itself, each with its own sound.",
    how: "Press and hold the button, drag onto a reaction and let go (or let go in the middle and click one). A quick tap sends your last one again.",
    mount(H) {
      alive = true;
      state = "idle";
      wheel = null;
      hover = -1;
      css();
      btn = H.makeButton(`<span>${byKey(last).e}</span>`, "Hold for reactions, tap to repeat");
      btn.classList.add("p4-btn");
      btn.style.touchAction = "none";
      H.slot.appendChild(btn);
      addEventListener("pointerdown", onDown, true);
      addEventListener("pointermove", onMove);
      addEventListener("pointerup", onUp);
      addEventListener("pointercancel", onUp);
      addEventListener("keydown", onKey);
      H.onLeave(() => {
        alive = false;
        gen++;
        clearTimeout(holdT);
        state = "idle";
        wheel = null;
        removeEventListener("pointerdown", onDown, true);
        removeEventListener("pointermove", onMove);
        removeEventListener("pointerup", onUp);
        removeEventListener("pointercancel", onUp);
        removeEventListener("keydown", onKey);
      });
    },
    play(H, who, choice) {
      play(who, choice || last);
    },
  });
})();
