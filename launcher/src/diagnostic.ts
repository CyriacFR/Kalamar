// "Copy the diagnostic": the technical details a problem used to show on screen (log excerpt, codes, versions), put
// in the clipboard for a bug report instead of being displayed. The Windows user name in paths and the room codes are
// replaced (`redact.ts`).
import { api, type AppInfo, type GameStatus, type InstallStatus } from "./api";
import { redact } from "./redact";

export { redact };

export interface DiagSource { info: AppInfo | null; status: InstallStatus | null; game?: GameStatus | null; lang?: string }

const field = (k: string, v: unknown) => (v === null || v === undefined || v === "" ? null : `${k}: ${v}`);

/** The report: what happened (`what`), extra lines of the caller, the game process, the installation and the end of
 *  the game's log (read now, when there is one). */
export async function diagnostic(src: DiagSource, what: string, extra: (string | null | undefined)[] = [], withLog = true): Promise<string> {
  const g = src.game;
  const s = src.status;
  const lines: (string | null | undefined)[] = [
    `${src.info?.name ?? "Kalamar"} ${src.info?.version ?? "?"} · ${new Date().toISOString()} · ${src.lang ?? ""} · ${navigator.userAgent.includes("Windows") ? "Windows" : navigator.platform}`,
    `problem: ${what}`,
    ...extra,
    g && g.state !== "idle" ? `game: ${[field("state", g.state), field("mode", g.mode), field("exit_code", g.exit_code), field("abnormal", g.abnormal), field("stopped_by_user", g.stopped_by_user), field("pid", g.pid)].filter(Boolean).join(", ")}` : null,
    s ? `install: ready=${s.ready}${s.missing.length ? `, missing=${s.missing.join("+")}` : ""}${s.build_date ? `, built=${new Date(s.build_date * 1000).toISOString().slice(0, 10)}` : ""}` : null,
    s?.last_log ? `last log: ${s.last_log.name} (${s.last_log.status})` : null,
    ...(s?.last_log?.problem_lines ?? []).map((l) => `  ${l}`),
  ];
  if (withLog) {
    let tail: string | null = g?.tail ?? null;
    if (!tail) {
      try {
        tail = await api.readLogTail(60);
      } catch {
        tail = null;
      }
    }
    if (tail) lines.push("", "--- end of the game log ---", tail);
  }
  return redact(lines.filter((l): l is string => l !== null && l !== undefined).join("\n"));
}

/** Puts text in the clipboard (the WebView's API, else a hidden text area). */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch {
      ok = false;
    }
    ta.remove();
    return ok;
  }
}
