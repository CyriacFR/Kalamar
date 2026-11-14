// FALLBACK ONLY: hand-written RGB values (not from the game), used when the catalogue has no colors for an entry
// (no game data, or the creation screen's color animation could not be read). The real colors are read from the
// player's own files at run time (`colors` in the catalogue). Index = choice number on the creation screen (0-based).
export const SKIN_APPROX: string[] = [
  "#fbe3d0", "#f4cfae", "#e9b78e", "#d89c6c", "#c08057", "#a4653f", "#84492c", "#5f3220", "#3e2116",
];
export const EYES_APPROX: string[] = [
  "#2b2b33", "#5a3a22", "#8a5a2b", "#c8962e", "#e8d44d", "#a8d63c", "#4fb84a", "#2e9a6a", "#26a8a0", "#2fb4d8",
  "#3a86e0", "#3a52d0", "#6a45c8", "#9a45c0", "#c845b0", "#e0457f", "#e0453f", "#e87a2e", "#b8b8c0", "#f2f2f2", "#6c7480",
];
