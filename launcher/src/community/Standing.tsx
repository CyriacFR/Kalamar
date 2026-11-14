// Ratings, leaderboards and statistics (`ratings`, `leaderboards`, `stats`): the server's presentable data, drawn
// without interpreting it (no formula, no tier list of this launcher's own), and the account.
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { serversApi } from "../api";
import { useApp } from "../ctx";
import { AddServerModal } from "../servers/AddServer";
import { proto, type J } from "./api";
import { BadgeChip, ViewBody } from "./view";
import { Modal } from "../ui";
import { eraseAccount, exportData, has, logout, myId, req, setLive, useLive } from "./store";
import { duration, leftForfeit, useC, type CKey } from "./texts";
import { ErrorLine, Tier, fmtDay, signed, useAction, useLeft, useModeName } from "./ui";

function RatingView({ r }: { r: J }) {
  const { c, L } = useC();
  const shown = r?.display ?? (typeof r?.value === "number" ? String(r.value) : null);
  return (
    <div className="cm-rating">
      <div className="cm-rating-main">
        {shown && <span className="cm-rating-value display">{shown}</span>}
        <Tier tier={r?.tier} />
      </div>
      {r?.progress && typeof r.progress.max === "number" && r.progress.max > 0 && (
        <div className="cm-progress" role="progressbar" aria-valuemin={0} aria-valuemax={r.progress.max} aria-valuenow={r.progress.value ?? 0}>
          <span style={{ width: `${Math.min(100, Math.max(0, ((r.progress.value ?? 0) / r.progress.max) * 100))}%` }} />
        </div>
      )}
      {L(r?.progress?.label) && <p className="small muted">{L(r.progress.label)}</p>}
      {r?.provisional?.remaining > 0 && <p className="small">{L(r.provisional.label) || r.provisional.remaining}</p>}
      {r?.last_change && typeof r.last_change.delta === "number" && <p className="small muted">{c("r.last", { delta: signed(r.last_change.delta) })} {L(r.last_change.reason)}</p>}
      {Array.isArray(r?.fields) && r.fields.length > 0 && (
        <dl className="cm-fields">
          {r.fields.map((f: J) => <div key={f.key}><dt>{L(f.label) || f.key}</dt><dd>{typeof f.value === "object" ? "" : String(f.value)}</dd></div>)}
        </dl>
      )}
    </div>
  );
}

function Cell({ col, v }: { col: J; v: J }) {
  const { L } = useC();
  switch (col?.type) {
    case "player":
      return <>{v?.display_name ?? ""}</>;
    case "tier":
      return <Tier tier={v} />;
    case "percent":
      return <>{typeof v === "number" ? `${Math.round(v * 10) / 10} %` : ""}</>;
    case "delta":
      return <>{typeof v === "number" ? signed(v) : ""}</>;
    case "rank":
    case "number":
    case "text":
    default:
      return <>{typeof v === "string" || typeof v === "number" ? String(v) : typeof v === "object" && v && !Array.isArray(v) && L(v) ? L(v) : ""}</>;
  }
}

function Leaderboard({ ladder }: { ladder: J }) {
  const { c, L } = useC();
  const [board, setBoard] = useState<J | null>(null);
  const [rows, setRows] = useState<J[]>([]);
  const a = useAction();
  const load = (cursor?: string) => void a.run(async () => {
    const b = await req("GET", `/v1/ladders/${encodeURIComponent(ladder.id)}/leaderboard?limit=25${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`);
    setBoard(b);
    setRows((old) => (cursor ? [...old, ...(b.items ?? [])] : (b.items ?? [])));
  });
  useEffect(() => { load(); }, [ladder.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const cols: J[] = board?.columns ?? [];
  const me = myId();
  return (
    <div className="cm-board">
      <h3>{c("r.board")}</h3>
      <ErrorLine text={a.error} />
      {board && rows.length === 0 && <p className="muted">{c("r.board.empty")}</p>}
      {rows.length > 0 && (
        <table className="cm-table">
          <thead><tr>{cols.map((col) => <th key={col.key} className={`cm-col-${col.type}`}>{L(col.label) || col.key}</th>)}</tr></thead>
          <tbody>
            {rows.map((row: J, i: number) => (
              <tr key={`${row.user_id}-${i}`} className={row.user_id === me ? "cm-me" : ""}>
                {cols.map((col) => <td key={col.key} className={`cm-col-${col.type}`}><Cell col={col} v={row.cells?.[col.key]} /></td>)}
              </tr>
            ))}
            {board?.you && !rows.some((r: J) => r.user_id === board.you.user_id) && (
              <tr className="cm-me cm-you">{cols.map((col) => <td key={col.key} className={`cm-col-${col.type}`}><Cell col={col} v={board.you.cells?.[col.key]} /></td>)}</tr>
            )}
          </tbody>
        </table>
      )}
      {board?.next_cursor && <button className="btn btn-small btn-ghost" disabled={a.busy} onClick={() => load(board.next_cursor)}>{c("r.more")}</button>}
    </div>
  );
}

export function Ratings() {
  const { c, L } = useC();
  const live = useLive();
  const [ladders, setLadders] = useState<J[] | null>(null);
  const [mine, setMine] = useState<J[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const a = useAction();
  const me = myId();
  useEffect(() => {
    void a.run(async () => {
      const raw: J[] = (await req("GET", "/v1/ladders")).items ?? [];
      // 1.1: the primary ladder first, then by the server's groups (in the order they appear).
      const groups: string[] = [];
      for (const x of raw) if (!groups.includes(L(x.group))) groups.push(L(x.group));
      const l = [...raw].sort((x: J, y: J) => Number(!!y.primary) - Number(!!x.primary) || groups.indexOf(L(x.group)) - groups.indexOf(L(y.group)));
      setLadders(l);
      setOpen((o) => o ?? l[0]?.id ?? null);
      if (me) setMine((await req("GET", `/v1/users/${encodeURIComponent(me ?? "")}/ratings`)).items ?? []);
    });
  }, [live.rev.ratings, me]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="stack">
      <ErrorLine text={a.error} />
      {ladders === null ? <span className="skeleton" style={{ width: "100%", height: "5rem" }} /> : ladders.length === 0 ? <p className="muted">{c("r.none")}</p> : (
        <>
          <div className="seg cm-ladders" role="tablist">
            {ladders.map((l: J) => (
              <button key={l.id} role="tab" aria-selected={open === l.id} className={`seg-btn ${open === l.id ? "on" : ""}`} onClick={() => setOpen(l.id)}>
                {L(l.group) && <span className="cm-tab-group">{L(l.group).slice(0, 40)} · </span>}
                {L(l.name) || l.id}
                {l.primary === true && <span className="cm-tab-primary"> ★ {c("r.primary")}</span>}
              </button>
            ))}
          </div>
          {ladders.filter((l: J) => l.id === open).map((l: J) => (
            <div key={l.id} className="cm-split">
              <article className="card">
                <h2>{c("r.mine")}</h2>
                {L(l.description) && <p className="muted small">{L(l.description)}</p>}
                {l.season && <p className="small">{c("r.season", { name: L(l.season.name) || l.season.id })}</p>}
                {mine.find((r: J) => r.ladder === l.id) ? <RatingView r={mine.find((r: J) => r.ladder === l.id)} /> : <p className="muted">{c("r.unranked")}</p>}
              </article>
              {has("leaderboards") && l.leaderboard !== false && <article className="card"><Leaderboard ladder={l} /></article>}
            </div>
          ))}
        </>
      )}
    </div>
  );
}

export function Stats() {
  const { c, L, lang } = useC();
  const live = useLive();
  const modeName = useModeName();
  const me = myId();
  const [stats, setStats] = useState<J | null>(null);
  const [hist, setHist] = useState<J[]>([]);
  const [next, setNext] = useState<string | null>(null);
  const a = useAction();
  const loadHist = (cursor?: string) => void a.run(async () => {
    const h = await req("GET", `/v1/users/${encodeURIComponent(me ?? "")}/matches?limit=20${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`);
    setHist((old) => (cursor ? [...old, ...(h.items ?? [])] : (h.items ?? [])));
    setNext(h.next_cursor ?? null);
  });
  useEffect(() => {
    if (!me) return;
    void a.run(async () => setStats(await req("GET", `/v1/users/${encodeURIComponent(me ?? "")}/stats`)));
    loadHist();
  }, [live.rev.stats, me]); // eslint-disable-line react-hooks/exhaustive-deps
  const res = (r: string) => (r === "win" ? c("s.win") : r === "loss" ? c("s.loss") : c("s.void"));
  // 1.1.11: a match won or lost because a player left it in play: the result, and why on a second line.
  const forfeitCell = (m: J) => {
    if (m.result !== "win" && m.result !== "loss") return null;
    const duel = Array.isArray(m.rules?.teams) && m.rules.teams[0] === 1 && m.rules.teams[1] === 1;
    const f = leftForfeit(c, m.forfeit, m.team, me, duel);
    if (!f) return null;
    // One block: the cell of a win is a flex row (`.ok-text`).
    return <div>{m.result === "win" ? c("s.forfeit.win") : c("s.forfeit.loss")}<div className="small muted cm-forfeit-why">{f.why}</div></div>;
  };
  return (
    <div className="cm-split">
      <div className="stack">
      <ErrorLine text={a.error} />
      <h2 className="section-title">{c("s.title")}</h2>
      {stats && (stats.groups ?? []).length === 0 && <p className="muted">{c("s.none")}</p>}
      {(stats?.groups ?? []).map((g: J) => (
        <article key={g.key} className="card">
          <h3>{L(g.label) || g.key}</h3>
          <dl className="cm-fields">{(g.fields ?? []).map((f: J) => <div key={f.key}><dt>{L(f.label) || f.key}</dt><dd>{typeof f.value === "object" ? "" : String(f.value)}</dd></div>)}</dl>
        </article>
      ))}
      </div>
      <div className="stack">
      <h2 className="section-title">{c("s.history")}</h2>
      {hist.length === 0 ? <p className="muted">{c("s.history.none")}</p> : (
        <table className="cm-table">
          <tbody>
            {hist.map((m: J) => (
              <tr key={m.match_id}>
                <td className="small muted">{fmtDay(m.ended_at, lang)}</td>
                <td>{modeName(m.rules?.mode, live.overview?.meta)}{m.rules?.rated ? ` · ${c("q.rated")}` : ""}</td>
                <td className={m.result === "win" ? "ok-text" : m.result === "loss" ? "bad-text" : "muted"}>{forfeitCell(m) ?? res(m.result)}</td>
                <td className="small">{m.stats ? `${m.stats.paint ?? 0}p · ${m.stats.kills ?? 0}/${m.stats.deaths ?? 0}` : ""}</td>
                <td className="small">{typeof m.rating_delta === "number" ? signed(m.rating_delta) : ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {next && <button className="btn btn-small btn-ghost" disabled={a.busy} onClick={() => loadHist(next)}>{c("s.more")}</button>}
      </div>
    </div>
  );
}

function Restriction({ r }: { r: J }) {
  const { c, L, lang } = useC();
  const left = useLeft(r?.until);
  let text: string;
  if (r?.kind === "ban") text = r.until ? c("acct.restr.ban.until", { date: fmtDay(r.until, lang) }) : c("acct.restr.ban");
  else if (r?.kind === "queue_cooldown") text = c("acct.restr.queue_cooldown", { left: duration(c, left ?? 0) });
  else text = L(r?.reason) || String(r?.kind ?? "");
  return <li className="bad-text">{text}{r?.kind !== "queue_cooldown" && L(r?.reason) && r?.kind === "ban" ? ` — ${L(r.reason)}` : ""}</li>;
}

/** Badges earned on this server (`badges`, 1.1): the featured one and the title shown next to the name. */
function Badges() {
  const { c, L, lang } = useC();
  const live = useLive();
  const me = myId();
  const [mine, setMine] = useState<J | null>(null);
  const [titles, setTitles] = useState<J[]>([]);
  const a = useAction();
  useEffect(() => {
    if (!me) return;
    void a.run(async () => {
      setMine(await req("GET", `/v1/users/${encodeURIComponent(me ?? "")}/badges`));
      const t = await req("GET", "/v1/titles").catch(() => ({ items: [] }));
      setTitles(Array.isArray(t?.items) ? t.items.slice(0, 200) : []);
    });
  }, [me, live.rev.badges]); // eslint-disable-line react-hooks/exhaustive-deps
  const items: J[] = Array.isArray(mine?.items) ? mine.items.slice(0, 200) : [];
  const save = (patch: Record<string, unknown>) => void a.run(async () => { setLive({ me: await req("PATCH", "/v1/me", patch) }); setMine(await req("GET", `/v1/users/${encodeURIComponent(me ?? "")}/badges`)); });
  return (
    <article className="card cm-wide">
      <h2>{c("b.title")}</h2>
      {items.length === 0 ? <p className="muted">{c("b.none")}</p> : (
        <ul className="cm-list">
          {items.map((b: J) => (
            <li key={b.id} className="cm-row">
              <BadgeChip b={b} />
              <span className="small muted grow">{L(b.description).slice(0, 200)}</span>
              {L(b.rarity) && <span className="small">{L(b.rarity).slice(0, 60)}</span>}
              {b.earned_at && <span className="small muted">{c("b.earned", { date: fmtDay(b.earned_at, lang) })}</span>}
            </li>
          ))}
        </ul>
      )}
      <div className="row wrap">
        {items.length > 0 && (
          <label className="cm-field">
            <span className="field-label">{c("b.featured")}</span>
            <select value={mine?.featured_badge ?? ""} disabled={a.busy} onChange={(e) => save({ featured_badge: e.target.value || null })}>
              <option value="">{c("b.no_badge")}</option>
              {items.map((b: J) => <option key={b.id} value={b.id}>{L(b.name).slice(0, 60) || b.id}</option>)}
            </select>
          </label>
        )}
        {titles.length > 0 && (
          <label className="cm-field">
            <span className="field-label">{c("b.title_pick")}</span>
            <select value={live.me?.title ?? ""} disabled={a.busy} onChange={(e) => save({ title: e.target.value || null })}>
              <option value="">{c("b.no_title")}</option>
              {titles.map((t: J) => <option key={t.id} value={t.id} disabled={t.unlocked === false} title={L(t.hint)}>{L(t.text).slice(0, 60) || t.id}</option>)}
            </select>
          </label>
        )}
      </div>
      <ErrorLine text={a.error} />
    </article>
  );
}

/** The player's page designed by the server (`profiles`, 1.1): a read-only view. */
export function Profile() {
  const { c } = useC();
  const live = useLive();
  const me = myId();
  const [view, setView] = useState<J | null>(null);
  const a = useAction();
  useEffect(() => {
    if (me) void a.run(async () => setView(await req("GET", `/v1/users/${encodeURIComponent(me ?? "")}/profile`)));
  }, [me, live.rev.ratings, live.rev.badges]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="stack">
      <h2 className="display">{c("pr.title")}</h2>
      <ErrorLine text={a.error} />
      {view ? <ViewBody view={view} h={{}} /> : !a.error && <span className="skeleton" style={{ width: "100%", height: "6rem" }} />}
    </div>
  );
}

/** Which server provides the online features, adding one, removing this one. */
function ServerSwitch({ onForget }: { onForget: () => void }) {
  const { c, t } = useC();
  const { servers, reloadServers, advanced, go } = useApp();
  const [adding, setAdding] = useState(false);
  const [engine, setEngine] = useState<J | null>(null);
  const list = servers?.servers ?? [];
  const current = list.find((x) => x.id === servers?.features.provider_id) ?? null;
  useEffect(() => {
    if (advanced) void proto.engine().then(setEngine).catch(() => setEngine(null));
  }, [advanced]);
  if (!advanced) {
    return (
      <article className="card cm-wide">
        <h2 className="card-title">{c("acct.servers")}</h2>
        <p className="hint">{c("acct.servers.hint")}</p>
        <div className="row wrap">
          <button className="btn btn-small btn-ghost" onClick={() => go("settings", "servers")}>{t("set.srv.manage")}</button>
          <button className="btn btn-small btn-ghost" onClick={onForget}>{c("acct.forget")}</button>
        </div>
      </article>
    );
  }
  return (
    <article className="card cm-wide">
      <h2 className="card-title">{c("acct.servers")}</h2>
      <div className="row wrap">
        <select value={servers?.features.provider_id ?? ""} aria-label={c("acct.servers")} onChange={(e) => {
          const from = servers?.features.provider_id;
          // Leaving this server: its stream stops (the session stays saved for when the player comes back).
          if (from && from !== e.target.value) void proto.disconnect(from).catch(() => {});
          void serversApi.select(e.target.value).then(reloadServers);
        }}>
          {list.map((x) => <option key={x.id} value={x.id}>{x.name}{x.api ? "" : ` (${c("acct.relayonly")})`}</option>)}
        </select>
        <button className="btn btn-small btn-ghost" onClick={() => setAdding(true)}>{c("acct.add")}</button>
        <button className="btn btn-danger btn-small" onClick={onForget}>{c("acct.forget")}</button>
      </div>
      <p className="hint">{c("acct.servers.hint")}</p>
      {current && (
        <label className="check">
          <input type="checkbox" checked={!!current.background} onChange={(e) => void proto.setBackground(current.id, e.target.checked).then(reloadServers)} />
          <span>{c("acct.background")}</span>
        </label>
      )}
      <p className="hint">{c("acct.background.hint")}</p>
      {engine && (
        <p className="small muted">
          {c("acct.engine")} : {engine.listed ? c("acct.engine.listed", { v: (engine.caps?.contract_versions ?? []).join(", "), rounds: engine.caps?.rounds ? c("acct.engine.rounds") : "" }) : c("acct.engine.none")}
        </p>
      )}
      {adding && createPortal(<AddServerModal onClose={() => setAdding(false)} onAdded={() => { setAdding(false); void reloadServers(); }} />, document.body)}
    </article>
  );
}

/** The player's own data on this server: a copy in a file, and the erasure of the account (both are the protocol's). */
function YourData() {
  const { c } = useC();
  const live = useLive();
  const a = useAction();
  const [saved, setSaved] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  const name = live.me?.display_name ?? live.overview?.session.display_name ?? "";
  const busyPlaying = !!live.match || !!live.ticket;
  const doExport = () => a.run(async () => { setSaved(null); const p = await exportData(); if (p) setSaved(p); });
  return (
    <article className="card">
      <h2>{c("acct.data")}</h2>
      <p className="hint">{c("acct.data.hint")}</p>
      <ErrorLine text={a.error} />
      {saved && <p className="small" role="status">{c("acct.data.saved")} <span className="mono wrapany">{saved}</span></p>}
      <div className="row wrap">
        <button className="btn btn-ghost" disabled={a.busy} onClick={() => void doExport()}>{c("acct.data.export")}</button>
        <button className="btn btn-ghost btn-small" disabled={a.busy || busyPlaying || !name} onClick={() => setAsking(true)}>{c("acct.data.erase")}</button>
      </div>
      {busyPlaying && <p className="small muted">{c("acct.data.erase.busy")}</p>}
      {asking && <EraseDialog name={name} onExport={doExport} exporting={a.busy} onClose={() => setAsking(false)} />}
    </article>
  );
}

/** The confirmation of an erasure: what goes, what stays, a last chance to save a copy, and the name to type. */
function EraseDialog({ name, onExport, exporting, onClose }: { name: string; onExport: () => Promise<unknown>; exporting: boolean; onClose: () => void }) {
  const { c } = useC();
  const live = useLive();
  const a = useAction();
  const [typed, setTyped] = useState("");
  const server = live.overview?.meta?.name ?? "";
  const ok = typed.trim() === name && name !== "";
  return (
    <Modal title={c("acct.erase.title")} labelledBy="erase-title" onClose={() => { if (!a.busy) onClose(); }}>
      <p>{c("acct.erase.gone", { server })}</p>
      <p className="muted">{c("acct.erase.kept")}</p>
      <p className="muted">{c("acct.erase.final")}</p>
      <label className="cm-erase-name">
        <span>{c("acct.erase.type", { name })}</span>
        <input value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" spellCheck={false} data-autofocus />
      </label>
      <ErrorLine text={a.error} />
      <div className="row wrap">
        <button className="btn btn-ghost btn-small" disabled={a.busy || exporting} onClick={() => void onExport()}>{c("acct.erase.export")}</button>
        <button className="btn btn-ghost" disabled={a.busy} onClick={onClose}>{c("acct.erase.cancel")}</button>
        <button className="btn btn-danger" disabled={a.busy || !ok} onClick={() => void a.run(() => eraseAccount(typed))}>{c("acct.erase.confirm")}</button>
      </div>
    </Modal>
  );
}

export function Account({ onForget }: { onForget: () => void }) {
  const { c, L, lang } = useC();
  const { advanced } = useApp();
  const live = useLive();
  const me = live.me;
  const check = live.overview?.check;
  const max = check?.display_name_max ?? 10;
  const min = check?.display_name_min ?? 1;
  const [name, setName] = useState(me?.display_name ?? "");
  const a = useAction();
  useEffect(() => { if (me?.display_name) setName(me.display_name); }, [me?.display_name]);
  const settings: Record<string, boolean> = me?.settings ?? {};
  const restrictions: J[] = (me?.restrictions ?? []).filter((r: J) => !r?.until || Date.parse(r.until) > Date.now());
  const rules = live.overview?.meta?.rules;
  // 1.1.9: a server with several relays; the latency is this computer's, the choice is the server's. The backend says
  // which relays it measures (one per address, 16 at most) and where each stands; the meta gives their names.
  const relays = live.overview?.relays ?? [];
  const listed: J[] = Array.isArray(live.overview?.meta?.relays) ? live.overview.meta.relays : [];
  const relayName = (id: string) => {
    const r = listed.find((x: J) => x?.id === id);
    return (r && (L(r.label) || (typeof r.region === "string" && r.region))) || id;
  };
  const len = [...name.trim()].length;
  return (
    <div className="cm-grid">
      <article className="card">
        <h2>{c("acct.name")}</h2>
        <form className="row wrap" onSubmit={(e) => { e.preventDefault(); void a.run(async () => setLive({ me: await req("PATCH", "/v1/me", { display_name: name.trim() }) }), () => ({ min, max })); }}>
          <input value={name} maxLength={max} onChange={(e) => setName(e.target.value)} aria-label={c("acct.name")} />
          <button className="btn btn-small" type="submit" disabled={a.busy || len < min || len > max || name.trim() === me?.display_name}>{c("acct.rename")}</button>
        </form>
        <p className="hint">{c("acct.name.hint", { min, max })}</p>
        {advanced && live.overview?.session.provider && <p className="small muted">{c("acct.provider", { p: live.overview.session.provider })}</p>}
        {me?.created_at && <p className="small muted">{c("acct.since", { date: fmtDay(me.created_at, lang) })}</p>}
        {has("friends") && me?.friend_code && <p className="small">{c("acct.code")} : <span className="mono">{me.friend_code}</span></p>}
        <ErrorLine text={a.error} />
        <div className="row wrap">
          <button className="btn btn-ghost" onClick={() => void a.run(() => logout(false))}>{c("acct.logout")}</button>
          <button className="btn btn-ghost btn-small" onClick={() => void a.run(() => logout(true))}>{c("acct.logout.all")}</button>
        </div>
      </article>
      {Object.keys(settings).length > 0 && (
        <article className="card">
          <h2>{c("acct.settings")}</h2>
          {Object.entries(settings).filter(([, v]) => typeof v === "boolean").map(([k, v]) => {
            const key = `acct.set.${k}` as CKey;
            return (
              <label key={k} className="check">
                <input type="checkbox" checked={v} disabled={a.busy} onChange={(e) => void a.run(async () => setLive({ me: await req("PATCH", "/v1/me", { settings: { [k]: e.target.checked } }) }))} />
                <span>{c(key) === key ? k : c(key)}</span>
              </label>
            );
          })}
        </article>
      )}
      {restrictions.length > 0 && (
        <article className="card card-bad">
          <h2>{c("acct.restrictions")}</h2>
          <ul>{restrictions.map((r: J, i: number) => <Restriction key={i} r={r} />)}</ul>
        </article>
      )}
      {relays.length > 0 && (
        <article className="card">
          <h2>{c("acct.relays")}</h2>
          <p className="hint">{c("acct.relays.hint")}</p>
          <ul className="cm-relays">
            {relays.map((r) => (
              <li key={r.id} className="row">
                <span>{relayName(r.id)}</span>
                <span className={`small ${r.state === "ok" ? "" : "muted"}`} data-state={r.state}>
                  {r.state === "ok" && r.ms != null ? c("acct.relays.ms", { ms: r.ms })
                    : r.state === "pending" ? c("acct.relays.pending")
                    : r.state === "silent" ? c("acct.relays.none")
                    : r.state === "refused" ? c("acct.relays.refused")
                    : c("acct.relays.unknown")}
                </span>
              </li>
            ))}
          </ul>
        </article>
      )}
      {rules && (L(rules.summary) || (rules.sections ?? []).length > 0) && (
        <article className="card cm-wide">
          <h2>{c("acct.rules")}</h2>
          {L(rules.summary) && <p>{L(rules.summary)}</p>}
          {(rules.sections ?? []).map((s: J) => <div key={s.id}><h3>{L(s.title)}</h3><p className="muted">{L(s.body)}</p></div>)}
          {rules.url && <p className="hint mono wrapany">{rules.url}</p>}
        </article>
      )}
      {has("badges") && <Badges />}
      <YourData />
      <ServerSwitch onForget={onForget} />
    </div>
  );
}
