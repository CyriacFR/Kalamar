// Versus stages as the game shows them: each stage's own picture and name, read from the player's game files by the
// backend (`gamedata_stages`, the catalogue's stages; pictures through the `gameicon` protocol at two widths). Nothing
// of them is in the launcher: without game data the cards are neutral and named from the engine's list, the server or
// the id itself.
//
// The pieces: `StageCard` (a 16:9 picture with the name over it; selected / banned / unavailable), `StagePicker` (a grid
// of cards to choose one stage or ban several), `StageField` (a form's stage: the chosen card and the picker in a
// dialog), `StageTag` (a small picture and the name, inside a sentence) and `StageStrip` (a row of small cards).
import { useEffect, useState, useSyncExternalStore, type KeyboardEvent, type ReactNode } from "react";
import { api, type StagesView } from "./api";
import { iconUrl, pickName } from "./gamedata";
import { useI18n } from "./i18n";
import { IconBan, IconCheck, IconLock, IconShuffle } from "./icons";
import { Modal } from "./ui";

// ---- the stages of the player's game data (asked once, shared) ------------------------------------------------------

let view: StagesView | null = null;
let askedAt = 0;
let pending = false;
const listeners = new Set<() => void>();
const subscribe = (f: () => void) => { listeners.add(f); return () => { listeners.delete(f); }; };

/** Asks the backend for the stages: once, and again after a while when there was no game data (the game may have
 *  been installed since). Building the catalogue the first time can take a few seconds; the cards are neutral until. */
export function loadStages(): void {
  if (pending || (view && (view.origin === "game" || Date.now() - askedAt < 30_000))) return;
  pending = true;
  askedAt = Date.now();
  api.stages()
    .then((v) => { view = v; })
    .catch(() => { view = view ?? { stages: [], origin: "neutral", reason: "unavailable", sizes: [] }; })
    .finally(() => { pending = false; listeners.forEach((f) => f()); });
}

/** The stages, asked for when `load` (a screen that shows no stage does not read the game data). */
export function useStagesView(load = true): StagesView | null {
  useEffect(() => { if (load) loadStages(); }, [load]);
  return useSyncExternalStore(subscribe, () => view);
}

/** True when the player's game data has stage pictures: a grid of cards then replaces the drop-down list. */
export function useStagePictures(load = true): boolean {
  return !!useStagesView(load)?.stages.some((s) => s.image);
}

// ---- names and pictures --------------------------------------------------------------------------------------------

/** An id made readable when nothing names it: the scene prefix and version digits dropped, words split. */
export function readableStage(id: string): string {
  const words = id.replace(/^Vss_/, "").replace(/\d+$/, "").replace(/_/g, " ").replace(/([a-z])([A-Z])/g, "$1 $2").trim();
  return words || id;
}

export interface StageInfo {
  id: string;
  name: string;
  /** The game (or the given fallback) names it; false: the readable id. */
  named: boolean;
  /** The picture at the card size, and the miniature; null without game data. */
  picture: string | null;
  thumb: string | null;
}

export function stageInfo(v: StagesView | null, id: string, lang: string, fallback?: string): StageInfo {
  const s = v?.stages.find((x) => x.scene === id);
  const own = s ? pickName(s.name, lang) : "";
  const sizes = v?.sizes ?? [];
  const at = (w: number | undefined) => (s?.image && w ? iconUrl(`${s.image}@${w}`) : null);
  const [big, small] = sizes.length ? [Math.max(...sizes), Math.min(...sizes)] : [undefined, undefined];
  return { id, name: own || fallback || readableStage(id), named: !!(own || fallback), picture: at(big), thumb: at(small) };
}

/** `(id, fallback name) => StageInfo` in the interface language; re-renders when the stages arrive (asked for unless
 *  `load` is false: names alone then wait for a card to ask). */
export function useStageInfo(load = true): (id: string, fallback?: string) => StageInfo {
  const v = useStagesView(load);
  const { lang } = useI18n();
  return (id, fallback) => stageInfo(v, id, lang, fallback);
}

/** A hue from the id, so that neutral cards differ from one another (nothing taken from the game). */
function hueOf(id: string): number {
  let h = 0x811c9dc5;
  for (const ch of id) h = Math.imul(h ^ ch.charCodeAt(0), 0x01000193) >>> 0;
  return Math.round(((h % 1000) / 1000) * 360);
}

// ---- one card -------------------------------------------------------------------------------------------------------

export type StageMark = "selected" | "banned" | "unavailable";

/** A stage as a card: its picture in 16:9 with the name over a dark gradient, or a neutral card. `id` null: the game's
 *  choice (a card of its own). A card with `onClick` is a button. */
export function StageCard({ id, fallback, size = "md", mark, note, onClick, className, role, checked, label, autoFocus, onKeyDown, sub }: {
  id: string | null;
  /** The name when the game data has none (the engine's label, the server's), or the game's-choice label. */
  fallback?: string;
  size?: "sm" | "md" | "lg";
  mark?: StageMark;
  /** Why it is unavailable (else "Unavailable"), or who banned it (else "Banned"). */
  note?: string;
  onClick?: () => void;
  className?: string;
  role?: "radio" | "checkbox";
  checked?: boolean;
  /** The accessible name when it differs from the visible one. */
  label?: string;
  autoFocus?: boolean;
  onKeyDown?: (e: KeyboardEvent<HTMLButtonElement>) => void;
  /** A second line under the name. */
  sub?: ReactNode;
}) {
  const { t } = useI18n();
  const info = useStageInfo();
  const s = id ? info(id, fallback) : null;
  const [broken, setBroken] = useState<string | null>(null);
  const src = s ? (size === "sm" ? s.thumb ?? s.picture : s.picture ?? s.thumb) : null;
  const shown = src && broken !== src ? src : null;
  const name = s ? s.name : fallback || t("stage.random");
  const cls = `stc stc-${size} ${mark ? `stc-${mark}` : ""} ${shown ? "" : "stc-plain"} ${onClick ? "stc-btn" : ""} ${className ?? ""}`;
  const body = (
    <>
      <span className="stc-media" aria-hidden="true" style={shown ? undefined : { ["--stc-h" as string]: id ? hueOf(id) : 170 }}>
        {shown ? (
          <img src={shown} alt="" loading="lazy" decoding="async" draggable={false} onError={() => setBroken(shown)}
            {...(size === "sm" && s?.thumb && s.picture && shown === s.thumb ? { srcSet: `${s.thumb} 160w, ${s.picture} 480w`, sizes: "10rem" } : {})} />
        ) : id ? (
          <span className="stc-blank"><span className="stc-blank-mark" /></span>
        ) : (
          <span className="stc-blank stc-random"><IconShuffle className="stc-random-ico" /></span>
        )}
      </span>
      <span className="stc-text">
        <span className="stc-name">{name}</span>
        {sub && <span className="stc-sub">{sub}</span>}
      </span>
      {mark === "selected" && <span className="stc-check" aria-hidden="true"><IconCheck /></span>}
      {mark === "banned" && <span className="stc-flag stc-flag-ban"><IconBan /> {note ?? t("stage.banned")}</span>}
      {mark === "unavailable" && <span className="stc-flag"><IconLock /> {note ?? t("stage.unavailable")}</span>}
    </>
  );
  if (!onClick) return <div className={cls} title={s && !s.named ? s.id : undefined}>{body}</div>;
  return (
    <button
      type="button" className={cls} onClick={onClick} onKeyDown={onKeyDown} role={role} aria-checked={role ? !!checked : undefined}
      aria-label={label} aria-disabled={mark === "unavailable" || undefined} disabled={mark === "unavailable"}
      data-autofocus={autoFocus || undefined} title={s && !s.named ? s.id : undefined}
    >
      {body}
    </button>
  );
}

// ---- a grid to choose or ban ------------------------------------------------------------------------------------------

/** Arrow keys move between the cards of a grid (the columns counted from the layout). */
function gridKeys(e: KeyboardEvent<HTMLButtonElement>) {
  const grid = e.currentTarget.parentElement;
  if (!grid) return;
  const cards = [...grid.querySelectorAll<HTMLButtonElement>("button.stc:not(:disabled)")];
  const i = cards.indexOf(e.currentTarget);
  const cols = Math.max(1, getComputedStyle(grid).gridTemplateColumns.split(" ").filter(Boolean).length);
  const step = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols, Home: -i, End: cards.length - 1 - i }[e.key];
  if (step === undefined || i < 0) return;
  e.preventDefault();
  cards[Math.max(0, Math.min(cards.length - 1, i + step))]?.focus();
}

/** A grid of stage cards. Choose mode (`onPick`): one stage, or the game's choice first when `nullable`. Ban mode
 *  (`onBan`): each card toggles its ban. `unavailable` cards are shown grayed with why, and cannot be chosen. */
export function StagePicker({ ids, value, onPick, nullable, nullLabel, banned = [], onBan, unavailable = {}, fallback, label, compact }: {
  ids: string[];
  value?: string | null;
  onPick?: (id: string | null) => void;
  nullable?: boolean;
  nullLabel?: string;
  banned?: string[];
  onBan?: (id: string, banned: boolean) => void;
  unavailable?: Record<string, string>;
  fallback?: (id: string) => string | undefined;
  label: string;
  /** Smaller cards (more per row). */
  compact?: boolean;
}) {
  const banMode = !!onBan;
  const chosen = value ?? null;
  // The stages that can be chosen first, the grayed ones after them (each group in the given order).
  const order = [...ids.filter((x) => !(x in unavailable)), ...ids.filter((x) => x in unavailable)];
  const focusId = banMode ? order[0] : chosen;
  return (
    <div className={`stp ${compact ? "stp-compact" : ""}`} role={banMode ? "group" : "radiogroup"} aria-label={label}>
      {!banMode && nullable && (
        <StageCard id={null} fallback={nullLabel} role="radio" checked={chosen === null} mark={chosen === null ? "selected" : undefined}
          onClick={() => onPick?.(null)} onKeyDown={gridKeys} autoFocus={chosen === null} />
      )}
      {order.map((id) => {
        const off = id in unavailable;
        const isBanned = banned.includes(id);
        const mark: StageMark | undefined = off ? "unavailable" : banMode ? (isBanned ? "banned" : undefined) : chosen === id ? "selected" : undefined;
        return (
          <StageCard key={id} id={id} fallback={fallback?.(id)} mark={mark} note={unavailable[id] || undefined}
            role={banMode ? "checkbox" : "radio"} checked={banMode ? isBanned : chosen === id}
            onClick={() => (banMode ? onBan?.(id, !isBanned) : onPick?.(id))} onKeyDown={gridKeys} autoFocus={!off && id === focusId} />
        );
      })}
    </div>
  );
}

/** A form's stage: the chosen card (or the game's choice) and a button that opens the picker in a dialog. `id` goes on
 *  the button, for the field's label. */
export function StageField({ id, ids, value, onChange, nullable, nullLabel, unavailable, fallback, label }: {
  id: string;
  ids: string[];
  value: string | null | undefined;
  onChange: (v: string | null) => void;
  nullable?: boolean;
  nullLabel?: string;
  unavailable?: Record<string, string>;
  fallback?: (id: string) => string | undefined;
  label: string;
}) {
  const { t } = useI18n();
  const info = useStageInfo();
  const [open, setOpen] = useState(false);
  const cur = value ?? null;
  const count = ids.filter((x) => !unavailable || !(x in unavailable)).length;
  const off = cur !== null && !!unavailable && cur in unavailable;
  const name = cur ? info(cur, fallback?.(cur)).name : nullLabel || t("stage.random");
  return (
    <div className="stf">
      <StageCard id={cur} fallback={cur ? fallback?.(cur) : nullLabel} className="stf-card" onClick={off ? undefined : () => setOpen(true)}
        label={`${name} — ${t("stage.change")}`} sub={cur ? undefined : t("stage.random.sub")} mark={off ? "unavailable" : undefined} note={off ? unavailable?.[cur!] : undefined} />
      <div className="stf-side">
        <button id={id} type="button" className="btn btn-small btn-ghost" onClick={() => setOpen(true)}>{t("stage.choose")}</button>
        <span className="hint">{t("stage.count", { n: count })}</span>
      </div>
      {open && (
        <Modal wide title={label} labelledBy={`${id}-title`} onClose={() => setOpen(false)}
          actions={<button type="button" className="btn btn-small btn-ghost" onClick={() => setOpen(false)}>{t("common.close")}</button>}>
          <StagePicker ids={ids} value={cur} nullable={nullable} nullLabel={nullLabel} unavailable={unavailable} fallback={fallback} label={label}
            onPick={(v) => { onChange(v); setOpen(false); }} />
        </Modal>
      )}
    </div>
  );
}

// ---- inline and rows ----------------------------------------------------------------------------------------------------

/** A stage inside a sentence or a value: its miniature and its name. */
export function StageTag({ id, fallback, className }: { id: string | null; fallback?: string; className?: string }) {
  const { t } = useI18n();
  const info = useStageInfo();
  const s = id ? info(id, fallback) : null;
  const [broken, setBroken] = useState(false);
  const src = s?.thumb && !broken ? s.thumb : null;
  return (
    <span className={`stg ${className ?? ""}`} title={s && !s.named ? s.id : undefined}>
      <span className={`stg-pic ${src ? "" : "stg-plain"}`} aria-hidden="true" style={src ? undefined : { ["--stc-h" as string]: id ? hueOf(id) : 170 }}>
        {src ? <img src={src} alt="" loading="lazy" decoding="async" draggable={false} onError={() => setBroken(true)} /> : !id ? <IconShuffle /> : null}
      </span>
      <span className="stg-name">{s ? s.name : fallback || t("stage.random")}</span>
    </span>
  );
}

/** One row of small stage cards (a queue's pool, a rotation): the first `max`, and "+n" on the last one for the rest. */
export function StageStrip({ ids, fallback, max = 3, label }: { ids: (string | null)[]; fallback?: (id: string) => string | undefined; max?: number; label?: string }) {
  const { t } = useI18n();
  const shown = ids.slice(0, max);
  const more = ids.length - shown.length;
  return (
    <div className="sts" role="list" aria-label={label} style={{ ["--sts-n" as string]: shown.length }}>
      {shown.map((id, i) => (
        <div role="listitem" key={id ?? `null-${i}`} className="sts-item">
          <StageCard id={id} fallback={id ? fallback?.(id) : undefined} size="sm" />
          {more > 0 && i === shown.length - 1 && <span className="sts-more">{t("stage.more", { n: more })}</span>}
        </div>
      ))}
    </div>
  );
}
