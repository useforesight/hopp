// @ts-nocheck
// 1. Party popper: one tap, confetti cannons fire from both bottom corners of everyone's screen.
H.register({
  n: 1,
  title: "Party popper",
  pitch: "One button, one tap. Confetti cannons fire from both corners of everyone's screen, with a proper pop.",
  how: "Click 🎉 under the call. Tap it again quickly for a bigger burst.",
  mount(H) {
    const b = H.makeButton("🎉", "Confetti");
    let last = 0,
      streak = 0;
    b.onclick = () => {
      const now = performance.now();
      streak = now - last < 900 ? Math.min(streak + 1, 4) : 0;
      last = now;
      this.play(H, H.me, streak);
    };
    H.slot.appendChild(b);
  },
  play(H, who, streak = 0) {
    const big = 1 + streak * 0.5;
    const pop = (when, pan) => {
      H.noise({ when, dur: 0.09, gain: 0.7, filter: "highpass", freq: 900, q: 0.7, wet: 0.1 });
      H.tone({ when, freq: 180, glideTo: 50, dur: 0.12, gain: 0.5, type: "sine", wet: 0.05 });
      // Paper flutter afterwards.
      H.noise({
        when: when + 0.05,
        dur: 1.2,
        gain: 0.08,
        filter: "bandpass",
        freq: 5000,
        q: 0.6,
        sweepTo: 2500,
        attack: 0.05,
      });
    };
    pop(0, -1);
    pop(0.07, 1);
    const fire = (x, angle) =>
      H.emit({
        x,
        y: innerHeight + 10,
        count: Math.round(110 * big),
        angle,
        spread: 26,
        speed: [16, 27],
        gravity: 0.32,
        drag: 0.983,
        life: [120, 200],
        size: [7, 13],
        shape: H.pick(["rect", "rect", "tri"]),
      });
    fire(0, -62);
    setTimeout(() => fire(innerWidth, -118), 70);
    setTimeout(() => {
      H.emit({
        x: 40,
        y: innerHeight,
        count: Math.round(30 * big),
        angle: -60,
        spread: 30,
        speed: [14, 22],
        shape: "circle",
        size: [5, 8],
      });
      H.emit({
        x: innerWidth - 40,
        y: innerHeight,
        count: Math.round(30 * big),
        angle: -120,
        spread: 30,
        speed: [14, 22],
        shape: "circle",
        size: [5, 8],
      });
    }, 160);
    H.shake(4 + streak * 2, 300);
    H.banner(who, streak ? `popped confetti ×${streak + 1}` : "popped confetti", "🎉");
  },
});
