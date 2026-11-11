//! The reactions' sounds, made as the prototypes make them with Web Audio: oscillators
//! and noise through gains, filters and reverbs, every change scheduled ahead. A
//! [`Graph`] is built the same way, node by node, then rendered offline to samples
//! (see [`Graph::render`]) and played as one sound.
//!
//! Every graph ends as the prototypes' does: a master gain of 0.8 into a compressor
//! (threshold −14 dB, ratio 6, attack 3 ms, release 200 ms), with a generated 2.8 s hall
//! reverb beside it. A limiter after the compressor keeps the peaks under full scale.

use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use super::stage::Rng;

pub const RATE: u32 = 48_000;
const RATE_F: f32 = RATE as f32;

/// A node in a [`Graph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Id(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Square,
    Saw,
    Triangle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Lowpass,
    Highpass,
    Bandpass,
    Peaking,
}

#[derive(Clone, Copy, Debug)]
enum Ramp {
    Set,
    Linear,
    Exponential,
    /// Towards the value with this time constant.
    Target(f32),
}

#[derive(Clone, Copy, Debug)]
struct Change {
    at: f32,
    value: f32,
    ramp: Ramp,
}

/// A value that changes on a schedule, as a Web Audio `AudioParam` does, plus whatever
/// nodes are connected to it.
#[derive(Clone, Debug)]
pub struct Param {
    value: f32,
    changes: Vec<Change>,
    inputs: Vec<Id>,
}

impl Param {
    fn new(value: f32) -> Self {
        Self {
            value,
            changes: Vec::new(),
            inputs: Vec::new(),
        }
    }

    fn push(&mut self, change: Change) -> &mut Self {
        // Kept in time order; a change at the same time as one before goes after it.
        let index = self.changes.partition_point(|other| other.at <= change.at);
        self.changes.insert(index, change);
        self
    }

    /// `setValueAtTime`.
    pub fn set(&mut self, value: f32, at: f32) -> &mut Self {
        self.push(Change {
            at,
            value,
            ramp: Ramp::Set,
        })
    }

    /// `linearRampToValueAtTime`.
    pub fn linear(&mut self, value: f32, at: f32) -> &mut Self {
        self.push(Change {
            at,
            value,
            ramp: Ramp::Linear,
        })
    }

    /// `exponentialRampToValueAtTime`.
    pub fn exp(&mut self, value: f32, at: f32) -> &mut Self {
        self.push(Change {
            at,
            value,
            ramp: Ramp::Exponential,
        })
    }

    /// `setTargetAtTime`.
    pub fn target(&mut self, value: f32, at: f32, constant: f32) -> &mut Self {
        self.push(Change {
            at,
            value,
            ramp: Ramp::Target(constant.max(1e-4)),
        })
    }

    /// The value at each of `len` samples from sample `from`, before any inputs.
    fn values(&self, from: usize, len: usize) -> Vec<f32> {
        if self.changes.is_empty() {
            return vec![self.value; len];
        }
        let mut out = Vec::with_capacity(len);
        // The last change passed, and the value it left.
        let mut passed: Option<(Change, f32)> = None;
        let mut next = 0;
        let curve = |passed: &Option<(Change, f32)>, next: Option<&Change>, t: f32| -> f32 {
            let (start_at, start) = match passed {
                Some((change, value)) => (change.at, *value),
                None => (0., self.value),
            };
            if let Some(next) = next {
                let span = (next.at - start_at).max(1e-9);
                let k = ((t - start_at) / span).clamp(0., 1.);
                match next.ramp {
                    Ramp::Linear => return start + (next.value - start) * k,
                    Ramp::Exponential => {
                        // Web Audio holds still when the ramp can't be exponential.
                        if start == 0. || next.value == 0. || start.signum() != next.value.signum()
                        {
                            return start;
                        }
                        return start * (next.value / start).powf(k);
                    }
                    Ramp::Set | Ramp::Target(_) => {}
                }
            }
            match passed {
                &Some((
                    Change {
                        at,
                        value: target,
                        ramp: Ramp::Target(constant),
                    },
                    start,
                )) => target + (start - target) * (-(t - at) / constant).exp(),
                _ => start,
            }
        };
        for n in 0..len {
            let t = (from + n) as f32 / RATE_F;
            while next < self.changes.len() && self.changes[next].at <= t {
                let change = self.changes[next];
                let left = match change.ramp {
                    // It starts from wherever the curve before it had reached.
                    Ramp::Target(_) => curve(&passed, None, change.at),
                    _ => change.value,
                };
                passed = Some((change, left));
                next += 1;
            }
            out.push(curve(&passed, self.changes.get(next), t));
        }
        out
    }
}

/// Hall and room reverbs: two channels of decaying noise.
pub type Response = Arc<[Vec<f32>; 2]>;

/// `seconds` of noise decaying as `(1 - i/len)^power`, as the prototypes make theirs.
pub fn response(seconds: f32, power: f32, rng: &mut Rng) -> Response {
    let len = (RATE_F * seconds) as usize;
    let channel = |rng: &mut Rng| {
        (0..len)
            .map(|i| rng.range(-1., 1.) * (1. - i as f32 / len as f32).powf(power))
            .collect::<Vec<_>>()
    };
    Arc::new([channel(rng), channel(rng)])
}

fn hall() -> Response {
    static HALL: OnceLock<Response> = OnceLock::new();
    HALL.get_or_init(|| response(2.8, 3.2, &mut Rng::new(0x4a11)))
        .clone()
}

/// The prototypes' shared noise: three seconds of it.
fn noise_buffer() -> &'static [f32] {
    static NOISE: OnceLock<Vec<f32>> = OnceLock::new();
    NOISE.get_or_init(|| {
        let mut rng = Rng::new(0x9015e);
        (0..RATE as usize * 3).map(|_| rng.range(-1., 1.)).collect()
    })
}

#[derive(Clone, Debug)]
enum Kind {
    Osc {
        wave: Wave,
        freq: Param,
        detune: Param,
    },
    Noise {
        looped: bool,
        offset: f32,
    },
    Gain {
        gain: Param,
    },
    Biquad {
        filter: Filter,
        freq: Param,
        q: Param,
        gain: Param,
    },
    Shaper {
        curve: Arc<Vec<f32>>,
    },
    Panner {
        pan: Param,
    },
    Convolver {
        response: Response,
    },
}

#[derive(Clone, Debug)]
struct Node {
    kind: Kind,
    inputs: Vec<Id>,
    /// For sources: when they play.
    start: f32,
    stop: f32,
}

/// `H.tone`: a tone with an envelope.
#[derive(Clone, Copy, Debug)]
pub struct Tone {
    pub freq: f32,
    pub wave: Wave,
    pub dur: f32,
    pub gain: f32,
    pub attack: f32,
    pub when: f32,
    pub glide_to: Option<f32>,
    pub detune: f32,
    pub wet: f32,
    /// Where it goes instead of a fresh `dest(wet)`.
    pub to: Option<Id>,
}

impl Default for Tone {
    fn default() -> Self {
        Self {
            freq: 440.,
            wave: Wave::Sine,
            dur: 0.3,
            gain: 0.3,
            attack: 0.005,
            when: 0.,
            glide_to: None,
            detune: 0.,
            wet: 0.15,
            to: None,
        }
    }
}

/// `H.noise`: a filtered noise burst.
#[derive(Clone, Copy, Debug)]
pub struct Noise {
    pub dur: f32,
    pub gain: f32,
    pub filter: Filter,
    pub freq: f32,
    pub q: f32,
    pub when: f32,
    pub attack: f32,
    pub wet: f32,
    pub sweep_to: Option<f32>,
    pub to: Option<Id>,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            dur: 0.2,
            gain: 0.3,
            filter: Filter::Bandpass,
            freq: 2000.,
            q: 1.,
            when: 0.,
            attack: 0.002,
            wet: 0.15,
            sweep_to: None,
            to: None,
        }
    }
}

/// A sound being put together: nodes and how they connect, from time 0.
pub struct Graph {
    nodes: Vec<Node>,
    master: Id,
    verb: Id,
    pub rng: Rng,
}

impl Graph {
    pub fn new(seed: u64) -> Self {
        let mut graph = Self {
            nodes: Vec::new(),
            master: Id(0),
            verb: Id(0),
            rng: Rng::new(seed),
        };
        graph.master = graph.gain(0.8);
        graph.verb = graph.convolver(hall());
        let wet = graph.gain(0.55);
        graph.connect(graph.verb, wet);
        graph.connect(wet, graph.master);
        graph
    }

    fn add(&mut self, kind: Kind) -> Id {
        self.nodes.push(Node {
            kind,
            inputs: Vec::new(),
            start: 0.,
            stop: f32::INFINITY,
        });
        Id(self.nodes.len() - 1)
    }

    pub fn osc(&mut self, wave: Wave, freq: f32) -> Id {
        self.add(Kind::Osc {
            wave,
            freq: Param::new(freq),
            detune: Param::new(0.),
        })
    }

    /// The shared noise, from a random point in its first two seconds (as `H.noise`
    /// starts it), looping if `looped`.
    pub fn noise_source(&mut self, looped: bool) -> Id {
        let offset = self.rng.range(0., 2.);
        self.add(Kind::Noise { looped, offset })
    }

    pub fn gain(&mut self, gain: f32) -> Id {
        self.add(Kind::Gain {
            gain: Param::new(gain),
        })
    }

    pub fn filter(&mut self, filter: Filter, freq: f32, q: f32) -> Id {
        self.add(Kind::Biquad {
            filter,
            freq: Param::new(freq),
            q: Param::new(q),
            gain: Param::new(0.),
        })
    }

    pub fn shaper(&mut self, curve: Arc<Vec<f32>>) -> Id {
        self.add(Kind::Shaper { curve })
    }

    pub fn panner(&mut self, pan: f32) -> Id {
        self.add(Kind::Panner {
            pan: Param::new(pan),
        })
    }

    pub fn convolver(&mut self, response: Response) -> Id {
        self.add(Kind::Convolver { response })
    }

    /// Play source `id` from `start` until `stop` seconds.
    pub fn play(&mut self, id: Id, start: f32, stop: f32) {
        let node = &mut self.nodes[id.0];
        node.start = start.max(0.);
        node.stop = stop.max(node.start);
    }

    pub fn connect(&mut self, from: Id, to: Id) {
        self.nodes[to.0].inputs.push(from);
    }

    /// Feed `from` into a param, which `param` picks from `to`.
    pub fn modulate(&mut self, from: Id, to: Id, param: impl FnOnce(&mut Graph, Id) -> &mut Param) {
        param(self, to).inputs.push(from);
    }

    fn param(&mut self, id: Id, name: &str) -> &mut Param {
        match (&mut self.nodes[id.0].kind, name) {
            (Kind::Osc { freq, .. }, "frequency") | (Kind::Biquad { freq, .. }, "frequency") => {
                freq
            }
            (Kind::Osc { detune, .. }, "detune") => detune,
            (Kind::Gain { gain, .. }, "gain") | (Kind::Biquad { gain, .. }, "gain") => gain,
            (Kind::Biquad { q, .. }, "q") => q,
            (Kind::Panner { pan }, "pan") => pan,
            (kind, name) => panic!("no {name} on {kind:?}"),
        }
    }

    pub fn frequency(&mut self, id: Id) -> &mut Param {
        self.param(id, "frequency")
    }

    pub fn detune(&mut self, id: Id) -> &mut Param {
        self.param(id, "detune")
    }

    pub fn gain_of(&mut self, id: Id) -> &mut Param {
        self.param(id, "gain")
    }

    /// `H.dest`: a gain into the master, sending `wet` of itself to the hall.
    pub fn dest(&mut self, wet: f32) -> Id {
        let g = self.gain(1.);
        self.connect(g, self.master);
        if wet > 0. {
            let send = self.gain(wet);
            self.connect(g, send);
            self.connect(send, self.verb);
        }
        g
    }

    /// `H.tone`. Returns the oscillator and its gain.
    pub fn tone(&mut self, o: Tone) -> (Id, Id) {
        let (t, dur) = (o.when, o.dur);
        let osc = self.osc(o.wave, o.freq);
        self.frequency(osc).set(o.freq, t);
        if let Some(to) = o.glide_to {
            self.frequency(osc).exp(to, t + dur);
        }
        self.detune(osc).value = o.detune;
        let g = self.gain(1.);
        self.gain_of(g)
            .set(0.0001, t)
            .exp(o.gain, t + o.attack)
            .exp(0.0001, t + dur);
        let to = o.to.unwrap_or_else(|| self.dest(o.wet));
        self.connect(osc, g);
        self.connect(g, to);
        self.play(osc, t, t + dur + 0.05);
        (osc, g)
    }

    /// `H.noise`. Returns the source, its filter and its gain.
    pub fn noise(&mut self, o: Noise) -> (Id, Id, Id) {
        let (t, dur) = (o.when, o.dur);
        let src = self.noise_source(false);
        let f = self.filter(o.filter, o.freq, o.q);
        self.frequency(f).set(o.freq, t);
        if let Some(to) = o.sweep_to {
            self.frequency(f).exp(to, t + dur);
        }
        let g = self.gain(1.);
        self.gain_of(g)
            .set(0.0001, t)
            .exp(o.gain, t + o.attack)
            .exp(0.0001, t + dur);
        let to = o.to.unwrap_or_else(|| self.dest(o.wet));
        self.connect(src, f);
        self.connect(f, g);
        self.connect(g, to);
        self.play(src, t, t + dur + 0.05);
        (src, f, g)
    }

    /// Whether anything plays.
    pub fn is_silent(&self) -> bool {
        !self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, Kind::Osc { .. } | Kind::Noise { .. }))
    }

    /// The sound as 16-bit stereo samples at [`RATE`], interleaved.
    pub fn render(&self) -> Vec<i16> {
        let mix = self.mix();
        let [left, right] = &mix;
        let mut out = Vec::with_capacity(left.len() * 2);
        for (l, r) in left.iter().zip(right) {
            out.push((l * f32::from(i16::MAX)) as i16);
            out.push((r * f32::from(i16::MAX)) as i16);
        }
        out
    }

    /// The master, compressed and limited: two channels from time 0.
    fn mix(&self) -> [Vec<f32>; 2] {
        let order = self.order();
        let mut users = vec![0usize; self.nodes.len()];
        for &id in &order {
            for input in self.feeds(id) {
                users[input.0] += 1;
            }
        }
        let mut signals: Vec<Option<Signal>> = vec![None; self.nodes.len()];
        for &id in &order {
            let signal = self.process(id, &signals);
            for input in self.feeds(id) {
                users[input.0] -= 1;
                if users[input.0] == 0 {
                    signals[input.0] = None;
                }
            }
            signals[id.0] = Some(signal);
        }
        let master = signals[self.master.0].take().unwrap_or_default();
        let len = master.end();
        let mut channels = [vec![0.; len], vec![0.; len]];
        if let Some(last) = master.channels.len().checked_sub(1) {
            for (c, channel) in channels.iter_mut().enumerate() {
                channel[master.start..].copy_from_slice(&master.channels[c.min(last)]);
            }
        }
        compress(&mut channels);
        // Trim the reverb's silent end.
        let last = (0..len)
            .rev()
            .find(|&i| channels[0][i].abs() > 2e-5 || channels[1][i].abs() > 2e-5)
            .map_or(0, |i| i + 1);
        for channel in &mut channels {
            channel.truncate(last);
        }
        channels
    }

    /// Everything a node reads: its inputs and its params' inputs.
    fn feeds(&self, id: Id) -> Vec<Id> {
        let node = &self.nodes[id.0];
        let mut feeds = node.inputs.clone();
        let params: Vec<&Param> = match &node.kind {
            Kind::Osc { freq, detune, .. } => vec![freq, detune],
            Kind::Gain { gain } => vec![gain],
            Kind::Biquad { freq, q, gain, .. } => vec![freq, q, gain],
            Kind::Panner { pan } => vec![pan],
            Kind::Noise { .. } | Kind::Shaper { .. } | Kind::Convolver { .. } => vec![],
        };
        for param in params {
            feeds.extend(&param.inputs);
        }
        feeds.sort_by_key(|id| id.0);
        feeds.dedup();
        feeds
    }

    /// The nodes the master hears, each after everything it reads.
    fn order(&self) -> Vec<Id> {
        let mut order = Vec::new();
        let mut state = vec![0u8; self.nodes.len()];
        let mut stack = vec![(self.master, false)];
        while let Some((id, done)) = stack.pop() {
            if done {
                state[id.0] = 2;
                order.push(id);
                continue;
            }
            if state[id.0] != 0 {
                continue;
            }
            state[id.0] = 1;
            stack.push((id, true));
            for input in self.feeds(id) {
                if state[input.0] == 0 {
                    stack.push((input, false));
                }
            }
        }
        order
    }

    fn mixed_inputs(&self, inputs: &[Id], signals: &[Option<Signal>]) -> Signal {
        Signal::sum(inputs.iter().filter_map(|id| signals[id.0].as_ref()))
    }

    /// A param's value over `len` samples from `from`, with its inputs added.
    fn param_values(
        &self,
        param: &Param,
        from: usize,
        len: usize,
        signals: &[Option<Signal>],
    ) -> Vec<f32> {
        let mut values = param.values(from, len);
        for input in &param.inputs {
            if let Some(signal) = &signals[input.0] {
                signal.add_mono_to(&mut values, from);
            }
        }
        values
    }

    fn process(&self, id: Id, signals: &[Option<Signal>]) -> Signal {
        let node = &self.nodes[id.0];
        match &node.kind {
            Kind::Osc { wave, freq, detune } => {
                let (from, to) = source_span(node);
                let len = to - from;
                let freqs = self.param_values(freq, from, len, signals);
                let detunes = self.param_values(detune, from, len, signals);
                let mut phase = 0f64;
                let mut out = Vec::with_capacity(len);
                for (f, d) in freqs.iter().zip(&detunes) {
                    let f = f * (d / 1200.).exp2();
                    let dt = (f64::from(f) / f64::from(RATE)).clamp(-0.5, 0.5);
                    out.push(oscillate(*wave, phase as f32, dt as f32));
                    phase = (phase + dt).rem_euclid(1.);
                }
                Signal::mono(from, out)
            }
            Kind::Noise { looped, offset } => {
                let (from, to) = source_span(node);
                let buffer = noise_buffer();
                let skip = (offset * RATE_F) as usize;
                let out = (0..to - from)
                    .map_while(|i| {
                        let at = skip + i;
                        if *looped {
                            Some(buffer[at % buffer.len()])
                        } else {
                            buffer.get(at).copied()
                        }
                    })
                    .collect();
                Signal::mono(from, out)
            }
            Kind::Gain { gain } => {
                let mut signal = self.mixed_inputs(&node.inputs, signals);
                if signal.is_empty() {
                    return signal;
                }
                let values = self.param_values(gain, signal.start, signal.len(), signals);
                for channel in &mut signal.channels {
                    for (sample, g) in channel.iter_mut().zip(&values) {
                        *sample *= g;
                    }
                }
                signal
            }
            Kind::Biquad {
                filter,
                freq,
                q,
                gain,
            } => {
                let mut signal = self.mixed_inputs(&node.inputs, signals);
                if signal.is_empty() {
                    return signal;
                }
                // Room for it to ring on after its input stops.
                signal.extend(RATE as usize / 4);
                let (from, len) = (signal.start, signal.len());
                let freqs = self.param_values(freq, from, len, signals);
                let qs = self.param_values(q, from, len, signals);
                let gains = self.param_values(gain, from, len, signals);
                for channel in &mut signal.channels {
                    let mut state = [0f32; 4];
                    let mut k = Coefficients::default();
                    for (i, sample) in channel.iter_mut().enumerate() {
                        if i % 16 == 0 {
                            k = Coefficients::new(*filter, freqs[i], qs[i], gains[i]);
                        }
                        *sample = k.run(*sample, &mut state);
                    }
                }
                signal
            }
            Kind::Shaper { curve } => {
                let mut signal = self.mixed_inputs(&node.inputs, signals);
                for channel in &mut signal.channels {
                    for sample in channel.iter_mut() {
                        *sample = shape(curve, *sample);
                    }
                }
                signal
            }
            Kind::Panner { pan } => {
                let signal = self.mixed_inputs(&node.inputs, signals);
                if signal.is_empty() {
                    return signal;
                }
                let pans = self.param_values(pan, signal.start, signal.len(), signals);
                let mut left = Vec::with_capacity(signal.len());
                let mut right = Vec::with_capacity(signal.len());
                for (i, p) in pans.iter().enumerate() {
                    let p = p.clamp(-1., 1.);
                    if signal.channels.len() == 1 {
                        let x = (p + 1.) / 2. * std::f32::consts::FRAC_PI_2;
                        let s = signal.channels[0][i];
                        left.push(s * x.cos());
                        right.push(s * x.sin());
                    } else {
                        let (l, r) = (signal.channels[0][i], signal.channels[1][i]);
                        let x = if p <= 0. { p + 1. } else { p } * std::f32::consts::FRAC_PI_2;
                        if p <= 0. {
                            left.push(l + r * x.cos());
                            right.push(r * x.sin());
                        } else {
                            left.push(l * x.cos());
                            right.push(r + l * x.sin());
                        }
                    }
                }
                Signal {
                    start: signal.start,
                    channels: vec![left, right],
                }
            }
            Kind::Convolver { response } => {
                let signal = self.mixed_inputs(&node.inputs, signals);
                if signal.is_empty() {
                    return signal;
                }
                convolve(&signal, response)
            }
        }
    }
}

fn source_span(node: &Node) -> (usize, usize) {
    let from = (node.start * RATE_F) as usize;
    // Sources left running stop where the longest sound in the prototypes would.
    let stop = if node.stop.is_finite() {
        node.stop
    } else {
        node.start + 20.
    };
    (from, ((stop * RATE_F) as usize).max(from))
}

/// One cycle of `wave` at `phase` (0 to 1), smoothed at its steps for `dt` cycles per
/// sample (PolyBLEP), so high notes don't alias.
fn oscillate(wave: Wave, phase: f32, dt: f32) -> f32 {
    let blep = |t: f32| -> f32 {
        let dt = dt.abs().max(1e-6);
        if t < dt {
            let t = t / dt;
            t + t - t * t - 1.
        } else if t > 1. - dt {
            let t = (t - 1.) / dt;
            t * t + t + t + 1.
        } else {
            0.
        }
    };
    match wave {
        Wave::Sine => (phase * std::f32::consts::TAU).sin(),
        Wave::Saw => {
            // Up from 0 through 1 at half a cycle, then down to −1: Web Audio's phase.
            let t = (phase + 0.5).fract();
            2. * t - 1. - blep(t)
        }
        Wave::Square => {
            let naive = if phase < 0.5 { 1. } else { -1. };
            naive + blep(phase) - blep((phase + 0.5).fract())
        }
        Wave::Triangle => {
            let t = (phase + 0.25).fract();
            4. * (t - 0.5).abs() - 1.
        }
    }
    .clamp(-1.5, 1.5)
}

/// A `WaveShaper` curve: `tanh(k x)`, normalised to ±1 (the airhorn's `sat`).
pub fn saturation(k: f32) -> Arc<Vec<f32>> {
    let n = 1024;
    Arc::new(
        (0..n)
            .map(|i| {
                let x = i as f32 / (n - 1) as f32 * 2. - 1.;
                (k * x).tanh() / k.tanh()
            })
            .collect(),
    )
}

fn shape(curve: &[f32], x: f32) -> f32 {
    let n = curve.len();
    if n == 0 {
        return x;
    }
    let at = ((x.clamp(-1., 1.) + 1.) / 2. * (n - 1) as f32).clamp(0., (n - 1) as f32);
    let i = at.floor() as usize;
    let j = (i + 1).min(n - 1);
    curve[i] + (curve[j] - curve[i]) * (at - i as f32)
}

/// A biquad's coefficients, normalised, from Web Audio's formulas (the Audio EQ
/// Cookbook, with the low and high passes' Q in decibels).
#[derive(Clone, Copy, Default)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    fn new(filter: Filter, freq: f32, q: f32, gain_db: f32) -> Self {
        let nyquist = RATE_F / 2.;
        let f = (freq / nyquist).clamp(1e-5, 0.9999);
        let w0 = std::f32::consts::PI * f;
        let (sin, cos) = w0.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match filter {
            Filter::Lowpass | Filter::Highpass => {
                let alpha = sin / (2. * 10f32.powf(q / 20.));
                let (b0, b1) = if filter == Filter::Lowpass {
                    ((1. - cos) / 2., 1. - cos)
                } else {
                    ((1. + cos) / 2., -(1. + cos))
                };
                (b0, b1, b0, 1. + alpha, -2. * cos, 1. - alpha)
            }
            Filter::Bandpass => {
                let alpha = sin / (2. * q.max(1e-4));
                (alpha, 0., -alpha, 1. + alpha, -2. * cos, 1. - alpha)
            }
            Filter::Peaking => {
                let a = 10f32.powf(gain_db / 40.);
                let alpha = sin / (2. * q.max(1e-4));
                (
                    1. + alpha * a,
                    -2. * cos,
                    1. - alpha * a,
                    1. + alpha / a,
                    -2. * cos,
                    1. - alpha / a,
                )
            }
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// One sample through, with the last two inputs and outputs in `state`.
    fn run(&self, x: f32, state: &mut [f32; 4]) -> f32 {
        let [x1, x2, y1, y2] = *state;
        let y = self.b0 * x + self.b1 * x1 + self.b2 * x2 - self.a1 * y1 - self.a2 * y2;
        *state = [x, x1, y, y1];
        if y.is_finite() { y } else { 0. }
    }
}

/// Some samples of one or two channels, from sample `start`.
#[derive(Clone, Debug, Default)]
struct Signal {
    start: usize,
    channels: Vec<Vec<f32>>,
}

impl Signal {
    fn mono(start: usize, samples: Vec<f32>) -> Self {
        Self {
            start,
            channels: vec![samples],
        }
    }

    fn len(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn end(&self) -> usize {
        self.start + self.len()
    }

    fn extend(&mut self, by: usize) {
        for channel in &mut self.channels {
            channel.resize(channel.len() + by, 0.);
        }
    }

    /// Signals added together, mono ones going to both sides of stereo ones.
    fn sum<'a>(signals: impl Iterator<Item = &'a Signal>) -> Signal {
        let signals: Vec<&Signal> = signals.filter(|signal| !signal.is_empty()).collect();
        let Some(start) = signals.iter().map(|signal| signal.start).min() else {
            return Signal::default();
        };
        if signals.len() == 1 {
            return signals[0].clone();
        }
        let end = signals
            .iter()
            .map(|signal| signal.end())
            .max()
            .unwrap_or(start);
        let count = signals
            .iter()
            .map(|signal| signal.channels.len())
            .max()
            .unwrap_or(1);
        let mut channels = vec![vec![0.; end - start]; count];
        for signal in signals {
            for (c, channel) in channels.iter_mut().enumerate() {
                let source = &signal.channels[c.min(signal.channels.len() - 1)];
                let offset = signal.start - start;
                for (out, sample) in channel[offset..offset + source.len()]
                    .iter_mut()
                    .zip(source)
                {
                    *out += sample;
                }
            }
        }
        Signal { start, channels }
    }

    /// Add this, mixed down to mono, to `values` (which start at sample `from`).
    fn add_mono_to(&self, values: &mut [f32], from: usize) {
        let count = self.channels.len() as f32;
        for (i, value) in values.iter_mut().enumerate() {
            let at = from + i;
            if at < self.start || at >= self.end() {
                continue;
            }
            let k = at - self.start;
            *value += self.channels.iter().map(|channel| channel[k]).sum::<f32>() / count;
        }
    }
}

// ---- Reverb ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    fn mul(self, other: Complex) -> Complex {
        Complex {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }
}

/// In-place radix-2 FFT of a power-of-two length; `inverse` without the 1/n.
fn fft(data: &mut [Complex], inverse: bool) {
    let n = data.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            data.swap(i, j);
        }
    }
    let sign = if inverse { 1. } else { -1. };
    let mut len = 2;
    while len <= n {
        let angle = sign * std::f64::consts::TAU / len as f64;
        let twiddles: Vec<Complex> = (0..len / 2)
            .map(|k| {
                let (sin, cos) = (angle * k as f64).sin_cos();
                Complex {
                    re: cos as f32,
                    im: sin as f32,
                }
            })
            .collect();
        for chunk in data.chunks_mut(len) {
            let (low, high) = chunk.split_at_mut(len / 2);
            for ((a, b), w) in low.iter_mut().zip(high.iter_mut()).zip(&twiddles) {
                let t = b.mul(*w);
                *b = Complex {
                    re: a.re - t.re,
                    im: a.im - t.im,
                };
                *a = Complex {
                    re: a.re + t.re,
                    im: a.im + t.im,
                };
            }
        }
        len <<= 1;
    }
}

/// A response's two channels' spectra at FFT size `n`, scaled as Web Audio normalises a
/// convolver's buffer; kept, since the same few are used again and again.
fn spectra(response: &Response, n: usize) -> Arc<(Vec<Complex>, Vec<Complex>)> {
    thread_local! {
        static SPECTRA: RefCell<HashMap<(usize, usize), Arc<(Vec<Complex>, Vec<Complex>)>>> = Default::default();
    }
    let key = (Arc::as_ptr(response) as usize, n);
    SPECTRA.with(|spectra| {
        if let Some(found) = spectra.borrow().get(&key) {
            return found.clone();
        }
        // Web Audio's normalisation: to a fixed power, calibrated at 44.1 kHz.
        let len = response[0].len();
        let power = (response
            .iter()
            .flat_map(|channel| channel.iter())
            .map(|s| s * s)
            .sum::<f32>()
            / (2 * len.max(1)) as f32)
            .sqrt()
            .max(0.000125);
        let scale = 0.00125 / power * 44_100. / RATE_F;
        let spectrum = |channel: &[f32]| {
            let mut data = vec![Complex::default(); n];
            for (out, s) in data.iter_mut().zip(channel) {
                out.re = s * scale;
            }
            fft(&mut data, false);
            data
        };
        let found = Arc::new((spectrum(&response[0]), spectrum(&response[1])));
        let mut spectra = spectra.borrow_mut();
        if spectra.len() > 16 {
            spectra.clear();
        }
        spectra.insert(key, found.clone());
        found
    })
}

/// `signal` through a convolver with `response`: mono in, or each side through its own
/// channel of the response; stereo out.
fn convolve(signal: &Signal, response: &Response) -> Signal {
    let len = signal.len() + response[0].len();
    let n = len.next_power_of_two();
    let (left, right) = &*spectra(response, n);
    // Both sides in one transform: left as the real part, right as the imaginary.
    let mut data = vec![Complex::default(); n];
    let stereo = signal.channels.len() > 1;
    for (i, out) in data.iter_mut().take(signal.len()).enumerate() {
        out.re = signal.channels[0][i];
        if stereo {
            out.im = signal.channels[1][i];
        }
    }
    fft(&mut data, false);
    let mut mixed = vec![Complex::default(); n];
    for k in 0..n {
        let x = data[k];
        let (xl, xr) = if stereo {
            let y = data[(n - k) % n];
            (
                Complex {
                    re: (x.re + y.re) / 2.,
                    im: (x.im - y.im) / 2.,
                },
                Complex {
                    re: (x.im + y.im) / 2.,
                    im: (y.re - x.re) / 2.,
                },
            )
        } else {
            (x, x)
        };
        let (l, r) = (xl.mul(left[k]), xr.mul(right[k]));
        // Left out as the real part, right as the imaginary, since both are real.
        mixed[k] = Complex {
            re: l.re - r.im,
            im: l.im + r.re,
        };
    }
    fft(&mut mixed, true);
    let scale = 1. / n as f32;
    Signal {
        start: signal.start,
        channels: vec![
            mixed[..len].iter().map(|c| c.re * scale).collect(),
            mixed[..len].iter().map(|c| c.im * scale).collect(),
        ],
    }
}

// ---- Master ---------------------------------------------------------------------------

/// The master's compressor, as Web Audio's `DynamicsCompressor` set up by the
/// prototypes (with its makeup gain and a little look-ahead), then a limiter that keeps
/// every sample under full scale.
fn compress(channels: &mut [Vec<f32>; 2]) {
    let (threshold, knee, ratio) = (-14f32, 30f32, 6f32);
    let curve = |x: f32| -> f32 {
        if x < threshold {
            x
        } else if x < threshold + knee {
            x + (1. / ratio - 1.) * (x - threshold).powi(2) / (2. * knee)
        } else {
            threshold + knee * (1. + 1. / ratio) / 2. + (x - threshold - knee) / ratio
        }
    };
    let makeup = -0.6 * curve(0.);
    let attack = (-1. / (0.003 * RATE_F)).exp();
    let release = (-1. / (0.2 * RATE_F)).exp();
    let ahead = (0.005 * RATE_F) as usize;
    let len = channels[0].len();
    let mut reduction = 0f32;
    let mut gains = Vec::with_capacity(len);
    for i in 0..len {
        let level = channels[0][i].abs().max(channels[1][i].abs()).max(1e-6);
        let x = 20. * level.log10();
        let wanted = curve(x) - x;
        let k = if wanted < reduction { attack } else { release };
        reduction = wanted + (reduction - wanted) * k;
        gains.push(10f32.powf((reduction + makeup) / 20.));
    }
    for channel in channels.iter_mut() {
        for i in 0..len {
            let gain = gains[(i + ahead).min(len.saturating_sub(1))];
            channel[i] = limit(channel[i] * gain);
        }
    }
}

/// Soft above 0.9, and never reaching 1.
fn limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.09 * ((a - 0.9) / 0.09).tanh())
    }
}

/// `samples` (interleaved stereo at [`RATE`]) as a WAV file.
pub fn wav(samples: &[i16]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    // PCM, two channels.
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(samples: &[i16]) -> i32 {
        samples
            .iter()
            .map(|s| i32::from(*s).abs())
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn params_follow_web_audio_schedules() {
        let mut param = Param::new(1.);
        param
            .set(0.0001, 0.)
            .exp(1., 0.01)
            .linear(0.5, 0.02)
            .set(0.25, 0.03);
        let at = |s: f32| param.values((s * RATE_F) as usize, 1)[0];
        assert!((at(0.) - 0.0001).abs() < 1e-6);
        // Half-way up an exponential ramp is the geometric mean.
        assert!((at(0.005) - 0.01).abs() < 0.001, "{}", at(0.005));
        assert!((at(0.015) - 0.75).abs() < 0.01);
        assert!((at(0.04) - 0.25).abs() < 1e-6);
        let mut target = Param::new(0.);
        target.set(1., 0.).target(0., 0.1, 0.05);
        let t = |s: f32| target.values((s * RATE_F) as usize, 1)[0];
        assert!((t(0.05) - 1.).abs() < 1e-6);
        assert!((t(0.15) - (-1f32).exp()).abs() < 0.01, "{}", t(0.15));
    }

    #[test]
    fn a_tone_sounds_and_stays_under_full_scale() {
        let mut graph = Graph::new(1);
        graph.tone(Tone {
            freq: 220.,
            wave: Wave::Saw,
            gain: 3.,
            dur: 0.5,
            ..Tone::default()
        });
        let samples = graph.render();
        assert!(peak(&samples) > 20_000, "{}", peak(&samples));
        assert!(peak(&samples) < i32::from(i16::MAX));
        // The hall rings on after it.
        assert!(samples.len() / 2 > RATE as usize, "{}", samples.len());
    }

    #[test]
    fn filters_pass_what_they_should() {
        let level = |filter: Filter, freq: f32| {
            let mut graph = Graph::new(2);
            let o = graph.osc(Wave::Sine, freq);
            let f = graph.filter(filter, 1000., 1.);
            let to = graph.dest(0.);
            graph.connect(o, f);
            graph.connect(f, to);
            graph.play(o, 0., 0.3);
            peak(&graph.render())
        };
        assert!(level(Filter::Lowpass, 200.) > 3 * level(Filter::Lowpass, 6000.));
        assert!(level(Filter::Highpass, 6000.) > 3 * level(Filter::Highpass, 200.));
        assert!(level(Filter::Bandpass, 1000.) > 3 * level(Filter::Bandpass, 8000.));
    }

    #[test]
    fn the_hall_rings_on_at_a_sensible_level() {
        let mut graph = Graph::new(3);
        graph.noise(Noise {
            dur: 0.1,
            gain: 0.5,
            wet: 1.,
            ..Noise::default()
        });
        let samples = graph.render();
        let tail = &samples[(RATE as usize / 2) * 2..RATE as usize * 2];
        let loud = peak(&samples[..(RATE as usize / 10) * 2]);
        // Quieter than the burst, but there (above −50 dBFS).
        assert!(peak(tail) > 100, "{}", peak(tail));
        assert!(peak(tail) < loud, "{} {loud}", peak(tail));
    }

    #[test]
    fn convolution_matches_the_direct_sum() {
        let mut rng = Rng::new(5);
        let response: Response = Arc::new([
            (0..300).map(|_| rng.range(-1., 1.)).collect(),
            (0..300).map(|_| rng.range(-1., 1.)).collect(),
        ]);
        let input: Vec<f32> = (0..200).map(|_| rng.range(-1., 1.)).collect();
        let out = convolve(&Signal::mono(0, input.clone()), &response);
        let power = (response.iter().flatten().map(|s| s * s).sum::<f32>() / 600.).sqrt();
        let scale = 0.00125 / power * 44_100. / RATE_F;
        for (c, channel) in out.channels.iter().enumerate() {
            for n in [0, 57, 250, 480] {
                let direct: f32 = (0..=n)
                    .filter(|&k| k < input.len() && n - k < 300)
                    .map(|k| input[k] * response[c][n - k] * scale)
                    .sum();
                assert!(
                    (channel[n] - direct).abs() < 1e-4,
                    "{c} {n}: {} {direct}",
                    channel[n]
                );
            }
        }
    }

    #[test]
    fn wav_files_are_whole() {
        let bytes = wav(&[0, 1, -1, 2]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 8);
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
            bytes.len() - 8
        );
    }
}
