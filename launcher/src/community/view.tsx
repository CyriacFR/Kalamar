// Server views (`activities.md` 3): declarative screens made of the protocol's closed list of data components,
// rendered generically. A view is data, never code: every text is plain text (React escapes it; nothing here sets
// HTML), no URL is fetched, links open in the system browser only on a click and show their host, no image is shown,
// and sizes are bounded here too. An unknown component shows its `fallback` text or nothing; an unknown member, style
// or level falls back to the default. Nothing a view says can start the game, change a setting or call another API.
import { useEffect, useRef, useState } from "react";
import { proto, type J } from "./api";
import { myId, req, useLive } from "./store";
import { duration, useC } from "./texts";
import { ErrorLine, FieldForm, Tier, Who, fieldDefaults, fieldBody, signed, unsupportedField, useAction, useLeft } from "./ui";
import { ProtoIcon, safeColor } from "./icons";
import { MatchCard } from "./Match";

export const LIMITS = { bytes: 256 * 1024, components: 500, depth: 4, items: 500, actions: 50, text: 2000, label: 120, rounds: 8, roundMatches: 128 };

/** Why a view cannot be shown (over the protocol's limits), or null. */
export function checkView(view: J): string | null {
  if (!view || typeof view !== "object") return "not a view";
  let size = 0;
  try {
    size = JSON.stringify(view).length;
  } catch {
    return "not a view";
  }
  if (size > LIMITS.bytes) return `larger than ${LIMITS.bytes / 1024} KiB`;
  let components = 0;
  let actions = Array.isArray(view.actions) ? view.actions.length : 0;
  let problem: string | null = null;
  const walk = (list: J, depth: number) => {
    if (!Array.isArray(list)) return;
    for (const c of list) {
      components++;
      if (!c || typeof c !== "object") continue;
      if (c.type === "group") {
        if (depth >= LIMITS.depth) problem = `groups nested deeper than ${LIMITS.depth}`;
        else walk(c.children, depth + 1);
      }
      if ((c.type === "list" || c.type === "table") && ((Array.isArray(c.items) && c.items.length > LIMITS.items) || (Array.isArray(c.rows) && c.rows.length > LIMITS.items))) problem = `more than ${LIMITS.items} items`;
      if (c.type === "actions" && Array.isArray(c.items)) actions += c.items.length;
      if (c.type === "form" && c.submit) actions++;
      if (c.type === "bracket" && Array.isArray(c.rounds) && (c.rounds.length > LIMITS.rounds || c.rounds.some((r: J) => Array.isArray(r?.matches) && r.matches.length > LIMITS.roundMatches))) problem = "a bracket over its limits";
    }
  };
  walk(view.sections, 1);
  if (components > LIMITS.components) return `more than ${LIMITS.components} components`;
  if (actions > LIMITS.actions) return `more than ${LIMITS.actions} actions`;
  return problem;
}

/** Server text, plain, cut at the protocol's limit. */
export function useText() {
  const { L } = useC();
  return (x: J, max: number = LIMITS.text): string => {
    const s = typeof x === "string" ? x : typeof x === "number" ? String(x) : L(x);
    return s.length > max ? `${s.slice(0, max)}…` : s;
  };
}

export interface ViewHandlers {
  /** Runs an action (a button, a form's submit); absent: the view is read-only (its actions are not shown). */
  onAction?: (action: J, values?: Record<string, unknown>) => void;
  /** Refetches the view (a countdown reached zero). */
  onRefresh?: () => void;
  busy?: boolean;
}

function ActionButton({ a, h, values }: { a: J; h: ViewHandlers; values?: () => Record<string, unknown> | undefined }) {
  const T = useText();
  if (!h.onAction || !a || typeof a !== "object") return null;
  const cls = a.style === "danger" ? "btn btn-danger" : a.style === "secondary" ? "btn btn-ghost" : "btn";
  const why = a.enabled === false ? T(a.disabled_reason, LIMITS.label) : "";
  return (
    <span className="cm-act">
      <button
        className={cls}
        disabled={a.enabled === false || h.busy}
        title={why || undefined}
        onClick={() => {
          if (a.confirm && !window.confirm(T(a.confirm))) return;
          h.onAction?.(a, values?.());
        }}
      >
        {T(a.label, LIMITS.label) || String(a.id ?? "")}
      </button>
      {why && <span className="hint">{why}</span>}
    </span>
  );
}

/** Cells of a table (the leaderboard column types; unknown: text when it is text). */
export function Cell({ type, v }: { type: string; v: J }) {
  const T = useText();
  switch (type) {
    case "player":
      return <>{v && typeof v === "object" ? String(v.display_name ?? "") : ""}</>;
    case "tier":
      return <Tier tier={v} />;
    case "percent":
      return <>{typeof v === "number" ? `${Math.round(v * 10) / 10} %` : ""}</>;
    case "delta":
      return <>{typeof v === "number" ? signed(v) : ""}</>;
    default:
      return <>{typeof v === "string" || typeof v === "number" ? T(v, LIMITS.label) : v && typeof v === "object" && !Array.isArray(v) ? T(v, LIMITS.label) : ""}</>;
  }
}

function Countdown({ c, h }: { c: J; h: ViewHandlers }) {
  const { c: tc } = useC();
  const T = useText();
  const left = useLeft(typeof c.until === "string" ? c.until : null);
  const fired = useRef(false);
  useEffect(() => {
    if (left === 0 && !fired.current) {
      fired.current = true;
      h.onRefresh?.();
    }
  }, [left, h]);
  return (
    <div className="cm-countdown-row">
      <span>{T(c.label, LIMITS.label)}</span>
      <strong className="cm-countdown" role="timer">{left === null ? "—" : duration(tc, left)}</strong>
    </div>
  );
}

function Progress({ c }: { c: J }) {
  const T = useText();
  const max = typeof c.max === "number" && c.max > 0 ? c.max : 1;
  const value = typeof c.value === "number" ? Math.max(0, Math.min(c.value, max)) : 0;
  return (
    <div>
      <div className="row"><span className="grow">{T(c.label, LIMITS.label)}</span><span className="small muted">{value} / {max}</span></div>
      <div className="cm-progress" role="progressbar" aria-valuemin={0} aria-valuemax={max} aria-valuenow={value}><span style={{ width: `${(value / max) * 100}%` }} /></div>
    </div>
  );
}

/** The series card (`series.md`): sides, wins, games. Fetched by id, then kept fresh by `series.updated`. */
export function SeriesCard({ id, title }: { id: string; title?: J }) {
  const { c, L } = useC();
  const T = useText();
  const live = useLive();
  const [fetched, setFetched] = useState<J | null>(null);
  const a = useAction();
  useEffect(() => {
    if (!id) return;
    void a.run(async () => setFetched(await req("GET", `/v1/series/${encodeURIComponent(id)}`)));
  }, [id]); // eslint-disable-line react-hooks/exhaustive-deps
  const s = live.series[id] ?? fetched;
  if (!s) return a.error ? <ErrorLine text={a.error} /> : <span className="skeleton" style={{ width: "100%", height: "3rem" }} />;
  const f = s.format ?? {};
  const fmt = f.kind === "best_of" ? c("sr.best_of", { n: f.games ?? "?" }) : f.kind === "first_to" ? c("sr.first_to", { n: f.wins ?? "?" }) : f.kind === "fixed" ? c("sr.fixed", { n: f.games ?? "?" }) : f.games ? c("sr.fixed", { n: f.games }) : "";
  const sides: J[] = Array.isArray(s.sides) ? s.sides.slice(0, 2) : [];
  const me = myId();
  return (
    <article className="cm-series">
      <header className="row">
        <h3 className="grow">{T(title, LIMITS.label) || c("sr.title")} <span className="muted small">{fmt}</span></h3>
        <span className={`pill ${s.state === "completed" ? "pill-ok" : s.state === "void" ? "pill-bad" : "pill-neutral"}`}>{c(s.state === "completed" ? "sr.completed" : s.state === "void" ? "sr.void" : "sr.running")}</span>
      </header>
      <div className="cm-series-score">
        {sides.map((side: J, i: number) => (
          <div key={i} className={`cm-side ${s.winner && s.winner === side.team ? "cm-side-win" : ""} ${(side.users ?? []).some((u: J) => u?.id === me) ? "cm-me" : ""}`}>
            <span className="cm-side-name">{T(side.label, LIMITS.label) || (side.users ?? []).map((u: J) => u?.display_name).join(", ")}</span>
            <span className="cm-side-wins display">{typeof side.wins === "number" ? side.wins : 0}</span>
          </div>
        ))}
      </div>
      {Array.isArray(s.games) && s.games.length > 0 && (
        <ol className="cm-games">
          {s.games.slice(0, 50).map((g: J) => (
            <li key={`${g.index}-${g.match_id}`} className={`cm-game cm-game-${g.state}`} title={L(g.state)}>
              {c("sr.game", { n: g.index ?? "?" })}
              {g.winner ? ` · ${String(sides.find((x: J) => x.team === g.winner)?.label ? T(sides.find((x: J) => x.team === g.winner).label, 40) : g.winner)}` : g.state === "void" ? ` · ${c("sr.void")}` : ""}
            </li>
          ))}
        </ol>
      )}
    </article>
  );
}

/** A match card by id (the launcher's own view of a match). */
function MatchById({ id }: { id: string }) {
  const live = useLive();
  const [m, setM] = useState<J | null>(null);
  const a = useAction();
  useEffect(() => {
    if (!id) return;
    if (live.match?.id === id) return;
    void a.run(async () => setM(await req("GET", `/v1/matches/${encodeURIComponent(id)}`)));
  }, [id, live.match?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const match = live.match?.id === id ? live.match : live.ended?.id === id ? live.ended : m;
  if (!match) return a.error ? <ErrorLine text={a.error} /> : null;
  return <MatchCard match={match} compact />;
}

function Bracket({ c }: { c: J }) {
  const { c: tc } = useC();
  const T = useText();
  const [open, setOpen] = useState<string | null>(null);
  const rounds: J[] = Array.isArray(c.rounds) ? c.rounds : [];
  return (
    <div className="cm-bracket-wrap">
      <div className="cm-bracket" role="list">
        {rounds.map((r: J, i: number) => (
          <div key={i} className="cm-bracket-round" role="listitem">
            <h4>{T(r.title, LIMITS.label) || tc("br.round", { n: i + 1 })}</h4>
            <div className="cm-bracket-matches">
            {(Array.isArray(r.matches) ? r.matches : []).map((m: J, k: number) => (
              <div key={m.id ?? k} className={`cm-bm cm-bm-${["running", "completed"].includes(m.state) ? m.state : "waiting"}`}>
                {(Array.isArray(m.slots) ? m.slots.slice(0, 2) : []).map((s: J, j: number) => (
                  <div key={j} className={`cm-slot ${s.winner === true ? "cm-slot-win" : ""}`}>
                    <span className="grow">{T(s.label, LIMITS.label) || (s.user?.display_name ?? "—")}</span>
                    {(typeof s.score === "number" || typeof s.score === "string") && <span className="cm-slot-score">{String(s.score)}</span>}
                  </div>
                ))}
                {m.series_id && <button className="link small" onClick={() => setOpen(open === m.series_id ? null : m.series_id)}>{tc("br.series")}</button>}
              </div>
            ))}
            </div>
          </div>
        ))}
      </div>
      {open && <SeriesCard id={open} />}
    </div>
  );
}

function UserCard({ c }: { c: J }) {
  const T = useText();
  const badges: J[] = Array.isArray(c.badges) ? c.badges.slice(0, 20) : [];
  return (
    <div className="cm-usercard">
      <Who user={c.user ?? {}} me={c.user?.id === myId()} />
      {c.subtitle && <span className="small muted">{T(c.subtitle, LIMITS.label)}</span>}
      {badges.length > 0 && <div className="cm-chips">{badges.map((b: J, i: number) => <BadgeChip key={b?.id ?? i} b={b} />)}</div>}
    </div>
  );
}

export function BadgeChip({ b }: { b: J }) {
  const T = useText();
  if (!b || typeof b !== "object") return null;
  return (
    <span className="chip cm-badge" title={T(b.description, LIMITS.label)} style={safeColor(b.color) ? { borderColor: safeColor(b.color) } : undefined}>
      <ProtoIcon name={b.icon} style={safeColor(b.color) ? { color: safeColor(b.color) } : undefined} />
      {T(b.name, LIMITS.label) || String(b.id ?? "")}
    </span>
  );
}

function LinkC({ c }: { c: J }) {
  const { c: tc } = useC();
  const T = useText();
  let host = "";
  try {
    const u = new URL(String(c.url ?? ""));
    if (u.protocol === "https:") host = u.host;
  } catch {
    host = "";
  }
  if (!host) return <p className="muted small">{T(c.label, LIMITS.label)}</p>;
  return (
    <p className="cm-link">
      <button className="link" onClick={() => void proto.openLink(String(c.url)).catch(() => {})}>{T(c.label, LIMITS.label) || host}</button>
      <span className="small muted"> ({tc("v.opens", { host })})</span>
    </p>
  );
}

function Form({ c, h, forms }: { c: J; h: ViewHandlers; forms: FormRegistry }) {
  const { c: tc } = useC();
  const live = useLive();
  const fields: J[] = Array.isArray(c.fields) ? c.fields.slice(0, 50) : [];
  const [values, setValues] = useState<Record<string, unknown>>(() => fieldDefaults(fields));
  const id = typeof c.id === "string" ? c.id : null;
  useEffect(() => {
    if (id) forms.set(id, fieldBody(fields, values));
  }, [id, values]); // eslint-disable-line react-hooks/exhaustive-deps
  const unknown = unsupportedField(fields);
  if (unknown) return <p className="bad-text">{tc("field.too_old", { key: unknown })}</p>;
  return (
    <div className="cm-viewform">
      <FieldForm fields={fields} values={values} set={(k, v) => setValues((o) => ({ ...o, [k]: v }))} meta={live.overview?.meta} idPrefix={`vf-${id ?? "f"}`} />
      {c.submit && <ActionButton a={c.submit} h={h} values={() => fieldBody(fields, values)} />}
    </div>
  );
}

/** One component. `depth`: the group nesting. */
function Component({ c, h, depth, forms }: { c: J; h: ViewHandlers; depth: number; forms: FormRegistry }) {
  const T = useText();
  if (!c || typeof c !== "object") return null;
  const fallback = c.fallback ? <p className="muted">{T(c.fallback)}</p> : null;
  switch (c.type) {
    case "text": {
      const t = T(c.text);
      if (c.style === "heading") return <h3 className="cm-v-heading">{t}</h3>;
      if (c.style === "caption") return <p className="small">{t}</p>;
      if (c.style === "muted") return <p className="muted">{t}</p>;
      return <p className="cm-v-text">{t}</p>;
    }
    case "notice": {
      const level = ["info", "success", "warning", "error"].includes(c.level) ? c.level : "info";
      return <p className={`notice ${level === "success" ? "notice-ok" : level === "error" ? "notice-error" : level === "warning" ? "notice-warn" : "notice-info"}`}>{T(c.text)}</p>;
    }
    case "fields":
      return (
        <dl className="cm-fields">
          {(Array.isArray(c.items) ? c.items.slice(0, LIMITS.items) : []).map((f: J, i: number) => (
            <div key={i}><dt>{T(f?.label, LIMITS.label)}</dt><dd>{T(f?.value, LIMITS.label)}</dd>{f?.hint && <span className="hint">{T(f.hint, LIMITS.label)}</span>}</div>
          ))}
        </dl>
      );
    case "list": {
      const items: J[] = Array.isArray(c.items) ? c.items.slice(0, LIMITS.items) : [];
      if (!items.length) return c.empty ? <p className="muted">{T(c.empty)}</p> : null;
      return (
        <ul className="cm-list">
          {items.map((it: J, i: number) => (
            <li key={i} className="cm-row">
              <ProtoIcon name={it?.icon} />
              <span className="grow cm-li">
                {it?.user ? <Who user={it.user} me={it.user?.id === myId()} /> : <strong>{T(it?.title, LIMITS.label)}</strong>}
                {it?.user && it?.title && <span className="small"> {T(it.title, LIMITS.label)}</span>}
                {it?.subtitle && <span className="small muted cm-sub">{T(it.subtitle, LIMITS.label)}</span>}
              </span>
              {it?.trailing !== undefined && it?.trailing !== null && <span className="cm-trailing">{T(it.trailing, LIMITS.label)}</span>}
              {it?.action && <ActionButton a={it.action} h={h} values={() => forms.values(it.action?.form)} />}
            </li>
          ))}
        </ul>
      );
    }
    case "table": {
      const cols: J[] = Array.isArray(c.columns) ? c.columns.slice(0, 20) : [];
      const rows: J[] = Array.isArray(c.rows) ? c.rows.slice(0, LIMITS.items) : [];
      return (
        <div className="cm-table-wrap">
          <table className="cm-table">
            {c.caption && <caption>{T(c.caption, LIMITS.label)}</caption>}
            <thead><tr>{cols.map((col: J, i: number) => <th key={i} className={`cm-col-${col?.type}`}>{T(col?.label, LIMITS.label)}</th>)}</tr></thead>
            <tbody>
              {rows.map((r: J, i: number) => (
                <tr key={i} className={`${r?.highlight || (r?.user_id && r.user_id === myId()) ? "cm-me" : ""}`}>
                  {cols.map((col: J, k: number) => <td key={k} className={`cm-col-${col?.type}`}><Cell type={String(col?.type ?? "text")} v={r?.cells?.[col?.key]} /></td>)}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }
    case "countdown":
      return <Countdown c={c} h={h} />;
    case "progress":
      return <Progress c={c} />;
    case "bracket":
      return <Bracket c={c} />;
    case "series":
      return typeof c.series_id === "string" ? <SeriesCard id={c.series_id} title={c.title} /> : fallback;
    case "match":
      return typeof c.match_id === "string" ? (
        <div>
          {c.title && <h4>{T(c.title, LIMITS.label)}</h4>}
          <MatchById id={c.match_id} />
        </div>
      ) : fallback;
    case "user":
      return <UserCard c={c} />;
    case "form":
      return <Form c={c} h={h} forms={forms} />;
    case "actions":
      return h.onAction ? (
        <div className="row wrap">{(Array.isArray(c.items) ? c.items.slice(0, LIMITS.actions) : []).map((a: J, i: number) => <ActionButton key={a?.id ?? i} a={a} h={h} values={() => forms.values(a?.form)} />)}</div>
      ) : null;
    case "link":
      return <LinkC c={c} />;
    case "group": {
      const body = depth >= LIMITS.depth ? null : <Components list={c.children} h={h} depth={depth + 1} forms={forms} />;
      return c.collapsed ? (
        <details className="cm-group"><summary>{T(c.title, LIMITS.label)}</summary>{body}</details>
      ) : (
        <section className="cm-group">{c.title && <h3>{T(c.title, LIMITS.label)}</h3>}{body}</section>
      );
    }
    default:
      return fallback;
  }
}

/** Values of the forms of a view, for actions that name a form (`form`). */
interface FormRegistry {
  values: (id: unknown) => Record<string, unknown> | undefined;
  set: (id: string, values: Record<string, unknown>) => void;
}

function Components({ list, h, depth, forms }: { list: J; h: ViewHandlers; depth: number; forms: FormRegistry }) {
  if (!Array.isArray(list)) return null;
  return <>{list.slice(0, LIMITS.components).map((c: J, i: number) => <div key={c?.id ?? i} className="cm-comp"><Component c={c} h={h} depth={depth} forms={forms} /></div>)}</>;
}

/** A whole view: its sections and its actions; an error card when it is over the protocol's limits. */
export function ViewBody({ view, h }: { view: J; h: ViewHandlers }) {
  const { c } = useC();
  const problem = checkView(view);
  if (problem) return <p className="notice notice-error">{c("v.too_big", { why: problem })}</p>;
  // Forms keep their values here so that any action naming a form (`form`) submits that form's values.
  const store = useRef<Record<string, Record<string, unknown>>>({});
  const forms: FormRegistry = { values: (id) => (typeof id === "string" ? store.current[id] : undefined), set: (id, v) => { store.current[id] = v; } };
  const actions: J[] = Array.isArray(view.actions) ? view.actions.slice(0, LIMITS.actions) : [];
  return (
    <div className="cm-view stack">
      <Components list={view.sections} h={h} depth={1} forms={forms} />
      {h.onAction && actions.length > 0 && <div className="row wrap cm-view-actions">{actions.map((a: J, i: number) => <ActionButton key={a?.id ?? i} a={a} h={h} values={() => forms.values(a?.form)} />)}</div>}
    </div>
  );
}

