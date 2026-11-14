// The single entry point of the interface sounds: `play("confirm")`. Web Audio synthesis only (see recipes.ts).
// Rules kept here and nowhere else: off switch and volume, nothing before the first user interaction, one sound at a
// time (a higher priority cuts a lower one, anything else is dropped), no burst within MIN_GAP_MS, and silence while the
// game runs in front except for the sound that announces it.
import { MIN_GAP_MS, RECIPES, gainFor, lengthOf, type SoundName, type Voice } from "./recipes";

let enabled = true;
let volume = 35;
let gameRunning = false;
let interacted = false;
let ctx: AudioContext | null = null;
let noise: AudioBuffer | null = null;
let current: { until: number; priority: number; master: GainNode } | null = null;
let lastStart = -Infinity;

/** 1.1.7: the player watches a match from the stands: no "your game starts" sound (nothing is the player's to do). */
let spectating = false;
export function setSpectating(on: boolean): void {
  spectating = on;
}
export function isSpectating(): boolean {
  return spectating;
}

export function configureSound(opts: { enabled?: boolean; volume?: number }): void {
  if (opts.enabled !== undefined) enabled = opts.enabled;
  if (opts.volume !== undefined) volume = opts.volume;
}

export function setGameRunning(on: boolean): void {
  gameRunning = on;
}

if (typeof window !== "undefined") {
  const mark = () => {
    interacted = true;
    window.removeEventListener("pointerdown", mark, true);
    window.removeEventListener("keydown", mark, true);
  };
  window.addEventListener("pointerdown", mark, true);
  window.addEventListener("keydown", mark, true);
}

function context(): AudioContext | null {
  try {
    if (!ctx) {
      const C = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!C) return null;
      ctx = new C();
    }
    if (ctx.state === "suspended") void ctx.resume().catch(() => {});
    return ctx;
  } catch {
    return null;
  }
}

function noiseBuffer(c: BaseAudioContext): AudioBuffer {
  if (!noise || noise.sampleRate !== c.sampleRate) {
    noise = c.createBuffer(1, Math.ceil(c.sampleRate * 0.3), c.sampleRate);
    const d = noise.getChannelData(0);
    let s = 12345;
    for (let i = 0; i < d.length; i++) {
      s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
      d[i] = s / 2147483648 - 1;
    }
  }
  return noise;
}

/** Builds one voice on `c` into `out`, starting at `t0`. Also used by the offline render of the checks. */
export function addVoice(c: BaseAudioContext, out: AudioNode, v: Voice, t0: number): void {
  const t = t0 + v.at;
  const end = t + v.dur;
  const env = c.createGain();
  env.gain.setValueAtTime(0.0001, t);
  env.gain.linearRampToValueAtTime(v.peak, t + v.attack);
  env.gain.exponentialRampToValueAtTime(0.0001, end);
  let head: AudioNode;
  let src: OscillatorNode | AudioBufferSourceNode;
  if (v.kind === "noise") {
    const s = c.createBufferSource();
    s.buffer = noiseBuffer(c);
    const band = c.createBiquadFilter();
    band.type = "bandpass";
    band.frequency.value = v.from;
    band.Q.value = 0.8;
    s.connect(band);
    head = band;
    src = s;
  } else {
    const o = c.createOscillator();
    o.type = v.kind;
    o.frequency.setValueAtTime(v.from, t);
    o.frequency.exponentialRampToValueAtTime(v.to, end);
    head = o;
    src = o;
  }
  if (v.lowpass) {
    const lp = c.createBiquadFilter();
    lp.type = "lowpass";
    lp.frequency.value = v.lowpass;
    head.connect(lp);
    head = lp;
  }
  head.connect(env);
  env.connect(out);
  src.start(t);
  src.stop(end + 0.02);
}

/** Plays an interface sound if the rules allow it. Never throws. `force`: the "Test" button (ignores the switch). */
export function play(name: SoundName, opts: { force?: boolean } = {}): void {
  try {
    const r = RECIPES[name];
    const now = performance.now();
    if (current && now >= current.until) current = null;
    if (!opts.force) {
      if (!enabled || !interacted) return;
      if (gameRunning && !r.duringGame) return;
      // One at a time: only a more important sound may cut the one still playing; no burst either.
      if (current && r.priority <= current.priority) return;
      if (now - lastStart < MIN_GAP_MS && !(current && r.priority > current.priority)) return;
    }
    const c = context();
    if (!c) return;
    if (current) {
      // The more important sound cuts the one still playing, with a very short fade (no click).
      const g = current.master.gain;
      g.cancelScheduledValues(c.currentTime);
      g.setValueAtTime(g.value, c.currentTime);
      g.linearRampToValueAtTime(0, c.currentTime + 0.015);
    }
    const master = c.createGain();
    master.gain.value = gainFor(volume);
    master.connect(c.destination);
    const t0 = c.currentTime + 0.01;
    for (const v of r.voices) addVoice(c, master, v, t0);
    current = { until: now + lengthOf(name) * 1000 + 20, priority: r.priority, master };
    lastStart = now;
  } catch {
    // a sound is a nicety: never let it break the interface
  }
}
