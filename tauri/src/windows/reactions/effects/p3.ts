// @ts-nocheck
// 3. Table flip: (╯°□°)╯︵ ┻━┻ from the sender's tile, the table smashes at the screen edge,
// then a calm little figure puts it back ┬─┬ノ( º _ ºノ)
(() => {
  const TW = 230,
    TH = 150; // table box size at scale 1
  let alive = false,
    gen = 0,
    busy = false,
    timers = new Set(),
    anims = [];
  const later = (fn, ms) => {
    const id = setTimeout(() => {
      timers.delete(id);
      if (alive) fn();
    }, ms);
    timers.add(id);
    return id;
  };
  const track = (a) => {
    anims.push(a);
    a.onfinish = a.oncancel = () => (anims = anims.filter((x) => x !== a));
    return a;
  };
  const WOOD =
    "repeating-linear-gradient(176deg,rgba(0,0,0,0) 0 7px,rgba(60,25,5,.18) 8px,rgba(0,0,0,0) 10px 17px),linear-gradient(180deg,#d08a4c,#a8622c 55%,#7a4119)";
  const LEG = "linear-gradient(90deg,#5a2c10,#9a5526 40%,#7a3f18 75%,#4a230c)";

  const css = () =>
    H.css(
      "p3",
      `
    .p3-rage { position: absolute; inset: 0; z-index: 3; display: grid; place-items: center; pointer-events: none;
      background: radial-gradient(circle at 50% 55%, rgba(255,70,40,.92), rgba(170,10,25,.95) 75%); opacity: 0; transition: opacity .2s; }
    .p3-rage.on { opacity: 1; }
    .p3-rage b { font: 800 30px/1 -apple-system, "Hiragino Sans", sans-serif; color: #fff; white-space: nowrap; text-shadow: 0 3px 0 rgba(90,0,10,.55);
      animation: p3-tremble .07s linear infinite alternate; }
    .p3-rage .steam { position: absolute; top: 10px; right: 14px; font-size: 22px; animation: p3-puff .5s ease-out infinite; }
    @keyframes p3-tremble { from { transform: translate(-1.5px, 1px) rotate(-1.5deg); } to { transform: translate(1.5px, -1px) rotate(1.5deg); } }
    @keyframes p3-puff { from { transform: translateY(4px) scale(.7); opacity: 1; } to { transform: translateY(-14px) scale(1.2); opacity: 0; } }
    .p3-table { position: fixed; left: 0; top: 0; width: ${TW}px; height: ${TH}px; margin: ${-TH / 2}px 0 0 ${-TW / 2}px; will-change: transform; }
    .p3-table > i, .p3-piece { position: absolute; display: block; }
    .p3-top { left: 0; right: 0; top: 0; height: 24px; border-radius: 5px; background: ${WOOD};
      box-shadow: inset 0 2px 0 rgba(255,220,170,.5), inset 0 -5px 0 rgba(70,30,8,.5), 0 6px 0 rgba(0,0,0,.12); }
    .p3-apron { left: 12px; right: 12px; top: 24px; height: 14px; background: linear-gradient(180deg,#7a4119,#5a2c10); border-radius: 0 0 3px 3px; }
    .p3-leg { top: 30px; width: 17px; height: ${TH - 30}px; border-radius: 2px 2px 5px 5px; background: ${LEG}; clip-path: polygon(0 0,100% 0,82% 100%,18% 100%); }
    .p3-leg.back { top: 26px; height: ${TH - 42}px; filter: brightness(.6); }
    .p3-crack { position: fixed; overflow: visible; }
    .p3-dust { position: fixed; border-radius: 50%; background: radial-gradient(circle, rgba(235,215,190,.7), rgba(235,215,190,0) 70%); }
    .p3-fig { position: fixed; white-space: nowrap; font: 700 46px/1 -apple-system, "Hiragino Sans", sans-serif; color: #fff;
      text-shadow: 0 3px 12px rgba(0,0,0,.45); }
    .p3-bubble { position: absolute; left: 60%; bottom: 100%; margin-bottom: 14px; padding: 7px 12px; border-radius: 14px 14px 14px 4px;
      background: #fff; color: #2a1640; font: 650 15px/1.2 -apple-system, sans-serif; white-space: nowrap; opacity: 0; transform: scale(.6); transform-origin: 0 100%;
      transition: opacity .25s, transform .4s cubic-bezier(.2,1.6,.4,1); text-shadow: none; }
    .p3-bubble.on { opacity: 1; transform: scale(1); }
  `,
    );

  const tableHTML = (cls = "") => `<div class="p3-table ${cls}">
    <i class="p3-leg back" style="left:42px"></i><i class="p3-leg back" style="right:42px"></i>
    <i class="p3-apron"></i>
    <i class="p3-leg" style="left:14px"></i><i class="p3-leg" style="right:14px"></i>
    <i class="p3-top"></i></div>`;

  // ---------- Sound ----------
  const growl = () => {
    const a = H.ac(),
      f = a.createBiquadFilter();
    f.type = "bandpass";
    f.frequency.value = 650;
    f.Q.value = 1.4;
    f.connect(H.dest(0.12));
    f.frequency.setValueAtTime(500, a.currentTime);
    f.frequency.linearRampToValueAtTime(1100, a.currentTime + 0.45);
    H.tone({ freq: 105, glideTo: 170, dur: 0.48, gain: 0.45, attack: 0.06, type: "sawtooth", to: f });
    H.tone({ freq: 108, glideTo: 176, dur: 0.48, gain: 0.3, attack: 0.06, type: "sawtooth", detune: 25, to: f });
    H.noise({ filter: "bandpass", freq: 1400, q: 0.8, dur: 0.4, attack: 0.08, gain: 0.12 });
  };
  const heave = () => {
    // the "HUP!" of the throw
    H.tone({ freq: 190, glideTo: 120, dur: 0.18, gain: 0.35, type: "sawtooth", wet: 0.1 });
    H.noise({ filter: "lowpass", freq: 900, dur: 0.15, gain: 0.35 });
  };
  const spinWhoosh = (n, total) => {
    for (let i = 0; i < n; i++) {
      const w = (i / n) * total;
      H.noise({
        when: w,
        filter: "bandpass",
        freq: 350 + i * 180,
        sweepTo: 1600 + i * 300,
        q: 1.6,
        dur: total / n,
        attack: (total / n) * 0.6,
        gain: 0.12 + i * 0.05,
        wet: 0.2,
      });
    }
  };
  const knock = (when, f, g = 0.25) => {
    H.tone({ when, freq: f, glideTo: f * 0.92, dur: 0.07, gain: g, type: "triangle", wet: 0.25 });
    H.tone({ when, freq: f * 2.7, dur: 0.03, gain: g * 0.4, wet: 0.25 });
    H.noise({ when, filter: "bandpass", freq: 2200, q: 2, dur: 0.025, gain: g * 0.6 });
  };
  const crash = () => {
    H.noise({ filter: "lowpass", freq: 7000, sweepTo: 600, dur: 0.6, gain: 0.9, wet: 0.35 });
    H.noise({ filter: "highpass", freq: 2500, dur: 0.3, gain: 0.45, wet: 0.2 });
    H.noise({ when: 0.03, filter: "bandpass", freq: 900, q: 0.7, dur: 0.45, gain: 0.5, wet: 0.3 });
    H.tone({ freq: 92, glideTo: 32, dur: 0.5, gain: 0.95, wet: 0.15 });
    H.tone({ freq: 160, glideTo: 60, dur: 0.18, gain: 0.5, type: "triangle", wet: 0.1 });
    let t = 0.08;
    for (let i = 0; i < 14; i++) {
      t += H.rand(0.035, 0.11) * (1 + i * 0.08);
      knock(t, H.rand(260, 950), 0.32 * (1 - i / 16));
    }
  };
  const tok = (when = 0, g = 0.32) => {
    H.tone({ when, freq: 880, glideTo: 840, dur: 0.1, gain: g, type: "triangle", wet: 0.3 });
    H.tone({ when, freq: 440, dur: 0.16, gain: g * 0.6, wet: 0.3 });
    H.tone({ when, freq: 140, glideTo: 90, dur: 0.12, gain: g * 0.7, wet: 0.1 });
  };

  // ---------- Pieces of a broken table, flung with simple physics ----------
  function shatter(x, y, dir) {
    const bits = [
      { w: TW * 0.52, h: 24, bg: WOOD, r: 5 },
      { w: TW * 0.48, h: 24, bg: WOOD, r: 5 },
      { w: 17, h: TH - 30, bg: LEG, r: 3 },
      { w: 17, h: TH - 30, bg: LEG, r: 3 },
      { w: 15, h: TH - 50, bg: LEG, r: 3 },
      { w: 15, h: 70, bg: LEG, r: 3 },
      { w: 60, h: 14, bg: "#6a3514", r: 2 },
      { w: 44, h: 12, bg: "#7a4119", r: 2 },
    ].map((b) => {
      const el = H.add(
        `<i class="p3-piece" style="position:fixed;left:0;top:0;width:${b.w}px;height:${b.h}px;margin:${-b.h / 2}px 0 0 ${-b.w / 2}px;border-radius:${b.r}px;background:${b.bg};box-shadow:0 4px 10px rgba(0,0,0,.3)"></i>`,
      );
      return {
        el,
        x: x + H.rand(-40, 40),
        y: y + H.rand(-50, 50),
        vx: dir * H.rand(3, 14),
        vy: H.rand(-17, -6),
        r: H.rand(0, 360),
        vr: H.rand(-22, 22),
      };
    });
    const t0 = performance.now(),
      g = gen;
    const step = (t) => {
      if (!alive || g !== gen) return;
      const age = t - t0,
        op = H.clamp(1 - (age - 1100) / 500, 0, 1);
      for (const b of bits) {
        b.vy += 0.75;
        b.vx *= 0.99;
        b.x += b.vx;
        b.y += b.vy;
        b.r += b.vr;
        if (b.y > innerHeight - 20 && b.vy > 0) {
          b.y = innerHeight - 20;
          b.vy *= -0.35;
          b.vx *= 0.7;
          b.vr *= 0.6;
        }
        b.el.style.transform = `translate(${b.x}px,${b.y}px) rotate(${b.r}deg)`;
        b.el.style.opacity = op;
      }
      if (age < 1650) requestAnimationFrame(step);
      else bits.forEach((b) => b.el.remove());
    };
    requestAnimationFrame(step);
  }

  function crack(x, y) {
    let d = "";
    for (let i = 0; i < 11; i++) {
      const a = (i / 11) * Math.PI * 2 + H.rand(-0.2, 0.2),
        L = H.rand(70, 190);
      let px = 0,
        py = 0;
      d += `M0 0`;
      for (let s = 1; s <= 3; s++) {
        px = (Math.cos(a + H.rand(-0.25, 0.25)) * L * s) / 3;
        py = (Math.sin(a + H.rand(-0.25, 0.25)) * L * s) / 3;
        d += ` L${px.toFixed(1)} ${py.toFixed(1)}`;
      }
    }
    for (const rr of [34, 78]) {
      d += ` M${rr} 0`;
      for (let i = 1; i <= 12; i++) {
        const a = (i / 12) * Math.PI * 2,
          k = rr * H.rand(0.85, 1.15);
        d += ` L${(Math.cos(a) * k).toFixed(1)} ${(Math.sin(a) * k).toFixed(1)}`;
      }
    }
    const svg = H.add(
      `<svg class="p3-crack" style="left:${x - 200}px;top:${y - 200}px" width="400" height="400" viewBox="-200 -200 400 400">
      <path d="${d}" fill="none" stroke="rgba(255,255,255,.85)" stroke-width="2" stroke-linejoin="round"/>
      <path d="${d}" fill="none" stroke="rgba(0,0,0,.25)" stroke-width="1" transform="translate(1.5 1.5)"/></svg>`,
      2400,
    );
    svg.animate(
      [
        { opacity: 0, transform: "scale(.6)" },
        { opacity: 1, transform: "scale(1)", offset: 0.05 },
        { opacity: 1, offset: 0.7 },
        { opacity: 0 },
      ],
      { duration: 2300, easing: "ease-out" },
    );
  }

  // ---------- The gag ----------
  function play(who) {
    if (busy) {
      // already mid-flip: an extra angry rattle of the call window
      growl();
      track(
        H.callWin.animate([{ rotate: "0deg" }, { rotate: "-3deg" }, { rotate: "2deg" }, { rotate: "0deg" }], {
          duration: 300,
        }),
      );
      return;
    }
    busy = true;
    const tile = H.tiles[who.key] || H.tiles.call;
    let rage = tile.querySelector(".p3-rage");
    if (!rage) {
      rage = H.el(`<div class="p3-rage"><b>(ಠ益ಠ)</b><span class="steam">💢</span></div>`);
      tile.appendChild(rage);
    }
    const face = rage.querySelector("b");
    face.textContent = "(ಠ益ಠ)";
    requestAnimationFrame(() => rage.classList.add("on"));
    growl();
    track(
      tile.animate(
        [
          { translate: "0 0" },
          { translate: "-3px 1px" },
          { translate: "3px -1px" },
          { translate: "-2px 0" },
          { translate: "0 0" },
        ],
        { duration: 120, iterations: 4 },
      ),
    );
    H.banner(who, "flipped the table", "┻━┻", 3200);

    later(() => {
      face.textContent = "(╯°□°)╯︵";
      face.animate([{ scale: 0.7 }, { scale: 1.25 }, { scale: 1 }], {
        duration: 320,
        easing: "cubic-bezier(.2,1.6,.4,1)",
      });
      heave();
      // The call window jolts from the force of the throw.
      track(
        H.callWin.animate(
          [
            { rotate: "0deg", translate: "0 0" },
            { rotate: "-6deg", translate: "-18px -14px", offset: 0.15 },
            { rotate: "3.5deg", translate: "8px 6px", offset: 0.4 },
            { rotate: "-1.5deg", translate: "-3px -2px", offset: 0.65 },
            { rotate: "0deg", translate: "0 0" },
          ],
          { duration: 900, easing: "ease-out" },
        ),
      );
      fling(tile);
    }, 480);
    later(() => rage.classList.remove("on"), 2500);
    later(() => rage.remove(), 2800);
  }

  function fling(tile) {
    const c = H.centerOf(tile),
      goLeft = c.x > innerWidth / 2,
      dir = goLeft ? -1 : 1;
    const ex = goLeft ? 0 : innerWidth,
      ey = innerHeight * H.rand(0.42, 0.58),
      endScale = 1.35;
    const P0 = { x: c.x, y: c.y - 20 },
      P2 = { x: ex - dir * TW * 0.42 * endScale, y: ey },
      P1 = { x: (P0.x + P2.x) / 2 + dir * 40, y: -innerHeight * 0.32 };
    const spins = 2.25,
      finalRot = dir * (360 * spins),
      N = 36,
      kf = [];
    for (let i = 0; i <= N; i++) {
      const k = i / N,
        u = 1 - k;
      const x = u * u * P0.x + 2 * u * k * P1.x + k * k * P2.x,
        y = u * u * P0.y + 2 * u * k * P1.y + k * k * P2.y;
      const s = 0.3 + (endScale - 0.3) * Math.sqrt(k);
      kf.push({ transform: `translate(${x}px,${y}px) rotate(${finalRot * k}deg) scale(${s})` });
    }
    const dur = 950,
      table = H.add(tableHTML()),
      g = gen;
    table.style.transform = kf[0].transform;
    spinWhoosh(5, dur / 1000);
    table.animate(kf, { duration: dur, easing: "cubic-bezier(.3,.1,.7,1)", fill: "forwards" }).onfinish = () => {
      if (!alive || g !== gen) return;
      table.remove();
      smash(ex, ey, -dir);
    };
  }

  function smash(x, y, away) {
    crash();
    H.shake(26, 700);
    H.flash("#fff", 220, 0.45);
    crack(x, y);
    shatter(x + away * 70, y, away);
    const ang = away > 0 ? 0 : 180;
    H.emit({
      x,
      y,
      count: 110,
      angle: ang,
      spread: 150,
      speed: [5, 19],
      gravity: 0.45,
      drag: 0.975,
      life: [50, 110],
      size: [5, 13],
      shape: "rect",
      colors: ["#c9803f", "#a8622c", "#7a4119", "#e3a56a", "#5a2c10"],
    });
    H.emit({
      x,
      y,
      count: 40,
      angle: ang,
      spread: 110,
      speed: [8, 22],
      gravity: 0.5,
      life: [40, 80],
      size: [4, 8],
      shape: "tri",
      colors: ["#e3a56a", "#f0c08a", "#8a4a1c"],
    });
    for (let i = 0; i < 5; i++) {
      const s = H.rand(90, 180),
        d = H.add(
          `<div class="p3-dust" style="left:${x - s / 2}px;top:${y - s / 2 + H.rand(-60, 60)}px;width:${s}px;height:${s}px"></div>`,
          1500,
        );
      d.animate(
        [
          { transform: "translate(0,0) scale(.3)", opacity: 0.9 },
          { transform: `translate(${away * H.rand(60, 180)}px,${H.rand(-60, 20)}px) scale(1.6)`, opacity: 0 },
        ],
        { duration: 1400, easing: "cubic-bezier(.1,.7,.3,1)", fill: "forwards" },
      );
    }
    later(putBack, 1200);
  }

  function putBack() {
    const cx = innerWidth / 2,
      cy = innerHeight * 0.52;
    const table = H.add(tableHTML());
    const at = (r, dy = 0, s = 1) => `translate(${cx - 70}px,${cy + dy}px) rotate(${r}deg) scale(${s})`;
    table.style.transform = at(180);
    table.animate(
      [
        { transform: at(180, -30, 0.9), opacity: 0 },
        { transform: at(180), opacity: 1 },
      ],
      { duration: 420, easing: "cubic-bezier(.2,1.4,.4,1)" },
    );
    tok(0.3, 0.22);
    const fig = H.add(
      `<div class="p3-fig" style="left:${cx + 52}px;top:${cy - 26}px">ノ( º _ ºノ)<span class="p3-bubble">please respect the furniture</span></div>`,
    );
    fig.animate(
      [
        { transform: "translateX(260px)", opacity: 0 },
        { transform: "translateX(0)", opacity: 1 },
      ],
      { duration: 700, easing: "cubic-bezier(.25,1,.4,1)", fill: "backwards", delay: 250 },
    );
    // Little walking bob.
    fig.animate([{ translate: "0 0" }, { translate: "0 -6px" }, { translate: "0 0" }], {
      duration: 175,
      iterations: 4,
      delay: 250,
    });
    later(() => {
      table.animate(
        [
          { transform: at(180), easing: "cubic-bezier(.4,0,.6,1)" },
          { transform: at(90, -70, 1.04), offset: 0.5, easing: "cubic-bezier(.3,0,.5,1)" },
          { transform: at(-4, 4, 1), offset: 0.85, easing: "ease-out" },
          { transform: at(0) },
        ],
        { duration: 900, fill: "forwards" },
      );
      fig.animate([{ rotate: "0deg" }, { rotate: "-8deg", translate: "0 -14px" }, { rotate: "0deg" }], {
        duration: 900,
        easing: "ease-in-out",
      });
      H.noise({ filter: "bandpass", freq: 600, sweepTo: 1200, q: 1, dur: 0.45, attack: 0.25, gain: 0.06, wet: 0.2 });
    }, 1000);
    later(() => {
      tok(0, 0.34);
      tok(0.11, 0.18);
      H.shake(2, 160);
      fig.querySelector(".p3-bubble").classList.add("on");
    }, 1780);
    later(() => {
      [table, fig].forEach((n) => n.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 500, fill: "forwards" }));
    }, 2700);
    later(() => {
      table.remove();
      fig.remove();
      busy = false;
    }, 3250);
  }

  H.register({
    n: 3,
    title: "Table flip",
    pitch:
      "(╯°□°)╯︵ ┻━┻  Your tile rages, a wooden table spins across everyone's screen and smashes at the edge. Then someone calmly puts it back.",
    how: "Click the ┻━┻ button under the call. Click again mid-flip for extra rage.",
    mount(H) {
      alive = true;
      busy = false;
      css();
      const b = H.makeButton(
        `<span style="font-size:11px;font-weight:800;letter-spacing:-1px">┻━┻</span>`,
        "Flip the table",
      );
      b.onclick = () => this.play(H, H.me);
      H.slot.appendChild(b);
      H.onLeave(() => {
        alive = false;
        busy = false;
        gen++;
        timers.forEach(clearTimeout);
        timers.clear();
        anims.slice().forEach((a) => a.cancel());
        anims = [];
        document.querySelectorAll(".p3-rage").forEach((e) => e.remove());
      });
    },
    play(H, who) {
      play(who);
    },
  });
})();
