// @ts-nocheck
// 9. Disco mode: a 6-second party. Mirror ball drops, lights sweep, a 124 BPM groove plays, tiles bounce on the beat.
(() => {
  const BPM = 124,
    BEAT = 60 / BPM,
    S16 = BEAT / 4,
    BEATS = 12; // 3 bars, then the cymbal
  const COLORS = ["#ff2bd6", "#22e4ff", "#ffe14d", "#6dff5c", "#9b5cff", "#ff7a2b"];
  const mtof = (m) => 440 * Math.pow(2, (m - 69) / 12);
  let party = null;

  const CSS = `
  .p9-dim { position: absolute; inset: 0; z-index: 8; pointer-events: none; opacity: 0; transition: opacity .5s ease;
    background: radial-gradient(120% 90% at 50% 10%, rgba(30,6,60,.7), rgba(4,0,14,.9)); }
  .p9-dim.on { opacity: 1; }
  .p9-layer { position: fixed; inset: 0; overflow: hidden; opacity: 0; transition: opacity .45s ease; }
  .p9-layer.on { opacity: 1; }
  .p9-beam { position: absolute; width: 230px; height: 160vh; transform-origin: 50% 0; margin-left: -115px; opacity: .55;
    clip-path: polygon(47% 0, 53% 0, 100% 100%, 0 100%); filter: blur(1.5px); }
  .p9-spot { position: absolute; width: 340px; height: 340px; margin: -170px 0 0 -170px; border-radius: 50%; opacity: .5; }
  .p9-refl { position: absolute; left: 0; top: 0; width: 0; height: 0; }
  .p9-dot { position: absolute; border-radius: 50%; background: #fff; animation: p9-tw 1.2s ease-in-out infinite alternate; }
  @keyframes p9-tw { from { opacity: .15; } to { opacity: 1; } }
  .p9-rig { position: fixed; left: 50%; top: 0; width: 150px; margin-left: -75px; transform-origin: 50% 0; }
  .p9-string { width: 2px; height: 96px; margin: 0 auto; background: linear-gradient(#777, #ccc); }
  .p9-cap { width: 22px; height: 12px; margin: -2px auto -4px; border-radius: 4px 4px 2px 2px; background: linear-gradient(90deg,#666,#ddd,#777); position: relative; z-index: 1; }
  .p9-ball { position: relative; width: 150px; height: 150px; border-radius: 50%; overflow: hidden;
    box-shadow: 0 0 60px rgba(255,255,255,.35), 0 0 140px rgba(160,120,255,.45); }
  .p9-facets { position: absolute; inset: 0; background-size: 300px 150px; background-repeat: repeat-x; animation: p9-spin 3.6s linear infinite; }
  @keyframes p9-spin { to { background-position: -300px 0; } }
  .p9-shade { position: absolute; inset: 0; border-radius: 50%;
    background: radial-gradient(circle at 34% 28%, rgba(255,255,255,.75) 0, rgba(255,255,255,.1) 22%, rgba(0,0,0,0) 45%, rgba(10,0,30,.55) 78%, rgba(0,0,0,.92) 100%); }
  .p9-glint { position: absolute; width: 26px; height: 26px; margin: -13px; pointer-events: none;
    background: radial-gradient(circle, #fff 0 8%, rgba(255,255,255,0) 60%); animation: p9-gl 0.9s ease-in-out infinite; }
  .p9-glint::before, .p9-glint::after { content: ""; position: absolute; left: 50%; top: 0; width: 2px; height: 100%; margin-left: -1px;
    background: linear-gradient(transparent, #fff, transparent); }
  .p9-glint::after { transform: rotate(90deg); }
  @keyframes p9-gl { 0%,100% { transform: scale(0) rotate(0); } 50% { transform: scale(1) rotate(45deg); } }
  .h-react-btn.p9-on { animation: p9-btn .97s linear infinite; box-shadow: 0 0 0 2px #fff, 0 0 18px #ff2bd6; }
  @keyframes p9-btn { 0% { filter: hue-rotate(0); } 100% { filter: hue-rotate(360deg); } }`;

  // A faceted mirror-ball texture: a grid of slightly different greys, with a few bright and tinted tiles.
  function facetTexture() {
    let r = "";
    for (let y = 0; y < 15; y++)
      for (let x = 0; x < 30; x++) {
        const k = Math.random();
        const fill =
          k > 0.93 ? "#ffffff"
          : k > 0.86 ? H.pick(["#ffd6f6", "#c8f4ff", "#fff3b8", "#d9c8ff"])
          : `hsl(240,${8 + Math.random() * 12}%,${35 + Math.random() * 45}%)`;
        r += `<rect x="${x * 10 + 0.6}" y="${y * 10 + 0.6}" width="8.8" height="8.8" fill="${fill}"/>`;
      }
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="300" height="150"><rect width="300" height="150" fill="#1b1830"/>${r}</svg>`;
    return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
  }

  // ---------- Sound ----------
  function kick(t, out) {
    const a = H.ac(),
      o = a.createOscillator(),
      g = a.createGain();
    o.frequency.setValueAtTime(170, t);
    o.frequency.exponentialRampToValueAtTime(48, t + 0.11);
    g.gain.setValueAtTime(1, t);
    g.gain.exponentialRampToValueAtTime(0.0001, t + 0.42);
    o.connect(g);
    g.connect(out);
    o.start(t);
    o.stop(t + 0.45);
  }
  function saw(t, freq, dur, gain, cutoff, out, q = 6, env = 4) {
    const a = H.ac(),
      o = a.createOscillator(),
      f = a.createBiquadFilter(),
      g = a.createGain();
    o.type = "sawtooth";
    o.frequency.value = freq;
    f.type = "lowpass";
    f.Q.value = q;
    f.frequency.setValueAtTime(cutoff * env, t);
    f.frequency.exponentialRampToValueAtTime(cutoff, t + dur * 0.8);
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(gain, t + 0.006);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    o.connect(f);
    f.connect(g);
    g.connect(out);
    o.start(t);
    o.stop(t + dur + 0.05);
  }
  function cymbal(when = 0) {
    const out = H.dest(0.45);
    H.noise({ when, dur: 2.4, gain: 0.32, filter: "highpass", freq: 5200, q: 0.5, attack: 0.003, to: out });
    H.noise({ when, dur: 1.2, gain: 0.18, filter: "bandpass", freq: 9000, q: 0.8, to: out });
    [567, 802, 1131, 1537, 2213].forEach((f) =>
      H.tone({ when, freq: f * 2.3, type: "square", gain: 0.012, dur: 1.1, to: out }),
    );
  }
  // Schedules the whole groove at t0 into two buses we can cut instantly.
  function groove(t0, dry, wet) {
    const bass = [
      [0, 45, 2],
      [3, 57, 1],
      [6, 45, 1],
      [8, 48, 2],
      [10, 50, 1],
      [11, 52, 1],
      [14, 57, 1],
    ];
    const fill = [
      [0, 45, 2],
      [3, 57, 1],
      [6, 45, 1],
      [8, 55, 1],
      [10, 57, 1],
      [12, 60, 1],
      [13, 59, 1],
      [14, 57, 1],
      [15, 55, 1],
    ];
    const chords = [
      [69, 72, 76, 79],
      [74, 77, 81, 84],
      [69, 72, 76, 79],
    ];
    for (let bar = 0; bar < BEATS / 4; bar++) {
      const tb = t0 + bar * 4 * BEAT;
      for (let b = 0; b < 4; b++) {
        const t = tb + b * BEAT;
        kick(t, dry);
        H.noise({
          when: t + BEAT / 2 - H.now(),
          dur: 0.2,
          gain: 0.16,
          filter: "highpass",
          freq: 7500,
          q: 0.7,
          to: dry,
        }); // open hat
        [1, 3].forEach((s) =>
          H.noise({ when: t + s * S16 - H.now(), dur: 0.035, gain: 0.05, filter: "highpass", freq: 9000, to: dry }),
        );
        if (b === 1 || b === 3) {
          // clap: three quick slaps and a tail
          [0, 0.011, 0.022].forEach((d) =>
            H.noise({ when: t + d - H.now(), dur: 0.03, gain: 0.32, filter: "bandpass", freq: 1300, q: 1.4, to: wet }),
          );
          H.noise({ when: t + 0.03 - H.now(), dur: 0.22, gain: 0.16, filter: "bandpass", freq: 1100, q: 1, to: wet });
        }
      }
      (bar === 2 ? fill : bass).forEach(([s, m, len]) => {
        saw(tb + s * S16, mtof(m), len * S16 * 0.95, 0.2, 260, dry, 9, 5);
        H.tone({
          when: tb + s * S16 - H.now(),
          freq: mtof(m - 12),
          type: "sine",
          gain: 0.22,
          dur: len * S16 * 0.95,
          to: dry,
        });
      });
      [6, 14].forEach((s) =>
        chords[bar].forEach((m, i) =>
          [-9, 9].forEach((d) => saw(tb + s * S16, mtof(m) * Math.pow(2, d / 1200), 0.2, 0.035, 1400, wet, 2, 3)),
        ),
      );
    }
  }

  function start(H, who) {
    const a = H.ac(),
      t0 = a.currentTime + 0.08,
      tEnd = t0 + BEATS * BEAT;
    const dry = H.dest(0.06),
      wet = H.dest(0.32);
    const timers = [],
      tiles = Object.values(H.tiles);
    let raf = 0,
      ended = false;
    // Slide-whistle drop, then a springy "doing" as the ball lands.
    H.tone({ freq: 1500, glideTo: 380, dur: 0.5, type: "sine", gain: 0.07, wet: 0.3 });
    H.tone({ when: 0.55, freq: 300, glideTo: 520, dur: 0.25, type: "triangle", gain: 0.06, wet: 0.2 });
    groove(t0, dry, wet);

    const dim = H.el(`<div class="p9-dim"></div>`);
    H.screen.appendChild(dim);
    const layer = H.add(`<div class="p9-layer"></div>`);
    const ballY = 26 + 96 + 8 + 75; // menubar + string + cap + radius
    const bx = innerWidth / 2;
    const beams = COLORS.map((c, i) => {
      const el = H.el(
        `<div class="p9-beam" style="left:${bx}px;top:${ballY}px;background:linear-gradient(${c}cc, ${c}00 85%)"></div>`,
      );
      layer.appendChild(el);
      const from = -70 + i * 26,
        to = from + (i % 2 ? -50 : 50);
      el.animate?.([{ transform: `rotate(${from}deg)` }, { transform: `rotate(${to}deg)` }], {
        duration: BEAT * 4000,
        direction: "alternate",
        iterations: Infinity,
        easing: "cubic-bezier(.45,0,.55,1)",
      });
      return el;
    });
    COLORS.slice(0, 5).forEach((c, i) => {
      const el = H.el(`<div class="p9-spot" style="background:radial-gradient(circle, ${c}cc, ${c}00 68%)"></div>`);
      layer.appendChild(el);
      const pts = Array.from({ length: 5 }, () => ({ left: `${H.rand(5, 95)}%`, top: `${H.rand(20, 95)}%` }));
      el.animate?.([...pts, pts[0]], {
        duration: BEAT * 8000 * (1 + i * 0.15),
        iterations: Infinity,
        easing: "ease-in-out",
      });
    });
    const refl = H.el(`<div class="p9-refl" style="left:${bx}px;top:${ballY}px"></div>`);
    const maxR = Math.hypot(innerWidth, innerHeight);
    for (let i = 0; i < 90; i++) {
      const r = H.rand(120, maxR),
        ang = H.rand(0, Math.PI * 2),
        s = H.rand(3, 9);
      const c = Math.random() < 0.25 ? H.pick(COLORS) : "#fff";
      refl.appendChild(
        H.el(
          `<i class="p9-dot" style="left:${Math.cos(ang) * r}px;top:${Math.sin(ang) * r}px;width:${s}px;height:${s}px;background:${c};box-shadow:0 0 ${s * 2}px ${c};animation-delay:${-H.rand(0, 1.2)}s"></i>`,
        ),
      );
    }
    layer.appendChild(refl);
    refl.animate?.([{ transform: "rotate(0)" }, { transform: "rotate(360deg)" }], {
      duration: 14000,
      iterations: Infinity,
    });

    const rig = H.add(`<div class="p9-rig"><div class="p9-string"></div><div class="p9-cap"></div><div class="p9-ball">
      <div class="p9-facets" style="background-image:${facetTexture()}"></div><div class="p9-shade"></div>
      <i class="p9-glint" style="left:38%;top:30%"></i><i class="p9-glint" style="left:68%;top:58%;animation-delay:-.45s"></i>
      <i class="p9-glint" style="left:30%;top:70%;animation-delay:-.2s;animation-duration:1.3s"></i></div></div>`);
    const drop = rig.animate?.(
      [
        { transform: "translateY(-300px)", offset: 0 },
        { transform: "translateY(22px) rotate(2deg)", offset: 0.55 },
        { transform: "translateY(-9px) rotate(-1.5deg)", offset: 0.75 },
        { transform: "translateY(3px) rotate(.6deg)", offset: 0.9 },
        { transform: "translateY(0)", offset: 1 },
      ],
      { duration: 900, easing: "cubic-bezier(.3,.6,.4,1)", fill: "forwards" },
    );
    requestAnimationFrame(() => {
      dim.classList.add("on");
    });
    timers.push(setTimeout(() => layer.classList.add("on"), 450));

    // Everything visual on the beat follows the audio clock.
    const frame = () => {
      const now = a.currentTime,
        pos = (now - t0) / BEAT;
      if (pos >= 0 && !ended) {
        const pulse = Math.exp(-(pos % 1) * 5),
          half = Math.exp(-((pos + 0.5) % 1) * 7);
        tiles.forEach((t, i) => {
          const h = (now * 140 + i * 120) % 360,
            up = (Math.floor(pos) + i) % 2 ? 1 : -1;
          t.style.transform = `translateY(${-9 * pulse}px) scale(${1 + 0.05 * pulse}) rotate(${up * 1.6 * pulse}deg)`;
          t.style.boxShadow = `0 0 0 3px hsl(${h},100%,62%), 0 0 ${12 + 34 * pulse}px hsl(${h},100%,60%)`;
          t.style.filter = `saturate(${1.2 + pulse * 0.6}) hue-rotate(${(h / 6) | 0}deg)`;
        });
        beams.forEach((b, i) => (b.style.opacity = 0.3 + 0.45 * (i % 2 ? half : pulse)));
        dim.style.opacity = 0.9 + 0.1 * (1 - pulse);
      }
      if (now >= tEnd && !ended) end(false);
      if (!ended) raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);

    const resetTiles = () =>
      tiles.forEach((t) => {
        t.style.transform = t.style.boxShadow = t.style.filter = "";
        t.style.transition = "";
      });
    const removeAll = () => {
      dim.remove();
      layer.remove();
      rig.remove();
      resetTiles();
    };
    function end(early) {
      if (ended) return;
      ended = true;
      cancelAnimationFrame(raf);
      const n = H.now();
      if (early)
        [dry, wet].forEach((g) => {
          g.gain.cancelScheduledValues(n);
          g.gain.setValueAtTime(g.gain.value, n);
          g.gain.linearRampToValueAtTime(0, n + 0.06);
        });
      setTimeout(
        () => {
          try {
            dry.disconnect();
            wet.disconnect();
          } catch {}
        },
        early ? 200 : 900,
      );
      if (early) {
        cymbal(0.02);
        kick(n + 0.02, H.dest(0.1));
      } else {
        cymbal(Math.max(0, tEnd - n));
        kick(tEnd, H.dest(0.1));
      }
      H.tone({ when: 0.15, freq: 300, glideTo: 2400, dur: 0.45, type: "sine", gain: 0.06, wet: 0.3 }); // zip up
      tiles.forEach((t) => {
        t.style.transition = "transform .6s cubic-bezier(.2,1.6,.4,1), box-shadow 1s, filter 1s";
        t.style.transform = "";
        t.style.boxShadow = "";
        t.style.filter = "";
      });
      H.flash("#fff", 400, 0.25);
      drop?.cancel?.();
      rig.animate?.(
        [
          { transform: "translateY(0)" },
          { transform: "translateY(14px)", offset: 0.25 },
          { transform: "translateY(-420px)" },
        ],
        { duration: 650, delay: 120, easing: "cubic-bezier(.5,0,.9,.4)", fill: "forwards" },
      );
      layer.classList.remove("on");
      dim.classList.remove("on");
      dim.style.transition = "opacity .9s ease";
      dim.style.opacity = "";
      timers.push(setTimeout(removeAll, 1100));
      if (party === me) party = null;
      setButton(false);
    }
    const me = {
      end,
      kill() {
        ended = true;
        cancelAnimationFrame(raf);
        timers.forEach(clearTimeout);
        try {
          dry.disconnect();
          wet.disconnect();
        } catch {}
        removeAll();
      },
    };
    party = me;
    setButton(true);
    H.banner(who, "started disco mode", "🪩", 2600);
  }

  let btn = null;
  const setButton = (on) => btn && btn.classList.toggle("p9-on", on);

  H.register({
    n: 9,
    title: "Disco mode",
    pitch:
      "Six seconds of party. A mirror ball drops onto everyone's screen, the lights go down, the beams sweep and the call bounces to a 124 BPM groove.",
    how: "Click 🪩 to start the party. Click it again to end it early.",
    mount(H) {
      H.css("p9", CSS);
      btn = H.makeButton("🪩", "Disco mode");
      btn.onclick = () => (party ? party.end(true) : this.play(H, H.me));
      H.slot.appendChild(btn);
      H.onLeave(() => {
        if (party) party.kill();
        party = null;
        btn = null;
      });
    },
    play(H, who) {
      if (party) party.kill();
      start(H, who);
      if (party) party.by = who;
    },
    /** Whoever started the party can end it early. */
    stop(H, who) {
      if (party && party.by === who) party.end(true);
    },
  });
})();
