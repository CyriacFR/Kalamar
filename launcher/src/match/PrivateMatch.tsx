// Private match: create (rules, code to share, Start) or join (paste the code, summary, Join).
import { play } from "../sound/engine";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, toAppError, type MatchModeId, type MatchPlan, type UiPrefs } from "../api";
import { useApp } from "../ctx";
import { useI18n, type Key, type TFn } from "../i18n";
import { IconAlert, IconCheck, IconCopy, IconJoin, IconPlay, IconPlus, IconRelay } from "../icons";
import { Card, NoticeBar, Page, Pill, useErrorNotice } from "../ui";
import type { Notice } from "../ctx";
import { useMatchFlow } from "./MatchFlow";
import { ServerPicker, relayText, selectedServer } from "../servers/ServerPicker";
import { ServerSwitch } from "../servers/ServerMenu";

export const MODES: { id: MatchModeId; verified: boolean }[] = [
  { id: "turf", verified: true },
  { id: "zones", verified: false },
  { id: "tower", verified: false },
  { id: "rainmaker", verified: false },
  { id: "clams", verified: false },
];

type View = "choose" | "create" | "join";

/** `checksum`, `version:2`, `field:host`... from the backend's validation error on the field `code`. */
export function codeErrorText(raw: string, t: TFn): string {
  const [kind, detail = ""] = raw.split(/:(.*)/s);
  const k = `mc.code.err.${kind}` as Key;
  const text = t(k, { detail });
  return text === k ? raw : text;
}

function useCopy() {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  const copy = useCallback(async (text: string, fallback?: HTMLElement | null) => {
    let ok = false;
    try {
      await navigator.clipboard.writeText(text);
      ok = true;
    } catch {
      try {
        if (fallback instanceof HTMLTextAreaElement || fallback instanceof HTMLInputElement) {
          fallback.select();
          ok = document.execCommand("copy");
        }
      } catch { /* the code stays selected for a manual copy */ }
    }
    setCopied(ok);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), 2200);
  }, []);
  return { copied, copy };
}

export default function PrivateMatch() {
  const { t } = useI18n();
  const { servers } = useApp();
  const flow = useMatchFlow();
  const [view, setView] = useState<View>("choose");
  const [joinCode, setJoinCode] = useState("");

  // "Play again": the next round of the same group, as host (the new code to share) or as guest (the code, ready to join).
  useEffect(() => {
    if (!flow.rematch) return;
    if (flow.role === "host") setView("create");
    else {
      setJoinCode(flow.rematch.code);
      setView("join");
    }
  }, [flow.rematch, flow.role]);

  return (
    <Page
      eyebrow={selectedServer(servers) ? <ServerSwitch name={selectedServer(servers)?.name ?? ""} /> : t("nav.community")}
      title={t("mc.title")}
      subtitle={view === "choose" ? t("mc.subtitle2") : undefined}
      back={view === "choose" ? undefined : { label: t("mc.title"), onClick: () => { flow.clearRematch(); setView("choose"); } }}
    >
      {view === "choose" && <Choose onPick={setView} />}
      {view === "create" && <Create />}
      {view === "join" && <Join initial={joinCode} />}
    </Page>
  );
}

function Choose({ onPick }: { onPick: (v: View) => void }) {
  const { t } = useI18n();
  const { servers, advanced } = useApp();
  return (
    <>
      <div className="choose">
        <button className="choose-card c-create" onClick={() => onPick("create")}>
          <span className="choose-ico"><IconPlus /></span>
          <span className="display choose-title">{t("mc.create")}</span>
          <span className="choose-desc">{t("mc.create.desc")}</span>
          <span className="choose-cta">{t("mc.create.cta")} →</span>
        </button>
        <button className="choose-card c-join" onClick={() => onPick("join")}>
          <span className="choose-ico"><IconJoin /></span>
          <span className="display choose-title">{t("mc.join")}</span>
          <span className="choose-desc">{t("mc.join.desc")}</span>
          <span className="choose-cta">{t("mc.join.cta")} →</span>
        </button>
      </div>
      <div className="how">
        <ol>
          <li><strong>{t("mc.how.1")}</strong><span>{t("mc.how.1d")}</span></li>
          <li><strong>{t("mc.how.2")}</strong><span>{t("mc.how.2d")}</span></li>
          <li><strong>{t("mc.how.3")}</strong><span>{t("mc.how.3d")}</span></li>
        </ol>
      </div>
      {advanced && <p className="hint">{t("mc.server.current", { server: (() => { const sv = selectedServer(servers); return sv ? `${sv.name} (${relayText(sv)})` : "—"; })() })}</p>}
    </>
  );
}

// ---- server ------------------------------------------------------------------------------------------------------

/** Kept for the Settings page: the community server list. */
export { ServerPicker as ServerField } from "../servers/ServerPicker";

// ---- create ------------------------------------------------------------------------------------------------------

function Create() {
  const { t } = useI18n();
  const { config, player, go, setConfig, servers, advanced } = useApp();
  const flow = useMatchFlow();
  const errNotice = useErrorNotice();
  const [players, setPlayers] = useState(() => config?.ui.match_players || 2);
  const [mode, setMode] = useState<MatchModeId>(() => MODES.find((m) => m.id === config?.ui.match_mode)?.id ?? "turf");
  const [plan, setPlan] = useState<MatchPlan | null>(flow.rematch);
  const [err, setErr] = useState<Notice | null>(null);
  const [starting, setStarting] = useState(false);
  const code = useRef<HTMLTextAreaElement>(null);
  const { copied, copy } = useCopy();
  const sel = selectedServer(servers);
  const server = sel ? relayText(sel) : "";
  const hasName = !!player.profile?.name?.trim();

  // The rematch plan wins once; later changes of the rules keep its room.
  useEffect(() => {
    if (flow.rematch) {
      setPlan(flow.rematch);
      setPlayers(flow.rematch.spec.players);
      setMode(flow.rematch.spec.mode);
    }
  }, [flow.rematch]);

  useEffect(() => {
    if (!server || (player.profile && !hasName)) return;
    // The same room and round while only the rules change; a new server gives a new room.
    if (plan && plan.spec.players === players && plan.spec.mode === mode && plan.relay === server) return;
    const keep = plan && plan.relay === server ? plan : null;
    let alive = true;
    void (async () => {
      try {
        let next = await api.matchCreate({ players, mode, room: keep?.spec.room ?? null });
        for (let i = 0; keep && i < keep.spec.round; i++) next = await api.matchNextRound(next.code);
        if (alive) { setPlan(next); setErr(null); }
      } catch (e) {
        if (alive) setErr(errNotice(e));
      }
    })();
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [players, mode, server, hasName, player.profile]);

  const remember = (patch: Partial<UiPrefs>) => { if (config) void api.setUiPrefs({ ...config.ui, ...patch }).then(setConfig).catch(() => {}); };
  const choosePlayers = (n: number) => { setPlayers(n); remember({ match_players: n }); };
  const chooseMode = (m: MatchModeId) => { setMode(m); remember({ match_mode: m }); };
  const start = async () => {
    if (!plan) return;
    setStarting(true);
    try {
      setErr(null);
      await flow.start(plan, "host");
    } catch (e) {
      setErr(errNotice(e));
    } finally {
      setStarting(false);
    }
  };

  if (!server) {
    return (
      <Card className="narrow">
        <h2>{t("mc.server.need.title")}</h2>
        <p>{t("mc.server.need")}</p>
        <ServerPicker />
      </Card>
    );
  }
  if (player.profile && !hasName) {
    return (
      <Card tone="warn" className="narrow">
        <p>{t("mc.needname")}</p>
        <button className="btn" onClick={() => go("profile")}>{t("nav.locker")}</button>
      </Card>
    );
  }
  const spec = plan?.spec;
  const untested = !MODES.find((m) => m.id === mode)?.verified;
  return (
    <div className="prematch">
      <div className="stack">
        <Card title={t("mc.rules")}>
          <div className="field">
            <span className="field-label" id="pl-label">{t("mc.rules.players")}</span>
            <div className="chips" role="radiogroup" aria-labelledby="pl-label">
              {[2, 3, 4, 5, 6, 7, 8].map((n) => (
                <button key={n} role="radio" aria-checked={players === n} className={`chip chip-num ${players === n ? "chip-on" : ""}`} onClick={() => choosePlayers(n)}>{n}</button>
              ))}
            </div>
            <p className="hint">{t("mc.rules.teams", { teams: plan?.rules.split("teams=")[1]?.split(",")[0] ?? "" })}</p>
          </div>
          <div className="field" style={{ marginTop: "var(--sp-4)" }}>
            <span className="field-label" id="md-label">{t("mc.rules.mode")}</span>
            <div className="chips" role="radiogroup" aria-labelledby="md-label">
              {MODES.map((m) => (
                <button key={m.id} role="radio" aria-checked={mode === m.id} className={`chip ${mode === m.id ? "chip-on" : ""}`} onClick={() => chooseMode(m.id)}>
                  {t(`mode.${m.id}` as Key)}
                  {advanced && !m.verified && <span className="badge badge-hot">{t("mc.untested")}</span>}
                </button>
              ))}
            </div>
            {untested && <p className="hint warn-text">{t("mc.untested.hint")}</p>}
          </div>
        </Card>
        <Card title={t("mc.server.title")}>
          <div className="row between">
            <span>{sel?.name}{advanced && <span className="mono muted"> · {server}</span>}</span>
            <IconRelay width="1.2rem" height="1.2rem" />
          </div>
          {advanced ? (
            <details className="advanced">
              <summary>{t("mc.server.change")}</summary>
              <ServerPicker />
            </details>
          ) : (
            <button className="link small" onClick={() => go("settings", "servers")}>{t("mc.server.change")}</button>
          )}
        </Card>
      </div>

      <div className="stack">
        <Card className="code-card">
          <div className="eyebrow">{t("mc.code.label")}{spec && spec.round > 0 ? ` · ${t("mc.round", { n: spec.round + 1 })}` : ""}</div>
          {plan ? (
            <>
              <textarea ref={code} className="code-box mono" readOnly rows={3} value={plan.code} aria-label={t("mc.code.label")} onFocus={(e) => e.currentTarget.select()} />
              <div className="row">
                <button className="btn" onClick={() => void copy(plan.code, code.current)}>{copied ? <IconCheck /> : <IconCopy />} {copied ? t("mc.copied") : t("mc.copy")}</button>
                <span className="hint grow">{t("mc.code.share")}</span>
              </div>
              <dl className="facts">
                <dt>{t("mc.fact.host")}</dt><dd>{spec?.host}</dd>
                <dt>{t("mc.fact.rules")}</dt><dd>{spec?.players} {t("mc.players")} · {t(`mode.${spec?.mode ?? "turf"}` as Key)} {untested && <Pill ok={null}>{t("mc.untested")}</Pill>}</dd>
                {advanced && <><dt>{t("mc.fact.server")}</dt><dd className="mono">{plan.relay}{plan.relay_resolved ? ` → ${plan.relay_resolved}` : ""}</dd></>}
                {advanced && <><dt>{t("mc.fact.id")}</dt><dd className="mono">{plan.match_id}</dd></>}
              </dl>
            </>
          ) : (
            <div className="code-box code-skel"><span className="skeleton" style={{ height: "1rem", width: "90%" }} /><span className="skeleton" style={{ height: "1rem", width: "70%" }} /></div>
          )}
        </Card>
        <button className="btn btn-big start-btn" disabled={!plan || starting} onClick={() => { play("confirm"); void start(); }}>
          <IconPlay /> {starting ? t("mc.starting") : t("mc.start")}
        </button>
        <p className="hint">{t("mc.start.hint", { n: players })}</p>
        <NoticeBar notice={err} />
      </div>
    </div>
  );
}

// ---- join --------------------------------------------------------------------------------------------------------

function Join({ initial }: { initial: string }) {
  const { t } = useI18n();
  const { player, go, advanced } = useApp();
  const flow = useMatchFlow();
  const errNotice = useErrorNotice();
  const [text, setText] = useState(initial);
  const [plan, setPlan] = useState<MatchPlan | null>(null);
  const [bad, setBad] = useState<string | null>(null);
  const [err, setErr] = useState<Notice | null>(null);
  const [starting, setStarting] = useState(false);
  const [trustRelay, setTrustRelay] = useState(false);
  const hasName = !!player.profile?.name?.trim();

  useEffect(() => { if (initial) setText(initial); }, [initial]);
  // Another code: its relay is confirmed anew.
  useEffect(() => { setTrustRelay(false); }, [plan?.relay]);
  useEffect(() => {
    if (!text.trim()) { setPlan(null); setBad(null); return; }
    let alive = true;
    const id = window.setTimeout(() => {
      api.matchParse(text).then((p) => { if (alive) { setPlan(p); setBad(null); if (p.code !== text) setText(p.code); } }).catch((e) => {
        if (!alive) return;
        const a = toAppError(e);
        setPlan(null);
        setBad(codeErrorText(a.fields.find((f) => f.key === "code")?.message ?? a.message, t));
      });
    }, 200);
    return () => { alive = false; window.clearTimeout(id); };
  }, [text, t]);

  const join = async () => {
    if (!plan) return;
    setStarting(true);
    try {
      setErr(null);
      await flow.start(plan, "join", trustRelay);
    } catch (e) {
      setErr(errNotice(e));
    } finally {
      setStarting(false);
    }
  };
  const paste = async () => {
    try { setText(await navigator.clipboard.readText()); } catch { /* the field takes a manual paste */ }
  };
  const spec = plan?.spec;
  const untested = spec ? spec.mode !== "turf" : false;
  // A relay this launcher does not know (not a saved server's): the player confirms it before the game talks to it.
  const unknownRelay = !!plan && plan.relay_known === false;
  return (
    <div className="prematch">
      <div className="stack">
        <Card title={t("mc.join.paste")}>
          <label className="sr-only" htmlFor="join-code">{t("mc.code.label")}</label>
          <textarea id="join-code" className={`code-box mono ${bad ? "invalid" : ""}`} rows={4} value={text} placeholder="KLM1-XXXXX-XXXXX-XXXXX-…" spellCheck={false}
            aria-invalid={!!bad} aria-describedby="join-msg" onChange={(e) => setText(e.target.value)} />
          <div className="row">
            <button className="btn btn-ghost" onClick={() => void paste()}>{t("mc.paste")}</button>
            {text && <button className="btn btn-ghost" onClick={() => setText("")}>{t("mc.clear")}</button>}
          </div>
          <div id="join-msg" aria-live="polite">
            {bad && <p className="field-error"><IconAlert width="1em" height="1em" style={{ verticalAlign: "-0.12em" }} /> {bad}</p>}
          </div>
        </Card>
      </div>
      <div className="stack">
        <Card title={t("mc.summary")} className={plan ? "summary-card ok" : "summary-card"}>
          {!plan && <p className="muted">{t("mc.summary.empty")}</p>}
          {plan && spec && (
            <dl className="facts">
              <dt>{t("mc.fact.host")}</dt><dd>{spec.host}</dd>
              <dt>{t("mc.fact.rules")}</dt><dd>{spec.players} {t("mc.players")} · {t(`mode.${spec.mode}` as Key)} {untested && <Pill ok={null}>{t("mc.untested")}</Pill>}</dd>
              {advanced && <><dt>{t("mc.fact.server")}</dt><dd className="mono">{plan.relay}{plan.relay_resolved ? ` → ${plan.relay_resolved}` : ""}</dd></>}
              {advanced && <><dt>{t("mc.fact.id")}</dt><dd className="mono">{plan.match_id}</dd></>}
              {spec.round > 0 && (<><dt>{t("mc.fact.round")}</dt><dd>{spec.round + 1}</dd></>)}
            </dl>
          )}
          {plan?.own_code && <p className="hint warn-text">{t("mc.join.own")}</p>}
        </Card>
        {unknownRelay && plan && (
          <Card tone="warn">
            <p>{t("mc.join.relay.unknown", { relay: plan.relay_resolved ? `${plan.relay} (${plan.relay_resolved})` : plan.relay })}</p>
            <label className="check">
              <input type="checkbox" checked={trustRelay} onChange={(e) => setTrustRelay(e.target.checked)} /> {t("mc.join.relay.trust")}
            </label>
          </Card>
        )}
        {player.profile && !hasName && (
          <Card tone="warn"><p>{t("mc.needname")}</p><button className="btn" onClick={() => go("profile")}>{t("nav.locker")}</button></Card>
        )}
        <button className="btn btn-big start-btn" disabled={!plan || starting || !hasName || (unknownRelay && !trustRelay)} onClick={() => { play("confirm"); void join(); }}>
          <IconJoin /> {starting ? t("mc.starting") : t("mc.join.go")}
        </button>
        <NoticeBar notice={err} />
      </div>
    </div>
  );
}
