// Activities (`activities` area, protocol 1.1): whatever the server invents that leads to matches — a cup, a league, a
// challenge, a themed evening — listed as it describes them and opened as its views. The launcher never branches on
// an activity's kind: it shows the server's view, sends the player's actions, and refreshes on `activity.updated`.
import { useCallback, useEffect, useState } from "react";
import { useApp } from "../ctx";
import { toFail, type J } from "./api";
import { req, setLive, useLive } from "./store";
import { failText, useC } from "./texts";
import { ErrorLine, fmtDay, useAction, useLeft } from "./ui";
import { ProtoIcon, safeColor } from "./icons";
import { LIMITS, SeriesCard, ViewBody, useText } from "./view";
import { IconBack } from "../icons";

const STATES = ["upcoming", "open", "running", "finished", "cancelled"];

function Starts({ at }: { at: string }) {
  const { c, lang } = useC();
  const left = useLeft(at);
  if (left === null) return null;
  return <span className="small muted">{left > 0 ? c("a.starts_in", { t: left > 86400 ? fmtDay(at, lang) : `${Math.floor(left / 3600)} h ${Math.floor((left % 3600) / 60)} min` }) : c("a.started")}</span>;
}

/** One activity's view: actions go to the server, its answer replaces the view; a stale view is fetched again. */
function ActivityView({ id, onBack }: { id: string; onBack: () => void }) {
  const { c } = useC();
  const T = useText();
  const { toast } = useApp();
  const live = useLive();
  const [view, setView] = useState<J | null>(null);
  const [series, setSeries] = useState<string | null>(null);
  const load = useAction();
  const act = useAction();
  const fetchView = useCallback(() => void load.run(async () => setView(await req("GET", `/v1/activities/${encodeURIComponent(id)}`))), [id]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(fetchView, [fetchView]);
  // `activity.updated` with a newer revision than the one shown: fetch it.
  const pushed = live.activityRev[id];
  useEffect(() => {
    if (pushed !== undefined && view && typeof view.revision === "number" && pushed > view.revision) fetchView();
  }, [pushed]); // eslint-disable-line react-hooks/exhaustive-deps
  const onAction = (a: J, values?: Record<string, unknown>) => {
    if (!a || typeof a.id !== "string") return;
    void act.run(async () => {
      try {
        const r = await req("POST", `/v1/activities/${encodeURIComponent(id)}/actions/${encodeURIComponent(a.id)}`, { values: values ?? {}, revision: view?.revision ?? null });
        if (r?.view && typeof r.view === "object") setView(r.view);
        else fetchView();
        if (r?.notice?.text) toast({ kind: r.notice.level === "error" ? "error" : r.notice.level === "success" ? "ok" : "info", text: T(r.notice.text) });
        if (typeof r?.series_id === "string") setSeries(r.series_id);
        if (typeof r?.match_id === "string") void req("GET", `/v1/matches/${encodeURIComponent(r.match_id)}`).then((m) => setLive({ match: m })).catch(() => {});
      } catch (e) {
        const f = toFail(e);
        if (f.code === "stale_view") {
          fetchView();
          throw { ...f, message: c("a.stale") };
        }
        throw f;
      }
    });
  };
  return (
    <div className="stack">
      <button className="back" onClick={onBack}><IconBack /> {c("a.back")}</button>
      {view ? (
        <>
          <header className="row cm-act-head">
            <h2 className="display grow">{T(view.title, LIMITS.label)}</h2>
            {typeof view.state === "string" && <span className="pill pill-neutral">{c(`a.state.${STATES.includes(view.state) ? view.state : "running"}` as "a.state.running")}</span>}
          </header>
          <ViewBody view={view} h={{ onAction, onRefresh: fetchView, busy: act.busy }} />
          {series && <SeriesCard id={series} />}
          <ErrorLine text={act.error} />
        </>
      ) : load.error ? (
        <ErrorLine text={load.error} />
      ) : (
        <span className="skeleton" style={{ width: "100%", height: "8rem" }} />
      )}
    </div>
  );
}

export function Activities() {
  const { c } = useC();
  const T = useText();
  const { toast } = useApp();
  const live = useLive();
  const [items, setItems] = useState<J[] | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const load = useAction();
  const join = useAction();
  useEffect(() => {
    void load.run(async () => {
      const r = await req("GET", "/v1/activities");
      const list: J[] = (Array.isArray(r?.items) ? r.items : []).filter((x: J) => x && typeof x.id === "string").slice(0, 200);
      list.sort((a: J, b: J) => (Number(a.order) || 0) - (Number(b.order) || 0));
      setItems(list);
      setLive((l) => ({ activityTitles: { ...l.activityTitles, ...Object.fromEntries(list.map((x: J) => [x.id, x.title])) } }));
    });
  }, [live.rev.activities]); // eslint-disable-line react-hooks/exhaustive-deps
  if (open) return <ActivityView id={open} onBack={() => setOpen(null)} />;
  const doJoin = (x: J) => void join.run(async () => {
    try {
      const r = await req("POST", `/v1/activities/${encodeURIComponent(x.id)}/actions/${encodeURIComponent(x.join.action)}`, { values: {}, revision: x.revision ?? null });
      if (r?.notice?.text) toast({ kind: "ok", text: T(r.notice.text) });
      setOpen(x.id);
    } catch (e) {
      const f = toFail(e);
      toast({ kind: "error", text: failText(f, c) });
      throw f;
    }
  });
  return (
    <div className="stack">
      <h2 className="display">{c("a.title")}</h2>
      <ErrorLine text={load.error || join.error} />
      {items === null ? <span className="skeleton" style={{ width: "100%", height: "6rem" }} /> : items.length === 0 ? <p className="muted">{c("a.none")}</p> : (
        <div className="cm-grid">
          {items.map((x: J) => {
            const color = safeColor(x.color);
            const state = STATES.includes(x.state) ? x.state : "running";
            return (
              <article key={x.id} className="card cm-activity" style={color ? { borderTopColor: color } : undefined}>
                <header className="row">
                  <span className="cm-act-ico" style={color ? { color } : undefined}><ProtoIcon name={x.icon} /></span>
                  <div className="grow">
                    <h3>{T(x.title, LIMITS.label) || x.id}</h3>
                    <div className="cm-chips">
                      {x.kind && <span className="chip">{T(x.kind, LIMITS.label)}</span>}
                      <span className={`pill ${state === "open" ? "pill-ok" : state === "cancelled" ? "pill-bad" : "pill-neutral"}`}>{c(`a.state.${state}` as "a.state.running")}</span>
                      {x.you?.joined && <span className="pill pill-ok">{c("a.joined")}</span>}
                    </div>
                  </div>
                </header>
                {x.subtitle && <p>{T(x.subtitle)}</p>}
                <p className="small muted">
                  {x.participants && typeof x.participants.count === "number" && (typeof x.participants.capacity === "number" ? c("a.players_of", { n: x.participants.count, max: x.participants.capacity }) : c("a.players", { n: x.participants.count }))}
                </p>
                {typeof x.starts_at === "string" && state === "upcoming" && <Starts at={x.starts_at} />}
                <div className="row wrap">
                  {x.join && typeof x.join.action === "string" && !x.you?.joined && ["open", "upcoming", "running"].includes(state) && (
                    <button className="btn" disabled={join.busy} onClick={() => doJoin(x)}>{T(x.join.label, LIMITS.label) || c("a.join")}</button>
                  )}
                  <button className="btn btn-ghost" onClick={() => setOpen(x.id)}>{c("a.open")}</button>
                </div>
              </article>
            );
          })}
        </div>
      )}
    </div>
  );
}
