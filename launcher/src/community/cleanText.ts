// What a server sends is shown as text, never as markup; this removes what plain text can still hide.

/** Unicode format characters (category Cf) that reorder or hide text: bidirectional embeddings, overrides, isolates and
 *  marks, invisible spaces, tags. The zero-width joiner and non-joiner stay (emoji sequences, Persian, Indic scripts). */
const HIDDEN = /(?![‌‍])\p{Cf}/gu;

/** Every string of a server's answer or push without hidden format characters, before the page renders it: a server or
 *  another player cannot reorder what the player reads around a name ("official" shown for another text). Keys too;
 *  numbers, booleans and null as they are. */
export function cleanText<T>(v: T, depth = 0): T {
  if (typeof v === "string") return v.replace(HIDDEN, "") as T;
  if (v === null || typeof v !== "object" || depth > 64) return v;
  if (Array.isArray(v)) return v.map((x) => cleanText(x, depth + 1)) as T;
  const out: Record<string, unknown> = {};
  for (const [k, x] of Object.entries(v as Record<string, unknown>)) out[k.replace(HIDDEN, "")] = cleanText(x, depth + 1);
  return out as T;
}
