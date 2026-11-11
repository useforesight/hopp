// @ts-nocheck
// 8. Achievement unlocked: a console-style toast for everyone, with the two-note "bloop-bling" chime.
(() => {
  const PICKS = ["Shipped it", "Fixed the bug", "First PR merged"];
  const GOLD = ["#ffd34d", "#ffe58a", "#fff3c4", "#ffb627", "#ffffff"];
  const esc = (s) =>
    String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const timers = new Set();
  const later = (ms, fn) => {
    const id = setTimeout(() => {
      timers.delete(id);
      fn();
    }, ms);
    timers.add(id);
    return id;
  };
  let active = null; // the toast on screen right now

  const CSS = `
  .p8-pop { position: absolute; right: calc(100% + 14px); bottom: 4px; width: 292px; padding: 14px; border-radius: 16px; z-index: 30;
    background: rgba(22,16,36,.94); backdrop-filter: blur(24px); border: 1px solid rgba(255,255,255,.12); color: #fff;
    box-shadow: 0 24px 60px rgba(0,0,0,.45); transform-origin: 100% 88%; transform: translateX(12px) scale(.86); opacity: 0; pointer-events: none;
    transition: transform .38s cubic-bezier(.2,1.5,.4,1), opacity .18s; }
  .p8-pop.open { transform: none; opacity: 1; pointer-events: auto; }
  .p8-pop::after { content: ""; position: absolute; right: -7px; bottom: 20px; width: 12px; height: 12px; transform: rotate(45deg);
    background: rgba(22,16,36,.94); border-top: 1px solid rgba(255,255,255,.12); border-right: 1px solid rgba(255,255,255,.12); }
  .p8-h { font-size: 15px; font-weight: 750; display: flex; align-items: center; gap: 8px; }
  .p8-lbl { font-size: 10.5px; text-transform: uppercase; letter-spacing: .08em; opacity: .55; font-weight: 700; margin: 12px 0 6px; }
  .p8-who { display: flex; gap: 7px; align-items: center; }
  .p8-who button { all: unset; cursor: pointer; height: 30px; min-width: 30px; border-radius: 999px; display: grid; place-items: center;
    font-size: 11px; font-weight: 800; opacity: .55; transition: box-shadow .15s, opacity .15s, transform .25s cubic-bezier(.2,1.6,.4,1); }
  .p8-who button.all { padding: 0 12px; background: rgba(255,255,255,.1); font-weight: 650; font-size: 12px; }
  .p8-who button:hover { opacity: .9; }
  .p8-who button.on { opacity: 1; transform: scale(1.08); box-shadow: 0 0 0 2px #ffd34d, 0 0 14px rgba(255,211,77,.55); }
  .p8-in { all: unset; display: block; width: 100%; box-sizing: border-box; padding: 10px 12px; border-radius: 10px; font-size: 14px; color: #fff;
    background: rgba(255,255,255,.08); border: 1px solid rgba(255,255,255,.14); transition: border-color .15s, box-shadow .15s; }
  .p8-in::placeholder { color: rgba(255,255,255,.38); }
  .p8-in:focus { border-color: #ffd34d; box-shadow: 0 0 0 3px rgba(255,211,77,.2); }
  .p8-picks { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 9px; }
  .p8-picks button { all: unset; cursor: pointer; font-size: 12px; padding: 5px 10px; border-radius: 999px; background: rgba(255,255,255,.08);
    border: 1px solid rgba(255,255,255,.12); transition: background .15s, border-color .15s, transform .2s cubic-bezier(.2,1.6,.4,1); }
  .p8-picks button:hover { background: rgba(255,211,77,.18); border-color: rgba(255,211,77,.55); transform: translateY(-1px); }
  .p8-send { all: unset; cursor: pointer; margin-top: 12px; display: flex; justify-content: center; align-items: center; gap: 8px; width: 100%;
    box-sizing: border-box; padding: 9px; border-radius: 10px; font-weight: 750; font-size: 13px; color: #3a2400;
    background: linear-gradient(180deg,#ffe58a,#ffb627); box-shadow: 0 6px 18px rgba(255,182,39,.35); transition: transform .2s cubic-bezier(.2,1.6,.4,1); }
  .p8-send:hover { transform: translateY(-1px) scale(1.02); }
  .p8-send kbd { font: inherit; font-size: 11px; opacity: .55; }

  .p8-toast { position: fixed; left: 50%; top: 100px; transform: translate(-50%, -40px) scale(.6); opacity: 0;
    transition: transform .5s cubic-bezier(.2,1.6,.4,1), opacity .2s; }
  .p8-toast.in { transform: translate(-50%, 0) scale(1); opacity: 1; }
  .p8-toast.out { transform: translate(-50%, -70px) scale(.85); opacity: 0; transition: transform .45s cubic-bezier(.55,-0.5,.8,.4), opacity .4s .05s; }
  .p8-pill { position: relative; height: 72px; width: 72px; border-radius: 999px; overflow: hidden; display: flex; align-items: center; color: #fff;
    background: linear-gradient(180deg,#34343c,#141418 70%); transition: width .62s cubic-bezier(.2,1.12,.3,1);
    box-shadow: 0 0 0 1.5px rgba(255,211,77,.55), inset 0 1px 0 rgba(255,255,255,.12), 0 20px 55px rgba(0,0,0,.55), 0 0 50px rgba(255,190,40,.28); }
  .p8-icon { flex: none; width: 72px; height: 72px; display: grid; place-items: center; position: relative; }
  .p8-icon::before { content: ""; position: absolute; inset: 9px; border-radius: 50%;
    background: radial-gradient(circle at 50% 35%, #fff6cf 0%, #ffd34d 38%, #e08a00 78%, #7a4200 100%);
    box-shadow: inset 0 -4px 8px rgba(120,60,0,.55); animation: p8-glow 1.1s ease-in-out infinite alternate; }
  .p8-icon span { position: relative; font-size: 28px; filter: drop-shadow(0 2px 2px rgba(80,40,0,.5)); animation: p8-pop .75s cubic-bezier(.2,1.9,.4,1) both; }
  .p8-body, .p8-end { flex: none; white-space: nowrap; opacity: 0; transform: translateX(-14px);
    transition: opacity .3s .18s, transform .55s .18s cubic-bezier(.2,1.5,.4,1); }
  .p8-end { display: flex; align-items: center; gap: 12px; padding-right: 16px; transition-delay: .28s; }
  .p8-pill.wide .p8-body, .p8-pill.wide .p8-end { opacity: 1; transform: none; }
  .p8-body { padding: 0 22px 0 6px; }
  .p8-k { font-size: 11.5px; font-weight: 750; letter-spacing: .09em; text-transform: uppercase; color: #ffd34d; }
  .p8-k span { color: rgba(255,255,255,.55); }
  .p8-t { font-size: 19px; font-weight: 750; margin-top: 3px; letter-spacing: -.01em; }
  .p8-g { font-size: 15px; font-weight: 800; color: #fff3c4; padding: 5px 11px; border-radius: 999px; background: rgba(255,211,77,.14); border: 1px solid rgba(255,211,77,.4); }
  .p8-av { width: 42px; height: 42px; border-radius: 50%; display: grid; place-items: center; font-size: 13px; font-weight: 800; box-shadow: 0 0 0 2px #ffd34d, 0 0 14px rgba(255,211,77,.5); }
  .p8-avs { display: flex; padding-left: 10px; }
  .p8-avs .p8-av { width: 34px; height: 34px; font-size: 11px; margin-left: -10px; box-shadow: 0 0 0 2px #1d1d22; }
  .p8-glint { position: absolute; top: -10px; bottom: -10px; left: -45%; width: 34%; transform: skewX(-22deg); pointer-events: none; mix-blend-mode: screen;
    background: linear-gradient(90deg, rgba(255,230,150,0), rgba(255,236,170,.65) 50%, rgba(255,230,150,0)); }
  .p8-glint.go { animation: p8-sweep 1s cubic-bezier(.45,0,.2,1) forwards; }
  @keyframes p8-sweep { to { left: 125%; } }
  @keyframes p8-glow { from { box-shadow: inset 0 -4px 8px rgba(120,60,0,.55), 0 0 12px 2px rgba(255,200,60,.55); }
    to { box-shadow: inset 0 -4px 8px rgba(120,60,0,.55), 0 0 30px 9px rgba(255,200,60,.9); } }
  @keyframes p8-pop { 0% { transform: scale(0) rotate(-40deg); } 100% { transform: none; } }
  .tile.p8-gold { z-index: 2; animation: p8-tile .8s ease-in-out infinite alternate; }
  .tile.p8-gold::after { content: "🏆"; position: absolute; top: 8px; right: 8px; font-size: 20px; filter: drop-shadow(0 0 8px rgba(255,200,60,.9));
    animation: p8-pop .6s cubic-bezier(.2,1.9,.4,1) both; }
  @keyframes p8-tile { from { box-shadow: 0 0 0 2px #ffd34d, 0 0 14px 2px rgba(255,196,50,.6); }
    to { box-shadow: 0 0 0 4px #ffe58a, 0 0 38px 12px rgba(255,196,50,.9); } }`;

  // The chime: a rising "bloop", then a bright bell-like "bling" with a sparkly shimmer on top.
  function chime(H) {
    const bus = H.dest(0.45);
    H.tone({ freq: 392, glideTo: 784, dur: 0.17, type: "sine", gain: 0.34, attack: 0.01, to: bus });
    H.tone({ freq: 784, glideTo: 1568, dur: 0.13, type: "triangle", gain: 0.07, attack: 0.01, to: bus });
    const t = 0.16;
    [
      [1046.5, 0.2],
      [1568, 0.15],
      [2093, 0.1],
      [2637, 0.07],
    ].forEach(([f, g], i) => {
      H.tone({ when: t + i * 0.012, freq: f, type: "sine", gain: g, dur: 1.7, attack: 0.004, to: bus });
      H.tone({ when: t + i * 0.012, freq: f, type: "triangle", detune: 6, gain: g * 0.35, dur: 0.9, to: bus });
      H.tone({ when: t + i * 0.012, freq: f * 2.76, type: "sine", gain: g * 0.18, dur: 0.35, to: bus }); // bell partial
    });
    const sparkle = [3136, 3520, 3951, 4186, 4699, 5274];
    for (let i = 0; i < 12; i++)
      H.tone({ when: t + 0.06 + i * 0.04, freq: sparkle[(i * 5) % 6], dur: 0.4, gain: 0.025, to: bus });
    H.noise({ when: t, dur: 1.1, gain: 0.035, filter: "highpass", freq: 8000, sweepTo: 12000, attack: 0.03, to: bus });
  }

  const avatar = (p) => `<span class="p8-av" style="background:${p.grad}">${p.initials}</span>`;

  H.register({
    n: 8,
    title: "Achievement unlocked",
    pitch:
      'Award an achievement to the whole team or one person. Everyone gets a console-style "Achievement unlocked" toast with that chime.',
    how: "Click 🏆, pick who it's for, type what they did (or tap a quick pick) and press Enter.",
    mount(H) {
      H.css("p8", CSS);
      const self = this;
      const b = H.makeButton("🏆", "Celebrate an achievement");
      H.slot.appendChild(b);
      const people = Object.values(H.people);
      const pop = H.el(`<div class="p8-pop">
        <div class="p8-h">🏆 Celebrate…</div>
        <div class="p8-lbl">For</div>
        <div class="p8-who"><button class="all on" data-to="">Everyone</button>${people
          .map(
            (p) => `<button data-to="${p.key}" title="${p.full}" style="background:${p.grad}">${p.initials}</button>`,
          )
          .join("")}</div>
        <div class="p8-lbl">What happened</div>
        <input class="p8-in" placeholder="Shipped it" maxlength="48">
        <div class="p8-picks">${PICKS.map((t) => `<button>${t}</button>`).join("")}</div>
        <button class="p8-send">Unlock it <kbd>↵</kbd></button></div>`);
      H.callWin.appendChild(pop);
      const input = pop.querySelector(".p8-in");
      let to = null;
      const open = (v) => {
        pop.classList.toggle("open", v);
        if (v) setTimeout(() => input.focus(), 60);
      };
      const send = (text) => {
        text = (text || input.value).trim() || input.placeholder;
        open(false);
        input.value = "";
        self.play(H, H.me, { text, to });
      };
      b.onclick = () => open(!pop.classList.contains("open"));
      pop.querySelectorAll(".p8-who button").forEach(
        (w) =>
          (w.onclick = () => {
            to = w.dataset.to && w.dataset.to !== to ? w.dataset.to : null;
            pop
              .querySelectorAll(".p8-who button")
              .forEach((x) => x.classList.toggle("on", (x.dataset.to || null) === to));
            input.focus();
          }),
      );
      pop.querySelectorAll(".p8-picks button").forEach((p) => (p.onclick = () => send(p.textContent)));
      pop.querySelector(".p8-send").onclick = () => send();
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          send();
        }
        if (e.key === "Escape") open(false);
      });
      const outside = (e) => {
        if (!pop.contains(e.target) && !b.contains(e.target)) open(false);
      };
      document.addEventListener("pointerdown", outside);
      H.onLeave(() => {
        document.removeEventListener("pointerdown", outside);
        pop.remove();
        timers.forEach(clearTimeout);
        timers.clear();
        Object.values(H.tiles).forEach((t) => t.classList.remove("p8-gold"));
        active = null;
      });
    },
    play(H, who, choice = {}) {
      const text = String(choice.text || "Nailed it").slice(0, 60);
      const rec = choice.to ? H.people[choice.to] : null;
      if (active) active.kill();
      const mine = [];
      const at = (ms, fn) => mine.push(later(ms, fn));
      const recName =
        rec ?
          rec === H.me ?
            "You"
          : rec.name
        : "Everyone";
      const toast = H.add(`<div class="p8-toast"><div class="p8-pill">
        <div class="p8-icon"><span>🏆</span></div>
        <div class="p8-body"><div class="p8-k">Achievement unlocked <span>· ${esc(recName)}</span></div><div class="p8-t">${esc(text)}</div></div>
        <div class="p8-end"><span class="p8-g">+100 G</span>${rec ? avatar(rec) : `<span class="p8-avs">${Object.values(H.people).map(avatar).join("")}</span>`}</div>
        <div class="p8-glint"></div></div></div>`);
      const pill = toast.querySelector(".p8-pill"),
        icon = toast.querySelector(".p8-icon");
      const wide = 72 + toast.querySelector(".p8-body").offsetWidth + toast.querySelector(".p8-end").offsetWidth;
      const tile = rec && H.tiles[rec.key];
      const me = (active = {
        kill() {
          mine.forEach((id) => {
            clearTimeout(id);
            timers.delete(id);
          });
          toast.classList.add("out");
          setTimeout(() => toast.remove(), 450);
          if (tile) tile.classList.remove("p8-gold");
          if (active === me) active = null;
        },
      });
      requestAnimationFrame(() => requestAnimationFrame(() => toast.classList.add("in")));
      chime(H);
      // Trophy pops: gold stars burst out of the circle.
      at(170, () => {
        const c = H.centerOf(icon);
        H.emit({
          x: c.x,
          y: c.y,
          count: 34,
          angle: 0,
          spread: 360,
          speed: [3, 10],
          gravity: 0.12,
          drag: 0.94,
          life: [40, 80],
          size: [8, 15],
          shape: "star",
          colors: GOLD,
          glow: true,
          spin: true,
        });
        H.emit({
          x: c.x,
          y: c.y,
          count: 24,
          angle: 0,
          spread: 360,
          speed: [1, 6],
          gravity: 0.05,
          drag: 0.95,
          life: [30, 60],
          size: [2, 4],
          shape: "spark",
          colors: GOLD,
          glow: true,
        });
      });
      // The circle opens into the wide bar.
      at(420, () => {
        pill.style.width = wide + "px";
        pill.classList.add("wide");
        H.noise({
          dur: 0.35,
          gain: 0.05,
          filter: "bandpass",
          freq: 700,
          sweepTo: 3500,
          q: 0.8,
          attack: 0.08,
          wet: 0.2,
        });
        if (tile) {
          tile.classList.add("p8-gold");
          const c = H.centerOf(tile);
          H.emit({
            x: c.x,
            y: c.y,
            count: 40,
            angle: -90,
            spread: 200,
            speed: [4, 11],
            gravity: 0.2,
            drag: 0.95,
            life: [40, 90],
            size: [7, 13],
            shape: "star",
            colors: GOLD,
            glow: true,
          });
        }
      });
      at(1050, () => {
        toast.querySelector(".p8-glint").classList.add("go");
        const r = pill.getBoundingClientRect();
        for (let i = 0; i < 5; i++)
          setTimeout(
            () =>
              H.emit({
                x: r.left + (r.width * (i + 0.5)) / 5,
                y: r.bottom - 6,
                count: 5,
                angle: 90,
                spread: 70,
                speed: [1, 3],
                gravity: 0.08,
                life: [30, 55],
                size: [5, 9],
                shape: "star",
                colors: GOLD,
                glow: true,
              }),
            i * 110,
          );
      });
      at(4300, () => {
        pill.classList.remove("wide");
        pill.style.width = "72px";
      });
      at(4700, () => {
        toast.classList.add("out");
        if (tile) tile.classList.remove("p8-gold");
      });
      at(5200, () => {
        toast.remove();
        if (active === me) active = null;
      });
      const txt =
        !rec ? "unlocked an achievement for everyone"
        : rec === who ? "unlocked an achievement"
        : `awarded ${rec === H.me ? "you" : rec.name} an achievement`;
      H.banner(who, txt, "🏆", 3000);
    },
  });
})();
