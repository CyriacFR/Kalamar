// The interface sounds as plain data: each one is a few short voices (a sine or triangle gliding from one pitch to
// another, or a burst of filtered noise) under a fast attack and an exponential decay. Everything is synthesised at run
// time by engine.ts; nothing is recorded, sampled or taken from a game. The identity is a soft "drop in water" plop.

export type SoundName = "confirm" | "found" | "soft" | "success" | "error";

export interface Voice {
  kind: "sine" | "triangle" | "noise";
  /** Start, seconds after the sound begins. */
  at: number;
  /** Length, seconds (the envelope reaches silence at the end). */
  dur: number;
  /** Pitch at the start and at the end, Hz (noise: the band-pass centre). */
  from: number;
  to: number;
  /** Peak level, 0 to 1, before the master volume. */
  peak: number;
  /** Attack, seconds. */
  attack: number;
  /** Low-pass cut-off, Hz (omitted: none). */
  lowpass?: number;
}

export interface Recipe {
  voices: Voice[];
  /** Higher interrupts a lower one that is still playing; equal or lower is dropped while one plays. */
  priority: 1 | 2 | 3;
  /** Whether it may sound while the game runs in front (only the "your game is starting" sound). */
  duringGame: boolean;
}

export const RECIPES: Record<SoundName, Recipe> = {
  // Main button: one small plop, 130 ms.
  confirm: { priority: 1, duringGame: false, voices: [{ kind: "sine", at: 0, dur: 0.13, from: 520, to: 300, peak: 0.5, attack: 0.004 }] },
  // Match found / the game is starting: two plops falling one after the other (the second a little higher) and a faint
  // splash of noise; 260 ms. The only sound allowed to stand out.
  found: {
    priority: 3, duringGame: true, voices: [
      { kind: "sine", at: 0, dur: 0.15, from: 620, to: 340, peak: 0.7, attack: 0.004 },
      { kind: "sine", at: 0.09, dur: 0.17, from: 780, to: 430, peak: 0.6, attack: 0.004 },
      { kind: "noise", at: 0, dur: 0.05, from: 1800, to: 1800, peak: 0.12, attack: 0.002, lowpass: 3000 },
    ],
  },
  // Somebody joined the room or is ready: a tiny, high drip, 70 ms.
  soft: { priority: 1, duringGame: false, voices: [{ kind: "sine", at: 0, dur: 0.07, from: 700, to: 520, peak: 0.22, attack: 0.003 }] },
  // Done / saved: one bubble rising, with its quieter octave, 190 ms.
  success: {
    priority: 2, duringGame: false, voices: [
      { kind: "sine", at: 0, dur: 0.19, from: 380, to: 720, peak: 0.42, attack: 0.006 },
      { kind: "sine", at: 0.02, dur: 0.15, from: 760, to: 1440, peak: 0.08, attack: 0.006 },
    ],
  },
  // Blocking error: one low, muffled thud, 170 ms.
  error: { priority: 3, duringGame: false, voices: [{ kind: "triangle", at: 0, dur: 0.17, from: 200, to: 130, peak: 0.5, attack: 0.008, lowpass: 600 }] },
};

export const SOUND_NAMES = Object.keys(RECIPES) as SoundName[];

/** Total length of a sound, seconds. */
export function lengthOf(name: SoundName): number {
  return Math.max(...RECIPES[name].voices.map((v) => v.at + v.dur));
}

/** The master gain for a volume setting, 0 to 100 (perceptual curve: low settings stay really quiet). */
export function gainFor(volume: number): number {
  const v = Math.min(100, Math.max(0, Number.isFinite(volume) ? volume : 0)) / 100;
  return Math.pow(v, 1.5) * 0.9;
}

/** Minimum gap between two sounds started, ms (no bursts). */
export const MIN_GAP_MS = 150;
