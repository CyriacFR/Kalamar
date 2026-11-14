// What a copied diagnostic must not carry: the Windows account name in paths, and the room codes (the relay room is the
// only key of a match's room: whoever has it can join it while it is open).

/** Hides the account name of `C:\Users\<name>\...` paths. */
export function redactUser(text: string): string {
  return text.replace(/([A-Za-z]:[\\/]+Users[\\/]+)[^\\/\s"']+/gi, "$1<user>");
}

/** Hides match codes (`KLM1-...`), the relay room and match id given to the game, a room named in a log line, and any
 *  relay room or match id of 20 to 32 symbols of the room alphabet (no 0, 1, I or O). */
export function redactRooms(text: string): string {
  return text
    .replace(/\b(KLM\d)(?:-[0-9A-Z]{1,5})+/gi, "$1-<code>")
    .replace(/\b((?:SPLAT3|KLM|TOTK)_(?:LAN_ROOM|MATCH_ID|MATCH_CODE)=)[^\s"']+/g, "$1<room>")
    .replace(/\b(room[ \t]*[:=][ \t]*|room[ \t]+)([A-Za-z0-9][A-Za-z0-9-]{3,39})/gi, "$1<room>")
    .replace(/\b[A-HJ-NP-Z2-9]{20,32}(?:-\d{1,3})?\b/g, "<room>");
}

export function redact(text: string): string {
  return redactRooms(redactUser(text));
}
