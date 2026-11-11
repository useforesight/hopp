// @ts-nocheck
// Shared helpers for the reactions. Everything lives on window.H.
//
// The effects play on a transparent layer over the whole screen. The call window isn't
// part of this page, so `#callwin` is an invisible stand-in where it is on screen: effects
// that come from someone's tile come from there.
(() => {
  const H = (window.H = {});
  const $ = (s, r = document) => r.querySelector(s);

  H.screen = $("#screen");
  H.callWin = $("#callwin");
  // Each effect's own button and picker are made here and never shown: the tray sends them.
  H.slot = document.createElement("div");
  H.controls = H.slot;
  H.fx = $("#fx");
  H.canvas = $("#fx-canvas");
  // Where the slow clap's meter shows, above the call window.
  H.meterHost = $("#meter-host");
  const tile = $("#callwin .tile");
  // Everyone's tile is the call window.
  H.tiles = new Proxy(
    {},
    {
      get: (_, key) => (typeof key === "string" ? tile : undefined),
      ownKeys: () => ["call"],
      getOwnPropertyDescriptor: () => ({ enumerable: true, configurable: true, value: tile }),
    },
  );
  // Everyone in the call, by identity; see `H.person`.
  H.people = {};
  H.me = null;

  /** Someone in the call, from their identity, name and colour. */
  H.person = (key, full, colour) => {
    const known = H.people[key];
    const name = (full || "").trim();
    const sameColour = !colour || (known && known.colour.join() === colour.join());
    if (known && (!name || known.full === name) && sameColour) return known;
    const words = name.split(/\s+/).filter(Boolean);
    const initials = (
      words.length > 1 ?
        words[0][0] + words[words.length - 1][0]
      : (words[0] || "?").slice(0, 2)).toUpperCase();
    const [r, g, b] = colour || (known && known.colour) || [124, 92, 255];
    const lighter = `rgb(${Math.min(255, r + 70)},${Math.min(255, g + 70)},${Math.min(255, b + 70)})`;
    const person = {
      key,
      full: name || (known && known.full) || "Someone",
      name: words[0] || (known && known.name) || "Someone",
      initials,
      colour: [r, g, b],
      grad: `linear-gradient(135deg,${lighter},rgb(${r},${g},${b}))`,
    };
    H.people[key] = person;
    return person;
  };

  H.wait = (ms) => new Promise((r) => setTimeout(r, ms));
  H.rand = (a, b) => a + Math.random() * (b - a);
  H.pick = (arr) => arr[Math.floor(Math.random() * arr.length)];
  H.clamp = (v, a, b) => Math.max(a, Math.min(b, v));

  /** Make an element from an HTML string. */
  H.el = (html) => {
    const t = document.createElement("template");
    t.innerHTML = html.trim();
    return t.content.firstElementChild;
  };

  /** Add a style block once per id. */
  H.css = (id, text) => {
    if (document.getElementById("css-" + id)) return;
    const s = document.createElement("style");
    s.id = "css-" + id;
    s.textContent = text;
    document.head.appendChild(s);
  };

  /** Centre of an element on screen. */
  H.centerOf = (el) => {
    const r = el.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2, r };
  };

  /** A standard round call-control button for the slot. Returns the element. */
  H.makeButton = (label, title) => {
    const b = H.el(`<button class="ctl h-react-btn" title="${title || ""}">${label}</button>`);
    H.css(
      "react-btn",
      `
      .h-react-btn { all: unset; cursor: pointer; width: 30px; height: 30px; border-radius: 50%; display: grid; place-items: center;
        font-size: 15px; background: linear-gradient(135deg,#ffd166,#ff7ab6 55%,#8b5cf6); box-shadow: 0 2px 10px rgba(255,122,182,.45);
        transition: transform .15s cubic-bezier(.2,1.6,.4,1); flex: none; }
      .h-react-btn:hover { transform: scale(1.12) rotate(-6deg); }
      .h-react-btn:active { transform: scale(.92); }`,
    );
    return b;
  };

  // ---------- Audio ----------
  let ac = null,
    master = null,
    verb = null,
    muted = false;
  /** Sounds off: everything still plays, silently. */
  H.setMuted = (value) => {
    muted = value;
    if (master) master.gain.setTargetAtTime(muted ? 0 : 0.8, ac.currentTime, 0.02);
  };
  H.ac = () => {
    if (ac) return ac;
    ac = new (window.AudioContext || window.webkitAudioContext)();
    const comp = ac.createDynamicsCompressor();
    comp.threshold.value = -14;
    comp.ratio.value = 6;
    comp.attack.value = 0.003;
    comp.release.value = 0.2;
    master = ac.createGain();
    master.gain.value = muted ? 0 : 0.8;
    master.connect(comp);
    comp.connect(ac.destination);
    // A generated hall reverb.
    verb = ac.createConvolver();
    const len = ac.sampleRate * 2.8,
      ir = ac.createBuffer(2, len, ac.sampleRate);
    for (let c = 0; c < 2; c++) {
      const d = ir.getChannelData(c);
      for (let i = 0; i < len; i++) d[i] = (Math.random() * 2 - 1) * Math.pow(1 - i / len, 3.2);
    }
    verb.buffer = ir;
    const vg = ac.createGain();
    vg.gain.value = 0.55;
    verb.connect(vg);
    vg.connect(master);
    return ac;
  };
  /** Where sounds go. Pass {wet:0..1} to send some to the reverb. */
  H.dest = (wet = 0.15) => {
    H.ac();
    const g = ac.createGain();
    g.connect(master);
    if (wet > 0) {
      const s = ac.createGain();
      s.gain.value = wet;
      g.connect(s);
      s.connect(verb);
    }
    return g;
  };
  H.now = () => H.ac().currentTime;
  /** A context made before any sound was asked for can start suspended. */
  H.wake = () => {
    const a = H.ac();
    if (a.state === "suspended") a.resume().catch(() => {});
  };

  /**
   * A tone with an envelope.
   * {freq, type='sine', dur=0.3, gain=0.3, attack=0.005, when=0, glideTo, detune=0, wet=0.15, to}
   */
  H.tone = (o) => {
    const a = H.ac(),
      t = a.currentTime + (o.when || 0),
      dur = o.dur ?? 0.3;
    const osc = a.createOscillator(),
      g = a.createGain();
    osc.type = o.type || "sine";
    osc.frequency.setValueAtTime(o.freq, t);
    if (o.glideTo) osc.frequency.exponentialRampToValueAtTime(o.glideTo, t + dur);
    osc.detune.value = o.detune || 0;
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(o.gain ?? 0.3, t + (o.attack ?? 0.005));
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    osc.connect(g);
    g.connect(o.to || H.dest(o.wet ?? 0.15));
    osc.start(t);
    osc.stop(t + dur + 0.05);
    return { osc, gain: g };
  };

  let noiseBuf = null;
  H.noiseBuffer = () => {
    const a = H.ac();
    if (noiseBuf) return noiseBuf;
    noiseBuf = a.createBuffer(1, a.sampleRate * 3, a.sampleRate);
    const d = noiseBuf.getChannelData(0);
    for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1;
    return noiseBuf;
  };
  /**
   * Filtered noise burst. {dur=0.2, gain=0.3, filter='bandpass', freq=2000, q=1, when=0, attack=0.002, wet=0.15, sweepTo, to}
   */
  H.noise = (o = {}) => {
    const a = H.ac(),
      t = a.currentTime + (o.when || 0),
      dur = o.dur ?? 0.2;
    const src = a.createBufferSource();
    src.buffer = H.noiseBuffer();
    const f = a.createBiquadFilter();
    f.type = o.filter || "bandpass";
    f.frequency.setValueAtTime(o.freq ?? 2000, t);
    f.Q.value = o.q ?? 1;
    if (o.sweepTo) f.frequency.exponentialRampToValueAtTime(o.sweepTo, t + dur);
    const g = a.createGain();
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(o.gain ?? 0.3, t + (o.attack ?? 0.002));
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    src.connect(f);
    f.connect(g);
    g.connect(o.to || H.dest(o.wet ?? 0.15));
    src.start(t, Math.random() * 2);
    src.stop(t + dur + 0.05);
    return { src, filter: f, gain: g };
  };

  // ---------- Particles ----------
  const cv = H.canvas,
    cx = cv.getContext("2d");
  let parts = [],
    running = false,
    dpr = 1;
  const resize = () => {
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    cv.width = innerWidth * dpr;
    cv.height = innerHeight * dpr;
    cv.style.width = innerWidth + "px";
    cv.style.height = innerHeight + "px";
  };
  addEventListener("resize", resize);
  resize();
  H.confettiColors = ["#ff5d8f", "#ffd166", "#06d6a0", "#4cc9f0", "#9b5de5", "#f15bb5", "#fee440", "#ffffff"];

  /**
   * Emit particles. {x, y, count=60, angle=-90 (deg, up), spread=60, speed=[6,14], gravity=0.35, drag=0.985,
   * life=[60,120] frames, size=[6,11], colors, shape: 'rect'|'circle'|'tri'|'star'|'emoji'|'spark', emoji: string|string[],
   * spin=true, wobble=true, glow=false, fade=true}
   */
  // Never more than this many particles at once, so a big show can't bog the machine down.
  const MAX_PARTS = 1400;
  H.emit = (o) => {
    const n = Math.min(o.count ?? 60, MAX_PARTS - parts.length);
    if (n <= 0) return;
    const sp = o.speed || [6, 14],
      lf = o.life || [60, 120],
      sz = o.size || [6, 11];
    for (let i = 0; i < n; i++) {
      const ang = (((o.angle ?? -90) + H.rand(-(o.spread ?? 60) / 2, (o.spread ?? 60) / 2)) * Math.PI) / 180;
      const v = H.rand(sp[0], sp[1]);
      const emoji = Array.isArray(o.emoji) ? H.pick(o.emoji) : o.emoji;
      parts.push({
        x: o.x,
        y: o.y,
        vx: Math.cos(ang) * v,
        vy: Math.sin(ang) * v,
        g: o.gravity ?? 0.35,
        drag: o.drag ?? 0.985,
        life: 0,
        max: H.rand(lf[0], lf[1]),
        size: H.rand(sz[0], sz[1]),
        color: H.pick(o.colors || H.confettiColors),
        shape: o.shape || "rect",
        emoji,
        rot: H.rand(0, Math.PI * 2),
        vr: o.spin === false ? 0 : H.rand(-0.3, 0.3),
        wob: o.wobble === false ? 0 : H.rand(0, Math.PI * 2),
        glow: !!o.glow,
        fade: o.fade !== false,
      });
    }
    if (!running) {
      running = true;
      requestAnimationFrame(tick);
    }
  };
  const star = (r) => {
    cx.beginPath();
    for (let i = 0; i < 10; i++) {
      const a = (i * Math.PI) / 5 - Math.PI / 2,
        rr = i % 2 ? r * 0.45 : r;
      cx.lineTo(Math.cos(a) * rr, Math.sin(a) * rr);
    }
    cx.closePath();
    cx.fill();
  };
  // Glow is a soft dot drawn once per colour and size, then stamped; a live canvas blur per
  // particle is far too slow for a fireworks show.
  const glowSprites = new Map();
  const glowSprite = (color) => {
    let s = glowSprites.get(color);
    if (s) return s;
    s = document.createElement("canvas");
    s.width = s.height = 64;
    const g = s.getContext("2d"),
      rg = g.createRadialGradient(32, 32, 0, 32, 32, 32);
    rg.addColorStop(0, "#ffffff");
    rg.addColorStop(0.18, color);
    rg.addColorStop(0.45, color + "55");
    rg.addColorStop(1, color + "00");
    g.fillStyle = rg;
    g.fillRect(0, 0, 64, 64);
    glowSprites.set(color, s);
    return s;
  };
  // Drawing a colour emoji from the font is slow, so each one is drawn once, large, and reused.
  const emojiSprites = new Map();
  const emojiSprite = (ch) => {
    let s = emojiSprites.get(ch);
    if (s) return s;
    s = document.createElement("canvas");
    s.width = s.height = 128;
    const g = s.getContext("2d");
    g.font = '104px "Apple Color Emoji","Segoe UI Emoji",sans-serif';
    g.textAlign = "center";
    g.textBaseline = "middle";
    g.fillText(ch, 64, 70);
    emojiSprites.set(ch, s);
    return s;
  };
  const hex = (c) =>
    /^#[0-9a-f]{6}$/i.test(c) ? c
    : /^#[0-9a-f]{3}$/i.test(c) ? "#" + [...c.slice(1)].map((x) => x + x).join("")
    : null;
  function tick() {
    cx.setTransform(dpr, 0, 0, dpr, 0, 0);
    cx.clearRect(0, 0, innerWidth, innerHeight);
    parts = parts.filter((p) => p.life < p.max && p.y < innerHeight + 80);
    for (const p of parts) {
      p.life++;
      p.vx *= p.drag;
      p.vy = p.vy * p.drag + p.g;
      p.x += p.vx;
      p.y += p.vy;
      p.rot += p.vr;
      p.wob += 0.1;
      const a = p.fade ? H.clamp(1 - (p.life / p.max) ** 3, 0, 1) : 1;
      cx.save();
      cx.globalAlpha = a;
      cx.translate(p.x, p.y);
      cx.rotate(p.rot);
      const gc = p.glow && hex(p.color);
      if (gc) {
        cx.globalCompositeOperation = "lighter";
        const r = p.size * 2.2;
        cx.drawImage(glowSprite(gc), -r, -r, r * 2, r * 2);
        cx.restore();
        continue;
      }
      cx.fillStyle = p.color;
      const s = p.size;
      if (p.shape === "rect") {
        cx.scale(1, Math.abs(Math.cos(p.wob)));
        cx.fillRect(-s / 2, -s / 4, s, s / 2);
      } else if (p.shape === "circle" || p.shape === "spark") {
        cx.beginPath();
        cx.arc(0, 0, s / 2, 0, Math.PI * 2);
        cx.fill();
      } else if (p.shape === "tri") {
        cx.scale(1, Math.abs(Math.cos(p.wob)));
        cx.beginPath();
        cx.moveTo(0, -s / 2);
        cx.lineTo(s / 2, s / 2);
        cx.lineTo(-s / 2, s / 2);
        cx.fill();
      } else if (p.shape === "star") star(s / 2);
      else if (p.shape === "emoji") {
        const e = emojiSprite(p.emoji || "🎉");
        cx.drawImage(e, -s / 2, -s / 2, s, s);
      }
      cx.restore();
    }
    H.lastFrame = performance.now();
    if (parts.length) requestAnimationFrame(tick);
    else {
      running = false;
      cx.clearRect(0, 0, innerWidth, innerHeight);
    }
  }
  H.clearParticles = () => {
    parts = [];
  };
  H.particleCount = () => parts.length;

  // ---------- Screen effects ----------
  /** Shake an element (default: the whole screen and the overlay). */
  H.shake = (intensity = 10, ms = 500, els) => {
    const targets = els || [H.screen, H.fx, H.canvas];
    const start = performance.now();
    const step = (t) => {
      const k = 1 - (t - start) / ms;
      if (k <= 0) {
        targets.forEach((e) => (e.style.translate = ""));
        return;
      }
      const x = (Math.random() * 2 - 1) * intensity * k,
        y = (Math.random() * 2 - 1) * intensity * k;
      targets.forEach((e) => (e.style.translate = `${x}px ${y}px`));
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  };
  /** Full-screen colour flash. */
  H.flash = (color = "#fff", ms = 350, opacity = 0.7) => {
    const d = H.el(
      `<div style="position:fixed;inset:0;background:${color};opacity:${opacity};transition:opacity ${ms}ms ease-out"></div>`,
    );
    H.fx.appendChild(d);
    requestAnimationFrame(() => requestAnimationFrame(() => (d.style.opacity = 0)));
    setTimeout(() => d.remove(), ms + 50);
  };
  /** Add an element to the overlay and remove it after ms (if given). */
  H.add = (node, ms) => {
    const n = typeof node === "string" ? H.el(node) : node;
    H.fx.appendChild(n);
    if (ms) setTimeout(() => n.remove(), ms);
    return n;
  };

  /** "Sam hit the gong" at the top of the screen. */
  H.banner = (who, text, emoji = "", ms = 2600) => {
    const p = typeof who === "string" ? H.people[who] : who;
    const b = H.el(
      `<div class="h-banner"><span class="av" style="background:${p.grad}">${p.initials}</span>${emoji ? `<span class="emo">${emoji}</span>` : ""}<span>${p === H.me ? "You" : p.name} ${text}</span></div>`,
    );
    document.body.appendChild(b);
    requestAnimationFrame(() => b.classList.add("in"));
    setTimeout(() => {
      b.classList.remove("in");
      setTimeout(() => b.remove(), 400);
    }, ms);
    return b;
  };

  // ---------- Registry ----------
  H.protos = {};
  H.register = (p) => (H.protos[p.n] = p);
  // Undoing what an effect set up is only for switching prototypes, which this never does.
  H.onLeave = () => {};

  /** Set every effect up, as the prototype did on showing it. */
  H.start = () => Object.values(H.protos).forEach((p) => p.mount(H));
})();
