// @ts-nocheck
// 10. Ship it: a mission-control countdown, a rocket launch through everyone's screen, then SHIPPED with confetti and a fanfare.
(() => {
  const esc = (s) =>
    String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const T = { three: 0.25, ignite: 2.65, lift: 3.45, flight: 1.9, shipped: 5.2, done: 8.6 };
  let run = null;

  const CSS = `
  .p10-split { display: flex; align-items: center; flex: none; border-radius: 999px; background: rgba(0,0,0,.18); padding-right: 2px; }
  .p10-caret { all: unset; cursor: pointer; width: 16px; height: 30px; display: grid; place-items: center; font-size: 9px; color: #2b2b33; opacity: .7; }
  .p10-caret:hover { opacity: 1; }
  .p10-pop { position: absolute; right: calc(100% + 14px); bottom: 4px; width: 250px; padding: 14px; border-radius: 16px; z-index: 30; color: #fff;
    background: rgba(14,18,34,.95); border: 1px solid rgba(120,180,255,.22); box-shadow: 0 24px 60px rgba(0,0,0,.45);
    transform-origin: 100% 85%; transform: translateX(12px) scale(.86); opacity: 0; pointer-events: none;
    transition: transform .38s cubic-bezier(.2,1.5,.4,1), opacity .18s; }
  .p10-pop.open { transform: none; opacity: 1; pointer-events: auto; }
  .p10-pop b { font-size: 14px; display: block; margin-bottom: 8px; }
  .p10-pop input { all: unset; display: block; width: 100%; box-sizing: border-box; padding: 9px 11px; border-radius: 9px; font: 600 14px ui-monospace, Menlo, monospace;
    background: rgba(255,255,255,.07); border: 1px solid rgba(255,255,255,.15); }
  .p10-pop input:focus { border-color: #5fb4ff; box-shadow: 0 0 0 3px rgba(95,180,255,.22); }
  .p10-go { all: unset; cursor: pointer; margin-top: 10px; display: block; text-align: center; padding: 9px; border-radius: 9px; font-weight: 750; font-size: 13px;
    background: linear-gradient(180deg,#ff6b5e,#e0283d); box-shadow: 0 6px 18px rgba(224,40,61,.4); }
  .p10-vig { position: fixed; inset: 0; opacity: 0; transition: opacity .5s; background: radial-gradient(70% 70% at 50% 55%, rgba(4,8,20,.25), rgba(2,4,12,.82)); }
  .p10-vig.on { opacity: 1; }
  .p10-num { position: fixed; left: 50%; top: 46%; transform: translate(-50%,-50%); font: 800 min(40vw, 380px)/1 ui-monospace, "SF Mono", Menlo, monospace;
    color: #fff; text-shadow: 0 0 30px rgba(120,190,255,.9), 0 0 90px rgba(60,140,255,.6); }
  .p10-cap { position: fixed; left: 50%; top: calc(46% - min(22vw, 210px)); transform: translateX(-50%); white-space: nowrap;
    font: 700 15px ui-monospace, Menlo, monospace; letter-spacing: .4em; color: #8fd0ff; opacity: 0; transition: opacity .3s; }
  .p10-cap.on { opacity: .9; }
  .p10-ring { position: fixed; left: 50%; top: 46%; width: 300px; height: 300px; margin: -150px; border-radius: 50%; border: 3px solid rgba(140,200,255,.8); }
  .p10-rocket { position: fixed; left: 50%; bottom: 74px; width: 120px; height: 280px; margin-left: -60px; transform-origin: 50% 100%; }
  .p10-flame { position: absolute; left: 50%; top: 214px; width: 52px; height: 130px; margin-left: -26px; transform-origin: 50% 0; transform: scale(.2, 0); }
  .p10-flame i { position: absolute; left: 0; right: 0; top: 0; bottom: 0; border-radius: 45% 45% 50% 50% / 20% 20% 80% 80%; animation: p10-fl .07s ease-in-out infinite alternate; }
  .p10-flame i:nth-child(1) { background: radial-gradient(50% 70% at 50% 15%, #fff7d6, #ffb02e 45%, rgba(255,70,20,.85) 70%, rgba(255,40,0,0)); filter: blur(2px); }
  .p10-flame i:nth-child(2) { left: 13px; right: 13px; bottom: 45%; background: radial-gradient(50% 70% at 50% 10%, #fff, #bfe8ff 50%, rgba(140,200,255,0)); animation-duration: .05s; }
  @keyframes p10-fl { from { transform: scaleY(.88) scaleX(1.05); } to { transform: scaleY(1.1) scaleX(.94); } }
  .p10-glow { position: fixed; left: 50%; bottom: -160px; width: 700px; height: 360px; margin-left: -350px; border-radius: 50%; opacity: 0;
    background: radial-gradient(closest-side, rgba(255,170,60,.85), rgba(255,90,20,.35) 55%, rgba(255,60,0,0)); }
  .p10-smoke { position: fixed; left: 0; top: 0; border-radius: 50%; will-change: transform, opacity;
    background: radial-gradient(closest-side, rgba(250,248,252,.95), rgba(228,224,234,.75) 45%, rgba(200,196,210,0)); }
  .p10-smoke.warm { background: radial-gradient(closest-side, rgba(255,226,190,.95), rgba(240,200,170,.7) 45%, rgba(210,180,170,0)); }
  .p10-title { position: fixed; left: 50%; top: 40%; transform: translate(-50%,-50%); text-align: center; white-space: nowrap; }
  .p10-title h1 { margin: 0; font: 900 min(13vw, 150px)/1 var(--font); letter-spacing: -.02em; }
  .p10-title h1 span { display: inline-block; background: linear-gradient(180deg,#fff 20%,#ffd166 60%,#ff7a2b); -webkit-background-clip: text; background-clip: text; color: transparent;
    filter: drop-shadow(0 6px 24px rgba(255,140,40,.55)); animation: p10-drop .7s cubic-bezier(.2,1.7,.4,1) both; }
  .p10-title h1 .emo { background: none; -webkit-background-clip: border-box; background-clip: border-box; color: initial; }
  .p10-title p { margin: 10px 0 0; font: 700 22px var(--font); opacity: 0; animation: p10-up .5s .55s cubic-bezier(.2,1.4,.4,1) forwards; }
  .p10-title p code { font: 800 22px ui-monospace, Menlo, monospace; padding: 2px 10px; border-radius: 8px; background: rgba(255,255,255,.14); }
  .p10-title.out { opacity: 0; transform: translate(-50%,-60%) scale(.92); transition: opacity .5s, transform .6s cubic-bezier(.5,0,.8,.4); }
  @keyframes p10-drop { from { transform: translateY(-120px) scale(.4) rotate(-12deg); opacity: 0; } to { transform: none; opacity: 1; } }
  @keyframes p10-up { from { transform: translateY(16px); opacity: 0; } to { transform: none; opacity: .92; } }`;

  const rocketSVG = (
    label,
  ) => `<svg width="120" height="280" viewBox="0 0 120 280" style="position:absolute;inset:0;overflow:visible">
    <defs>
      <linearGradient id="p10b" x1="0" x2="1"><stop offset="0" stop-color="#8e95a3"/><stop offset=".3" stop-color="#fff"/><stop offset=".62" stop-color="#e3e7ee"/><stop offset="1" stop-color="#7d8494"/></linearGradient>
      <linearGradient id="p10r" x1="0" x2="1"><stop offset="0" stop-color="#8a0d1d"/><stop offset=".35" stop-color="#ff5a6a"/><stop offset=".7" stop-color="#df2a3e"/><stop offset="1" stop-color="#6d0915"/></linearGradient>
      <linearGradient id="p10n" x1="0" x2="1"><stop offset="0" stop-color="#2c303a"/><stop offset=".45" stop-color="#8a909c"/><stop offset="1" stop-color="#23262e"/></linearGradient>
      <radialGradient id="p10w" cx=".38" cy=".35" r=".75"><stop offset="0" stop-color="#d8f3ff"/><stop offset=".45" stop-color="#3aa0ff"/><stop offset="1" stop-color="#0a2f73"/></radialGradient>
      <clipPath id="p10c"><path d="M60 6C90 38 97 92 96 202H24C23 92 30 38 60 6Z"/></clipPath>
    </defs>
    <path d="M27 136C8 154 2 196 5 236L27 214Z" fill="url(#p10r)"/>
    <path d="M93 136C112 154 118 196 115 236L93 214Z" fill="url(#p10r)"/>
    <path d="M38 202H82L89 224H31Z" fill="url(#p10n)"/>
    <path d="M60 6C90 38 97 92 96 202H24C23 92 30 38 60 6Z" fill="url(#p10b)"/>
    <g clip-path="url(#p10c)">
      <rect x="0" y="0" width="120" height="74" fill="url(#p10r)"/>
      <rect x="0" y="74" width="120" height="5" fill="#2a2f3b" opacity=".55"/>
      <rect x="0" y="190" width="120" height="12" fill="url(#p10r)"/>
    </g>
    <circle cx="60" cy="102" r="17" fill="#c9ced8" stroke="#7d8494" stroke-width="1.5"/>
    <circle cx="60" cy="102" r="12" fill="url(#p10w)"/>
    <ellipse cx="55.5" cy="97" rx="4.5" ry="2.8" fill="#fff" opacity=".85" transform="rotate(-30 55.5 97)"/>
    <text transform="translate(64.5 156) rotate(-90)" text-anchor="middle" font-family="ui-monospace,Menlo,monospace" font-size="12.5" font-weight="800" fill="#1d2433" letter-spacing=".5">${esc(label)}</text>
    <rect x="56" y="196" width="8" height="40" rx="3" fill="#7a0b19"/>
  </svg>`;

  // Brass-ish: three detuned saws per note through a lowpass that swells open.
  function brass(freq, when, dur, gain, out) {
    const a = H.ac(),
      t = a.currentTime + when,
      f = a.createBiquadFilter(),
      g = a.createGain();
    f.type = "lowpass";
    f.Q.value = 1.5;
    f.frequency.setValueAtTime(400, t);
    f.frequency.exponentialRampToValueAtTime(3200, t + 0.09);
    f.frequency.exponentialRampToValueAtTime(1500, t + dur);
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(gain, t + 0.04);
    g.gain.setValueAtTime(gain, t + dur * 0.7);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    f.connect(g);
    g.connect(out);
    [-11, 0, 9].forEach((d) => {
      const o = a.createOscillator();
      o.type = "sawtooth";
      o.frequency.value = freq;
      o.detune.value = d;
      o.connect(f);
      o.start(t);
      o.stop(t + dur + 0.05);
    });
  }
  function fanfare() {
    const out = H.dest(0.35),
      m = (n) => 440 * Math.pow(2, (n - 69) / 12);
    [0, 0.13, 0.26].forEach((w) => [67, 72].forEach((n) => brass(m(n), w, 0.12, 0.07, out)));
    [60, 64, 67, 72, 76].forEach((n) => brass(m(n), 0.4, 1.9, 0.065, out));
    brass(m(48), 0.4, 1.9, 0.09, out);
    H.tone({ when: 0.4, freq: m(84), type: "sine", gain: 0.05, dur: 1.6, wet: 0.5 });
    H.noise({ when: 0.4, dur: 1.8, gain: 0.12, filter: "highpass", freq: 6000, wet: 0.4 }); // cymbal
    H.tone({ when: 0.4, freq: 120, glideTo: 45, dur: 0.5, gain: 0.5, wet: 0.1 });
  }
  // A looping rumble we can swell and kill.
  function rumble() {
    const a = H.ac(),
      t = a.currentTime,
      src = a.createBufferSource(),
      lp = a.createBiquadFilter(),
      bp = a.createBiquadFilter(),
      g = a.createGain(),
      cg = a.createGain();
    src.buffer = H.noiseBuffer();
    src.loop = true;
    lp.type = "lowpass";
    lp.frequency.value = 90;
    lp.Q.value = 0.8;
    bp.type = "bandpass";
    bp.frequency.value = 900;
    bp.Q.value = 0.6;
    const sub = a.createOscillator();
    sub.frequency.value = 42;
    const sg = a.createGain();
    sg.gain.value = 0.25;
    src.connect(lp);
    lp.connect(g);
    src.connect(bp);
    bp.connect(cg);
    cg.connect(g);
    sub.connect(sg);
    sg.connect(g);
    g.connect(H.dest(0.2));
    const at = (s) => t + s;
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(0.25, at(T.ignite));
    g.gain.exponentialRampToValueAtTime(1.1, at(T.lift + 0.3));
    g.gain.setValueAtTime(1.1, at(T.lift + 0.8));
    g.gain.exponentialRampToValueAtTime(0.0001, at(T.shipped + 1));
    cg.gain.setValueAtTime(0.0001, t);
    cg.gain.exponentialRampToValueAtTime(0.25, at(T.lift + 0.4));
    cg.gain.exponentialRampToValueAtTime(0.02, at(T.shipped));
    lp.frequency.setValueAtTime(70, t);
    lp.frequency.exponentialRampToValueAtTime(420, at(T.lift + 0.5));
    src.start(t);
    sub.start(t);
    const end = T.shipped + 1.1;
    src.stop(at(end));
    sub.stop(at(end));
    return {
      kill() {
        try {
          src.stop();
          sub.stop();
          g.disconnect();
        } catch {}
      },
    };
  }

  function launch(H, who, name) {
    name = String(name || "")
      .trim()
      .slice(0, 12);
    const timers = [],
      fxEls = [],
      shakeEls = [H.screen, H.fx, H.canvas];
    let raf = 0,
      alive = true,
      snd = null;
    const at = (s, fn) => timers.push(setTimeout(() => alive && fn(), s * 1000));
    const add = (html) => {
      const e = H.add(html);
      fxEls.push(e);
      return e;
    };
    const vig = add(`<div class="p10-vig"></div>`);
    const glow = add(`<div class="p10-glow"></div>`);
    const rocket = add(
      `<div class="p10-rocket"><div class="p10-flame"><i></i><i></i></div>${rocketSVG(name || "HOPP")}</div>`,
    );
    const flame = rocket.querySelector(".p10-flame");
    const cap = add(`<div class="p10-cap">T-MINUS${name ? ` · ${esc(name.toUpperCase())}` : ""}</div>`);
    requestAnimationFrame(() => {
      vig.classList.add("on");
      cap.classList.add("on");
    });

    const t0 = performance.now(),
      scale = H.clamp(innerHeight / 900, 0.75, 1.3);
    const easeBack = (p) => 1 + 2.2 * Math.pow(p - 1, 3) + 1.2 * Math.pow(p - 1, 2);
    let lastPuff = 0;
    const puff = (x, y, big, warm) => {
      const s = big ? H.rand(90, 170) : H.rand(36, 70),
        life = big ? H.rand(2400, 3400) : H.rand(1500, 2300);
      const e = add(`<div class="p10-smoke${warm ? " warm" : ""}" style="width:${s}px;height:${s}px"></div>`);
      const dx = big ? H.pick([-1, 1]) * H.rand(140, innerWidth * 0.42) : H.rand(-40, 40),
        dy = big ? H.rand(-80, 10) : H.rand(10, 60);
      const st = `translate(${x - s / 2}px,${y - s / 2}px)`;
      const a = e.animate?.(
        [
          { transform: `${st} scale(.3)`, opacity: 0.95 },
          {
            transform: `${st} translate(${dx * 0.6}px,${dy * 0.6}px) scale(${big ? 2.2 : 1.6})`,
            opacity: 0.8,
            offset: 0.4,
          },
          { transform: `${st} translate(${dx}px,${dy}px) scale(${big ? 3.2 : 2.6})`, opacity: 0 },
        ],
        { duration: life, easing: "cubic-bezier(.15,.7,.3,1)", fill: "forwards" },
      );
      if (a) a.onfinish = () => e.remove();
      else setTimeout(() => e.remove(), life);
    };
    let shakeAmt = 0;
    const frame = (now) => {
      if (!alive) return;
      const t = (now - t0) / 1000;
      let dy = 0,
        jx = 0,
        sx = 1;
      if (t < 0.7) dy = 420 * (1 - easeBack(H.clamp(t / 0.7, 0, 1)));
      if (t >= T.ignite && t < T.lift) {
        const k = (t - T.ignite) / (T.lift - T.ignite);
        jx = H.rand(-1.5, 1.5) * k;
        dy = 4 * k;
        sx = 1 + 0.03 * k; // squat before lift-off
        flame.style.transform = `scale(${0.55 + 0.45 * k}, ${0.4 + 0.6 * k})`;
        shakeAmt = 2 + 7 * k;
        if (now - lastPuff > 55) {
          lastPuff = now;
          const r = rocket.getBoundingClientRect();
          for (let i = 0; i < 3; i++)
            puff(r.left + r.width / 2 + H.rand(-30, 30), innerHeight - H.rand(10, 60), true, k < 0.3 && i === 0);
        }
      } else if (t >= T.lift) {
        const k = H.clamp((t - T.lift) / T.flight, 0, 1);
        dy = -(innerHeight + 360 * scale) * Math.pow(k, 2.4);
        jx = H.rand(-1, 1) * (1 - k);
        flame.style.transform = `scale(${1 + 0.15 * k}, ${1.25 + 1.1 * k})`;
        shakeAmt = t < T.lift + 0.9 ? 9 : Math.max(0, 9 * (1 - (t - T.lift - 0.9) / 1.2));
        glow.style.opacity = Math.max(0, 1 - k * 2.2);
        if (k < 1 && now - lastPuff > 38) {
          lastPuff = now;
          const r = rocket.getBoundingClientRect();
          const y = r.top + 214 * scale;
          if (y < innerHeight + 40) puff(r.left + r.width / 2 + H.rand(-8, 8), y + H.rand(0, 30), k < 0.12, false);
        }
      }
      if (t >= T.ignite && t < T.lift)
        glow.style.opacity = 0.4 + 0.6 * ((t - T.ignite) / (T.lift - T.ignite)) + H.rand(-0.1, 0.1);
      rocket.style.transform = `translate(${jx}px, ${dy}px) scale(${scale * sx}, ${scale / sx})`;
      const sh = shakeAmt;
      shakeEls.forEach((e) => (e.style.translate = sh > 0.2 ? `${H.rand(-sh, sh)}px ${H.rand(-sh, sh)}px` : ""));
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);

    // Countdown: 3, 2, 1 with blips; the last one higher.
    ["3", "2", "1"].forEach((n, i) => {
      const w = T.three + i * 0.8;
      H.tone({
        when: w,
        freq: i < 2 ? 880 : 1760,
        type: "square",
        dur: i < 2 ? 0.11 : 0.32,
        gain: 0.08,
        attack: 0.003,
        wet: 0.2,
      });
      H.tone({ when: w, freq: i < 2 ? 1760 : 3520, type: "sine", dur: 0.08, gain: 0.03 });
      at(w, () => {
        const el = add(`<div class="p10-num">${n}</div>`),
          ring = add(`<div class="p10-ring"></div>`);
        el.animate?.(
          [
            { transform: "translate(-50%,-50%) scale(1.7)", opacity: 0, filter: "blur(12px)" },
            { transform: "translate(-50%,-50%) scale(1)", opacity: 1, filter: "blur(0)", offset: 0.22 },
            { transform: "translate(-50%,-50%) scale(.94)", opacity: 1, offset: 0.75 },
            { transform: "translate(-50%,-50%) scale(.6)", opacity: 0 },
          ],
          { duration: 780, easing: "cubic-bezier(.2,.9,.3,1)", fill: "forwards" },
        );
        ring.animate?.(
          [
            { transform: "scale(.4)", opacity: 1 },
            { transform: "scale(2.6)", opacity: 0 },
          ],
          { duration: 750, easing: "cubic-bezier(.1,.7,.3,1)", fill: "forwards" },
        );
        setTimeout(() => {
          el.remove();
          ring.remove();
        }, 800);
      });
    });
    snd = rumble();
    at(T.ignite, () => {
      cap.classList.remove("on");
      H.noise({ dur: 0.6, gain: 0.5, filter: "lowpass", freq: 3000, sweepTo: 200, attack: 0.01, wet: 0.2 }); // whoomp
      H.tone({ freq: 110, glideTo: 38, dur: 0.6, gain: 0.5, wet: 0.1 });
      H.flash("#ffb36b", 300, 0.18);
    });
    at(T.lift, () =>
      H.noise({ dur: 1.6, gain: 0.18, filter: "bandpass", freq: 400, sweepTo: 3000, q: 0.7, attack: 0.5, wet: 0.25 }),
    ); // whoosh
    at(T.shipped, () => {
      vig.classList.remove("on");
      fanfare();
      H.flash("#fff", 450, 0.35);
      const word = [..."SHIPPED"].map((c, i) => `<span style="animation-delay:${i * 45}ms">${c}</span>`).join("");
      const title =
        add(`<div class="p10-title"><h1>${word} <span class="emo" style="animation-delay:340ms">🚀</span></h1>
        <p>${name ? `<code>${esc(name)}</code> is out · ` : ""}${
          who === H.me ?
            name ? "you"
            : "You"
          : esc(who.name)
        } shipped it</p></div>`);
      const c = H.centerOf(title.querySelector("h1"));
      H.emit({
        x: c.x,
        y: c.y,
        count: 120,
        angle: -90,
        spread: 360,
        speed: [6, 18],
        gravity: 0.3,
        life: [90, 150],
        size: [7, 12],
      });
      H.emit({
        x: c.x,
        y: c.y,
        count: 30,
        angle: 0,
        spread: 360,
        speed: [3, 10],
        gravity: 0.1,
        drag: 0.95,
        life: [50, 90],
        size: [10, 16],
        shape: "star",
        colors: ["#ffd166", "#fff", "#ffe58a"],
        glow: true,
      });
      [
        [0, -62],
        [innerWidth, -118],
      ].forEach(([x, ang], i) =>
        setTimeout(
          () =>
            H.emit({
              x,
              y: innerHeight + 10,
              count: 90,
              angle: ang,
              spread: 26,
              speed: [16, 26],
              gravity: 0.32,
              life: [120, 190],
              size: [7, 12],
            }),
          120 + i * 80,
        ),
      );
      at(T.done - T.shipped - 0.8, () => title.classList.add("out"));
    });
    at(T.done, () => stop());
    function stop() {
      if (!alive) return;
      alive = false;
      cancelAnimationFrame(raf);
      timers.forEach(clearTimeout);
      snd && snd.kill();
      shakeEls.forEach((e) => (e.style.translate = ""));
      fxEls.forEach((e) => e.remove());
      if (run === me) run = null;
      setBtn(false);
    }
    const me = { stop };
    run = me;
    setBtn(true);
    H.banner(who, name ? `is shipping ${esc(name)}` : "is shipping it", "🚀", 3000);
  }

  let btn = null;
  const setBtn = (on) => btn && (btn.style.opacity = on ? ".55" : "");

  H.register({
    n: 10,
    title: "Ship it!",
    pitch:
      "For release days. A mission-control countdown, then a rocket launches up through everyone's screen in a cloud of smoke and lands on SHIPPED with a fanfare.",
    how: "Click 🚀 to launch. Click the little caret next to it to name what shipped (it goes on the rocket and the title).",
    mount(H) {
      H.css("p10", CSS);
      const self = this;
      const wrap = H.el(`<span class="p10-split"></span>`);
      btn = H.makeButton("🚀", "Ship it");
      const caret = H.el(`<button class="p10-caret" title="Name what shipped">▲</button>`);
      wrap.append(btn, caret);
      H.slot.appendChild(wrap);
      const pop = H.el(
        `<div class="p10-pop"><b>🚀 What are we shipping?</b><input placeholder="v2.0" maxlength="12"><button class="p10-go">Launch</button></div>`,
      );
      H.callWin.appendChild(pop);
      const input = pop.querySelector("input");
      const open = (v) => {
        pop.classList.toggle("open", v);
        if (v) setTimeout(() => input.focus(), 60);
      };
      const go = (name) => {
        open(false);
        if (!run) self.play(H, H.me, name);
      };
      btn.onclick = () => go("");
      caret.onclick = () => open(!pop.classList.contains("open"));
      pop.querySelector(".p10-go").onclick = () => go(input.value);
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          go(input.value);
        }
        if (e.key === "Escape") open(false);
      });
      const outside = (e) => {
        if (!pop.contains(e.target) && !wrap.contains(e.target)) open(false);
      };
      document.addEventListener("pointerdown", outside);
      H.onLeave(() => {
        document.removeEventListener("pointerdown", outside);
        pop.remove();
        if (run) run.stop();
        run = null;
        btn = null;
      });
    },
    play(H, who, name) {
      if (run) run.stop();
      launch(H, who, name);
    },
  });
})();
