// @ts-nocheck
// 2. The Gong: a bronze gong swings into the middle of everyone's screen, a mallet winds up and strikes it.
// While it hangs there, clicking the gong hits it again, harder each time.
(() => {
  const WOOD_V =
    "repeating-linear-gradient(90deg,rgba(0,0,0,0) 0 6px,rgba(0,0,0,.14) 7px,rgba(0,0,0,0) 9px 13px),linear-gradient(90deg,#2e1406,#6e3514 22%,#a25a2a 50%,#6e3514 78%,#2e1406)";
  const WOOD_H =
    "repeating-linear-gradient(0deg,rgba(0,0,0,0) 0 5px,rgba(0,0,0,.13) 6px,rgba(0,0,0,0) 8px 12px),linear-gradient(180deg,#b5683a,#7a3d18 45%,#3b1a09)";
  let st = null,
    alive = false,
    timers = new Set(),
    buses = [];
  const later = (fn, ms) => {
    const id = setTimeout(() => {
      timers.delete(id);
      if (alive) fn();
    }, ms);
    timers.add(id);
    return id;
  };
  const bus = (wet) => {
    const g = H.ac().createGain();
    g.connect(H.dest(wet));
    buses.push(g);
    return g;
  };

  const css = () =>
    H.css(
      "p2",
      `
    .p2-mini { width: 17px; height: 17px; border-radius: 50%; background: radial-gradient(circle at 40% 35%, #fff2b8, #e0a540 35%, #8a5418 80%, #4a2a08);
      box-shadow: 0 0 0 1.5px #5b3410, 0 1px 3px rgba(0,0,0,.4); }
    .p2-dim { position: fixed; inset: 0; background: radial-gradient(ellipse at 50% 50%, rgba(25,10,0,0) 20%, rgba(12,5,0,.62) 100%); opacity: 0; transition: opacity .5s ease; }
    .p2-dim.on { opacity: 1; }
    .p2-rig { position: fixed; will-change: transform; }
    .p2-rig > div, .p2-hang > div { position: absolute; }
    .p2-post { border-radius: 6px 6px 3px 3px; box-shadow: inset -3px 0 6px rgba(0,0,0,.35), 0 10px 25px rgba(0,0,0,.45); background: ${WOOD_V}; }
    .p2-bar { border-radius: 10px; background: ${WOOD_H}; box-shadow: 0 8px 20px rgba(0,0,0,.45), inset 0 2px 0 rgba(255,210,160,.35); }
    .p2-cap { border-radius: 50%; background: radial-gradient(circle at 35% 30%, #fff1b0, #d49a35 45%, #6b4210); box-shadow: 0 2px 6px rgba(0,0,0,.5); }
    .p2-foot { border-radius: 8px 8px 4px 4px; background: ${WOOD_H}; box-shadow: 0 10px 18px rgba(0,0,0,.5); }
    .p2-hang { transform-origin: 50% 0; }
    .p2-cord { width: 4px; margin-left: -2px; transform-origin: 50% 0; border-radius: 2px; background: linear-gradient(90deg,#1b0d04,#5a3a1c,#1b0d04); }
    .p2-glow { border-radius: 50%; opacity: 0; background: radial-gradient(circle, rgba(255,214,120,.95) 0%, rgba(255,170,60,.5) 35%, rgba(255,140,40,0) 70%); }
    .p2-disc { border-radius: 50%; cursor: pointer; overflow: hidden;
      background:
        radial-gradient(circle at 44% 41%, rgba(255,250,220,.95) 0 1.5%, rgba(255,236,170,.6) 4%, rgba(255,236,170,0) 10%),
        radial-gradient(circle at 50% 50%, #f9dd8a 0 6%, #d79b3c 11%, #9a5f1d 14.5%, rgba(60,30,5,.55) 15.5%, rgba(255,220,140,.35) 17%, rgba(0,0,0,0) 19%),
        radial-gradient(circle at 50% 50%, rgba(0,0,0,0) 0 55%, rgba(60,30,5,.35) 80%, rgba(30,14,2,.75) 94%, rgba(255,214,130,.55) 95.5%, rgba(40,20,4,.9) 97%),
        repeating-radial-gradient(circle at 50% 50%, rgba(255,240,200,.07) 0 2px, rgba(0,0,0,.07) 3px 5px),
        conic-gradient(from 25deg, #8a5418, #f6d27a, #a8681f, #e9b85a, #7a4612, #f1c66b, #9c5f1c, #ffe39a, #8a5418);
      box-shadow: 0 30px 70px rgba(0,0,0,.55), 0 0 0 3px #4a2a08; transition: filter .15s; }
    .p2-disc:hover { filter: brightness(1.08) saturate(1.1); }
    .p2-sheen { position: absolute; inset: 0; border-radius: 50%; mix-blend-mode: overlay;
      background: conic-gradient(from 0deg, rgba(255,255,255,0) 0 8%, rgba(255,250,225,.75) 13%, rgba(255,255,255,0) 19% 44%, rgba(255,250,225,.6) 50%, rgba(255,255,255,0) 56% 80%, rgba(255,255,255,.45) 86%, rgba(255,255,255,0) 92%); }
    .p2-light { position: absolute; inset: 0; border-radius: 50%; background: radial-gradient(ellipse at 32% 24%, rgba(255,255,255,.4), rgba(255,255,255,0) 45%); pointer-events: none; }
    .p2-mallet { transform-origin: 100% 50%; border-radius: 6px; background: ${WOOD_H}; box-shadow: 0 6px 14px rgba(0,0,0,.45); opacity: 0; }
    .p2-head { position: absolute; border-radius: 50%; background: radial-gradient(circle at 35% 30%, #ff9b8a, #c4162f 42%, #5a0716 100%); box-shadow: inset -4px -6px 10px rgba(0,0,0,.4), 0 8px 16px rgba(0,0,0,.45); }
    .p2-ring { position: fixed; border-radius: 50%; border: 6px solid rgba(255,222,150,.9); box-shadow: 0 0 40px rgba(255,180,60,.7), inset 0 0 40px rgba(255,180,60,.45); }
    .p2-ring.first { background: radial-gradient(circle, rgba(255,220,140,0) 55%, rgba(255,220,140,.28) 70%, rgba(255,220,140,0) 72%); }
    .p2-hint { position: fixed; transform: translateX(-50%); padding: 6px 13px; border-radius: 999px; font-size: 13px; font-weight: 700; letter-spacing: .02em;
      background: rgba(20,10,0,.65); border: 1px solid rgba(255,214,130,.4); color: #ffe6a8; opacity: 0; transition: opacity .4s, transform .4s cubic-bezier(.2,1.5,.4,1); }
    .p2-hint.on { opacity: 1; transform: translateX(-50%) translateY(-6px); }
  `,
    );

  // ---------- Sound ----------
  function gongSound(p) {
    const a = H.ac(),
      now = a.currentTime;
    const wet = bus(0.85),
      dry = bus(0.3),
      v = Math.min(1, 0.6 + p * 0.09);
    // Slow tremolo on the wet bus gives the shimmering wash.
    const lfo = a.createOscillator(),
      lg = a.createGain();
    lfo.frequency.value = H.rand(2.6, 3.6);
    lg.gain.value = 0.22;
    lfo.connect(lg);
    lg.connect(wet.gain);
    lfo.start(now);
    lfo.stop(now + 7);
    const f0 = H.rand(70, 76);
    // [ratio, gain, decay seconds]: inharmonic partials from ~72 Hz up to ~560 Hz.
    const partials = [
      [1, 1, 5.6],
      [1.47, 0.72, 5.2],
      [2.09, 0.58, 4.6],
      [2.56, 0.5, 4.2],
      [3.14, 0.38, 3.8],
      [3.71, 0.32, 3.4],
      [4.42, 0.24, 3.1],
      [5.21, 0.18, 2.7],
      [6.33, 0.13, 2.4],
      [7.6, 0.1, 2.1],
    ];
    partials.forEach(([r, g, d], i) => {
      const f = f0 * r;
      H.tone({
        freq: f,
        glideTo: f * 0.985,
        dur: d,
        gain: g * 0.17 * v,
        attack: 0.004,
        detune: -4,
        to: i < 3 ? dry : wet,
      });
      H.tone({ freq: f + 0.35 + i * 0.17, dur: d * 0.95, gain: g * 0.12 * v, attack: 0.01, detune: 5, to: wet }); // beats against the first
    });
    // The tam-tam "bloom": upper partials swell in after the hit.
    for (let i = 0; i < 6; i++)
      H.tone({
        freq: H.rand(620, 1500),
        dur: H.rand(2.8, 4),
        attack: H.rand(0.35, 0.9),
        gain: 0.018 * v * (1 + p * 0.2),
        to: wet,
      });
    const wash = H.noise({
      filter: "bandpass",
      freq: 3200,
      sweepTo: 1100,
      q: 2.5,
      dur: 3.6,
      attack: 0.45,
      gain: 0.05 * v,
      to: wet,
    });
    wash.src.loop = true;
    // Strike transient: felt thump plus a short broadband smack.
    H.noise({ filter: "lowpass", freq: 1800, dur: 0.12, gain: 0.5 * v, to: dry });
    H.tone({ freq: 115, glideTo: 42, dur: 0.38, gain: 0.6 * v, to: dry });
    later(() => {
      [wet, dry].forEach((g) => {
        g.disconnect();
        buses = buses.filter((b) => b !== g);
      });
    }, 8000);
  }
  const whoosh = (up) =>
    H.noise({
      filter: "bandpass",
      freq: up ? 1300 : 280,
      sweepTo: up ? 260 : 1300,
      q: 1.1,
      dur: 0.6,
      attack: 0.3,
      gain: 0.2,
      wet: 0.3,
    });
  const clunk = () => {
    H.tone({ freq: 95, glideTo: 50, dur: 0.22, gain: 0.35, wet: 0.2 });
    H.tone({ freq: 420, glideTo: 360, dur: 0.07, gain: 0.12, type: "triangle", wet: 0.2 });
  };

  // ---------- Build ----------
  function build() {
    const D = Math.round(Math.min(innerHeight * 0.5, innerWidth * 0.36, 480)),
      R = D / 2;
    const W = D * 1.55,
      bh = D * 0.075,
      pw = D * 0.075,
      cord = D * 0.16,
      rigH = D * 1.6;
    const hangTop = bh * 0.6,
      cy = hangTop + cord + R,
      cx = W / 2;
    const left = innerWidth / 2 - W / 2,
      top = innerHeight / 2 - cy + 8;
    // Mallet: pivot below right of the gong, head resting near the boss.
    const P = { x: cx + 1.05 * R, y: cy + 0.95 * R },
      T = { x: cx + 0.2 * R, y: cy + 0.14 * R };
    const L = Math.hypot(P.x - T.x, P.y - T.y),
      rest = (Math.atan2(P.y - T.y, P.x - T.x) * 180) / Math.PI;
    const mh = D * 0.045,
      hs = D * 0.17;
    const cordX = D * 0.2,
      cordEnd = D * 0.13,
      cordLen = Math.hypot(cordX - cordEnd, cord + D * 0.02),
      cordAng = (Math.atan2(cordX - cordEnd, cord) * 180) / Math.PI;
    const dim = H.add(`<div class="p2-dim"></div>`);
    const rig = H.add(`<div class="p2-rig" style="left:${left}px;top:${top}px;width:${W}px;height:${rigH}px">
      <div class="p2-post" style="left:0;top:0;width:${pw}px;height:${rigH - pw * 0.6}px"></div>
      <div class="p2-post" style="right:0;top:0;width:${pw}px;height:${rigH - pw * 0.6}px"></div>
      <div class="p2-foot" style="left:${-pw}px;bottom:0;width:${pw * 3}px;height:${pw * 0.8}px"></div>
      <div class="p2-foot" style="right:${-pw}px;bottom:0;width:${pw * 3}px;height:${pw * 0.8}px"></div>
      <div class="p2-bar" style="left:${-pw * 0.8}px;right:${-pw * 0.8}px;top:${-bh * 0.3}px;height:${bh}px"></div>
      <div class="p2-cap" style="left:${-pw * 1.1}px;top:${-bh * 0.45}px;width:${bh * 1.3}px;height:${bh * 1.3}px"></div>
      <div class="p2-cap" style="right:${-pw * 1.1}px;top:${-bh * 0.45}px;width:${bh * 1.3}px;height:${bh * 1.3}px"></div>
      <div class="p2-glow" style="left:${cx - D}px;top:${cy - D}px;width:${D * 2}px;height:${D * 2}px"></div>
      <div class="p2-hang" style="left:${cx - R}px;top:${hangTop}px;width:${D}px;height:${cord + D}px">
        <div class="p2-cord" style="left:${R - cordX}px;top:0;height:${cordLen}px;transform:rotate(-${cordAng}deg)"></div>
        <div class="p2-cord" style="left:${R + cordX}px;top:0;height:${cordLen}px;transform:rotate(${cordAng}deg)"></div>
        <div class="p2-disc interactive" style="left:0;top:${cord}px;width:${D}px;height:${D}px"><div class="p2-sheen"></div><div class="p2-light"></div></div>
      </div>
      <div class="p2-mallet" style="left:${P.x - L}px;top:${P.y - mh / 2}px;width:${L}px;height:${mh}px;transform:rotate(${rest + 30}deg)">
        <div class="p2-head" style="left:${-hs * 0.45}px;top:${mh / 2 - hs / 2}px;width:${hs}px;height:${hs}px"></div>
      </div>
    </div>`);
    const hint = H.add(
      `<div class="p2-hint" style="left:50%;top:${top + rigH + 4}px">hit it again for a bigger gong</div>`,
    );
    const q = (s) => rig.querySelector(s);
    st = {
      D,
      rig,
      dim,
      hint,
      top,
      rigH,
      T,
      rest,
      power: 0,
      sheenAng: 0,
      counts: {},
      banner: null,
      bannerWho: null,
      leaveT: 0,
      hang: q(".p2-hang"),
      disc: q(".p2-disc"),
      sheen: q(".p2-sheen"),
      glow: q(".p2-glow"),
      mallet: q(".p2-mallet"),
    };
    st.disc.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      H.ac();
      hit(H.me);
    });
    return st;
  }

  // ---------- Motion ----------
  function swingMallet(intro, then) {
    const s = st,
      m = s.mallet,
      r = s.rest,
      Rt = (d, k = 1) => `rotate(${d}deg) scale(${k})`;
    const kf =
      intro ?
        [
          { transform: Rt(r + 35), opacity: 0, easing: "ease-out" },
          { transform: Rt(r + 18), opacity: 1, offset: 0.2, easing: "cubic-bezier(.2,.8,.3,1)" },
          { transform: Rt(r + 74, 1.2), opacity: 1, offset: 0.72, easing: "linear" },
          { transform: Rt(r + 77, 1.22), opacity: 1, offset: 0.84, easing: "cubic-bezier(.7,0,1,.6)" },
          { transform: Rt(r - 3, 0.95), opacity: 1 },
        ]
      : [
          { transform: Rt(r + 14), opacity: 1, easing: "cubic-bezier(.2,.8,.3,1)" },
          { transform: Rt(r + 52, 1.12), opacity: 1, offset: 0.55, easing: "cubic-bezier(.7,0,1,.6)" },
          { transform: Rt(r - 3, 0.95), opacity: 1 },
        ];
    const dur = intro ? 820 : 150;
    m.getAnimations().forEach((x) => x.cancel());
    later(
      () => H.noise({ filter: "bandpass", freq: 500, sweepTo: 1900, q: 1, dur: 0.16, gain: 0.13, attack: 0.12 }),
      dur * (intro ? 0.84 : 0.5),
    );
    m.animate(kf, { duration: dur, fill: "forwards" }).onfinish = () => {
      if (!alive || st !== s) return;
      then();
      m.animate(
        [
          { transform: Rt(r - 3, 0.95), opacity: 1 },
          { transform: Rt(r + 22, 1.04), opacity: 1, offset: 0.35 },
          { transform: Rt(r + 14), opacity: 1 },
        ],
        { duration: 520, easing: "ease-out", fill: "forwards" },
      );
    };
  }

  function ring(c, p, i) {
    const D = st ? st.D : 400,
      S = (Math.hypot(innerWidth, innerHeight) / D) * 1.2;
    const el = H.add(
      `<div class="p2-ring${i ? "" : " first"}" style="left:${c.x}px;top:${c.y}px;width:${D}px;height:${D}px;margin:${-D / 2}px 0 0 ${-D / 2}px;border-width:${3 + p + (i ? 0 : 3)}px"></div>`,
    );
    el.animate(
      [
        { transform: "scale(.96)", opacity: 0.95 },
        { transform: `scale(${S})`, opacity: 0 },
      ],
      { duration: 1400 + i * 140 + p * 60, easing: "cubic-bezier(.12,.7,.3,1)" },
    ).onfinish = () => el.remove();
  }

  function strike(p) {
    const s = st;
    if (!s) return;
    const c = H.centerOf(s.disc),
      rr = s.rig.getBoundingClientRect();
    const a = Math.min(5 + p * 3, 22),
      sw = Math.min(1.6 + p, 7);
    const wob = [];
    for (let i = 0; i <= 9; i++) {
      const k = i / 9,
        amp = a * Math.pow(1 - k, 1.6),
        sg = i % 2 ? 1 : -1;
      wob.push({
        transform: `perspective(${s.D * 2.4}px) rotateY(${i ? amp * sg : 0}deg) rotateX(${i ? -amp * 0.4 * sg : 0}deg) scale(${i === 1 ? 0.95 : 1})`,
        offset: i === 1 ? 0.04 : k,
      });
    }
    s.disc.animate(wob, { duration: 1700, easing: "ease-out" });
    s.hang.animate(
      [0, sw, -sw * 0.75, sw * 0.5, -sw * 0.3, sw * 0.12, 0].map((d) => ({ transform: `rotate(${d}deg)` })),
      { duration: 2300, easing: "ease-out" },
    );
    s.sheen.animate(
      [{ transform: `rotate(${s.sheenAng}deg)` }, { transform: `rotate(${(s.sheenAng += 100 + p * 45)}deg)` }],
      { duration: 2800, easing: "cubic-bezier(.1,.8,.2,1)", fill: "forwards" },
    );
    s.glow.animate(
      [
        { opacity: 0, transform: "scale(.8)" },
        { opacity: Math.min(0.6 + p * 0.1, 1), transform: "scale(1.1)", offset: 0.08 },
        { opacity: 0, transform: "scale(1.5)" },
      ],
      { duration: 1400 + p * 120, easing: "ease-out" },
    );
    for (let i = 0; i < Math.min(2 + p, 7); i++) later(() => ring(c, p, i), i * 115);
    H.shake(9 + p * 4, 650 + p * 110);
    H.flash("#ffcf70", 420, Math.min(0.1 + p * 0.05, 0.4));
    H.screen.animate([{ scale: 1 }, { scale: 1 + 0.004 * p + 0.006 }, { scale: 1 }], {
      duration: 300,
      easing: "ease-out",
    });
    H.emit({
      x: rr.left + s.T.x,
      y: rr.top + s.T.y,
      count: 18 + p * 12,
      angle: 0,
      spread: 360,
      speed: [3, 8 + p * 2],
      gravity: 0.18,
      drag: 0.96,
      life: [30, 70],
      size: [3, 6],
      shape: "spark",
      glow: true,
      colors: ["#fff3c4", "#ffd27a", "#ffb347", "#ffe9a0"],
    });
    gongSound(p);
    if (!s.hinted) {
      s.hinted = true;
      later(() => s.hint.isConnected && s.hint.classList.add("on"), 500);
    }
    clearTimeout(s.leaveT);
    timers.delete(s.leaveT);
    s.leaveT = later(leave, p === 1 ? 3300 : 2700);
  }

  function announce(who) {
    const s = st,
      n = (s.counts[who.key] = (s.counts[who.key] || 0) + 1);
    const text = n > 1 ? `hit the gong ×${n}` : "hit the gong";
    if (s.banner && s.banner.isConnected && s.bannerWho === who) {
      const span = s.banner.querySelector("span:not(.av):not(.emo)");
      if (span) span.textContent = `${who === H.me ? "You" : who.name} ${text}`;
      s.banner.animate([{ scale: 1 }, { scale: 1.08 }, { scale: 1 }], { duration: 260, easing: "ease-out" });
    } else {
      if (s.banner) s.banner.remove();
      s.banner = H.banner(who, text, "🔔", 3400);
      s.bannerWho = who;
    }
  }

  function hit(who) {
    if (!st) return;
    st.power = Math.min(st.power + 1, 6);
    const p = st.power;
    announce(who);
    clearTimeout(st.leaveT);
    timers.delete(st.leaveT);
    swingMallet(false, () => strike(p));
  }

  function leave() {
    const s = st;
    if (!s) return;
    st = null;
    s.disc.classList.remove("interactive");
    s.hint.classList.remove("on");
    s.dim.classList.remove("on");
    whoosh(true);
    s.rig.animate(
      [
        { transform: "none", easing: "ease-out" },
        { transform: "translateY(18px) rotate(-1deg)", offset: 0.24, easing: "cubic-bezier(.55,0,.85,.35)" },
        { transform: `translateY(${-(s.top + s.rigH + 80)}px) rotate(5deg)` },
      ],
      { duration: 820, fill: "forwards" },
    ).onfinish = () => s.rig.remove();
    later(() => {
      s.dim.remove();
      s.hint.remove();
    }, 900);
  }

  function intro(who) {
    const s = build();
    requestAnimationFrame(() => s.dim.classList.add("on"));
    const fromY = -(s.top + s.rigH + 40);
    s.rig.animate(
      [
        { transform: `translateY(${fromY}px) rotate(-7deg)`, easing: "cubic-bezier(.45,0,.9,.55)" },
        { transform: "translateY(16px) rotate(1deg)", offset: 0.68, easing: "cubic-bezier(.2,.7,.4,1)" },
        { transform: "translateY(-5px) rotate(-.3deg)", offset: 0.85, easing: "ease-in-out" },
        { transform: "none" },
      ],
      { duration: 760 },
    );
    whoosh(false);
    later(() => {
      clunk();
      s.hang.animate(
        [0, 8, -5.5, 3.2, -1.6, 0.6, 0].map((d) => ({ transform: `rotate(${d}deg)` })),
        { duration: 1500, easing: "ease-out" },
      );
    }, 520);
    announce(who);
    s.power = 1;
    later(() => {
      if (st === s) swingMallet(true, () => strike(1));
    }, 430);
  }

  H.register({
    n: 2,
    title: "The Gong",
    pitch:
      "A huge bronze gong swings down into the middle of everyone's screen and gets a proper whack. Rings ripple across the whole screen and a long gong sound plays.",
    how: "Click the gong button under the call. While the gong is up, click the gong itself to hit it again, harder each time.",
    mount(H) {
      alive = true;
      st = null;
      css();
      const b = H.makeButton(`<span class="p2-mini"></span>`, "Hit the gong");
      b.onclick = () => this.play(H, H.me);
      H.slot.appendChild(b);
      H.onLeave(() => {
        alive = false;
        st = null;
        timers.forEach(clearTimeout);
        timers.clear();
        if (buses.length) {
          const t = H.ac().currentTime;
          buses.forEach((g) => {
            try {
              g.gain.cancelScheduledValues(t);
              g.gain.setTargetAtTime(0, t, 0.05);
            } catch {}
          });
          const old = buses;
          buses = [];
          setTimeout(
            () =>
              old.forEach((g) => {
                try {
                  g.disconnect();
                } catch {}
              }),
            500,
          );
        }
      });
    },
    play(H, who) {
      if (st) hit(who);
      else intro(who);
    },
  });
})();
