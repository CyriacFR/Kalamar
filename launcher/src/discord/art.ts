// The small pictures of the Discord presence: original drawings of this launcher (the same strokes as src/icons.tsx),
// a dark glyph on a mint disc. One source for both places they appear: the Settings preview draws them from here, and
// `scripts/discord-assets.mjs` turns them into the PNG files the maintainers upload to the Discord application under these
// exact keys (docs/discord-presence.md). The large picture is the launcher's logo (app-icon.svg), key `kalamar_logo`.
// No picture of the game, ever.

export interface Glyph {
  /** Stroked paths on a 24x24 grid (round caps and joins). */
  paths: string[];
  /** Filled shapes on the same grid. */
  fills?: string[];
  /** Stroked circles: [cx, cy, r]. */
  circles?: [number, number, number][];
}

export const ART_BG = "#2ee6c0";
export const ART_FG = "#0c0f17";
export const ART_STROKE = 2.2;

export const GLYPHS: Record<string, Glyph> = {
  /** The game, offline or in training. */
  act_solo: { paths: [], fills: ["M8 5.200v13.600L18.800 12 8 5.200Z"] },
  /** Online, in the server's menus. */
  act_online: {
    paths: ["M7.200 7.200a6.800 6.800 0 0 0 0 9.600", "M16.800 7.200a6.800 6.800 0 0 1 0 9.600", "M4.200 4.200a11 11 0 0 0 0 15.600", "M19.800 4.200a11 11 0 0 1 0 15.600"],
    circles: [[12, 12, 2.200]],
  },
  /** In a private room. */
  act_room: {
    paths: ["M3 19.500c.4-3.300 2.800-5.200 6-5.200s5.600 1.900 6 5.200", "M16 5.800a3.200 3.200 0 0 1 0 5.800", "M18.200 14.700c1.600.7 2.600 2.300 2.800 4.800"],
    circles: [[9, 8.500, 3.200]],
  },
  /** Looking for a match. */
  act_search: { paths: ["m15.200 15.200 5 5"], circles: [[10.500, 10.500, 6.200]] },
  /** In a match (also the stage draft, the local network and a private match by code). */
  act_match: { paths: ["m4 4 7 7", "M20 4l-7 7", "M4 20l6-6", "M20 20l-6-6", "M10.500 13.500 13.500 10.500"] },
  /** In the stands. */
  act_watch: { paths: ["M2.500 12s3.500-6.500 9.500-6.500S21.500 12 21.500 12s-3.500 6.500-9.500 6.500S2.500 12 2.500 12Z"], circles: [[12, 12, 2.800]] },
  /** The match is over. */
  act_done: { paths: ["M6 21V3.500", "M6 4h11.500l-2.500 4.250L17.500 12.500H6"] },
};

/** One picture as a standalone SVG document (`size` pixels square): the disc, then the glyph at 60 % of it. */
export function glyphSvg(key: string, size: number): string {
  const g = GLYPHS[key];
  if (!g) throw new Error(`no picture ${key}`);
  const s = (size * 0.6) / 24;
  const o = (size - 24 * s) / 2;
  const stroke = `fill="none" stroke="${ART_FG}" stroke-width="${ART_STROKE}" stroke-linecap="round" stroke-linejoin="round"`;
  const body = [
    ...g.paths.map((d) => `<path d="${d}" ${stroke}/>`),
    ...(g.circles ?? []).map(([cx, cy, r]) => `<circle cx="${cx}" cy="${cy}" r="${r}" ${stroke}/>`),
    ...(g.fills ?? []).map((d) => `<path d="${d}" fill="${ART_FG}"/>`),
  ].join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}">`
    + `<circle cx="${size / 2}" cy="${size / 2}" r="${size / 2}" fill="${ART_BG}"/>`
    + `<g transform="translate(${o} ${o}) scale(${s})">${body}</g></svg>`;
}
