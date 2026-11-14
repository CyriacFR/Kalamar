// Settings › Discord: the presence switch (off by default), what it does and through what, whether Discord is there,
// the level of detail, and a preview of exactly what the player's contacts see (the very function that builds what
// goes out, `discord/presence.ts`). The section exists only in a build that names a Discord application.
import { useEffect, useMemo, useState } from "react";
import { discordApi, onDiscordStatus, type DiscordView } from "../../api";
import { useApp } from "../../ctx";
import { useI18n, type Key } from "../../i18n";
import { BRAND } from "../../brand";
import { Mark } from "../../icons";
import { pickName } from "../../gamedata";
import { useStagesView } from "../../stages";
import { heroServer } from "../play/Online";
import { buildActivity, sampleSituations, type ActivityInput, type Prefs, type Situation } from "../../discord/presence";
import { prefsOf, useCurrentSituation } from "../../discord/useDiscordPresence";
import { ART_BG, ART_FG, ART_STROKE, GLYPHS } from "../../discord/art";

/** The section's data: once, then the thread's changes as they happen, and the pipe listing again now and then. */
export function useDiscordView(): [DiscordView | null, (v: DiscordView) => void] {
  const [view, setView] = useState<DiscordView | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () => void discordApi.view().then((v) => { if (alive) setView(v); }).catch(() => {});
    load();
    const id = window.setInterval(load, 15_000);
    let off: (() => void) | undefined;
    void onDiscordStatus((s) => setView((v) => (v ? { ...v, status: s } : v))).then((u) => (off = u));
    return () => { alive = false; window.clearInterval(id); off?.(); };
  }, []);
  return [view, setView];
}

function SmallArt({ k }: { k: string }) {
  const g = GLYPHS[k];
  if (!g) return null;
  return (
    <svg viewBox="-8 -8 40 40" aria-hidden="true" focusable="false">
      <circle cx="12" cy="12" r="20" fill={ART_BG} />
      <g fill="none" stroke={ART_FG} strokeWidth={ART_STROKE} strokeLinecap="round" strokeLinejoin="round">
        {g.paths.map((d) => <path key={d} d={d} />)}
        {(g.circles ?? []).map(([cx, cy, r]) => <circle key={`${cx}-${cy}`} cx={cx} cy={cy} r={r} />)}
      </g>
      {(g.fills ?? []).map((d) => <path key={d} d={d} fill={ART_FG} />)}
    </svg>
  );
}

function clock(s: number): string {
  const v = Math.max(0, Math.floor(s));
  const h = Math.floor(v / 3600), m = Math.floor((v % 3600) / 60), x = v % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${two(m)}:${two(x)}` : `${two(m)}:${two(x)}`;
}

/** A presence as Discord lays it out in a profile: the application, the two lines, the group size, the time. */
export function PresenceCard({ a, off }: { a: ActivityInput; off: boolean }) {
  const { t } = useI18n();
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => window.clearInterval(id);
  }, []);
  return (
    <div className={`dc-card ${off ? "is-off" : ""}`} data-dc-card>
      <div className="dc-card-head">{t("dc.prev.playing", { name: BRAND.name })}</div>
      <div className="dc-card-body">
        <div className="dc-art" title={a.large_text}>
          <span className="dc-logo"><Mark size="100%" /></span>
          {a.small_image && <span className="dc-small" title={a.small_text}><SmallArt k={a.small_image} /></span>}
        </div>
        <div className="dc-lines">
          <div className="dc-app">{BRAND.name}</div>
          {a.details && <div className="dc-line" data-dc="details">{a.details}</div>}
          {(a.state || a.party) && (
            <div className="dc-line" data-dc="state">
              {a.state}{a.party ? `${a.state ? " " : ""}${t("dc.prev.party", { n: a.party[0], max: a.party[1] })}` : ""}
            </div>
          )}
          {a.start ? <div className="dc-line dc-time">{t("dc.prev.elapsed", { t: clock(now - a.start) })}</div> : null}
        </div>
      </div>
      {a.large_text && <p className="dc-hover">{t("dc.prev.hover", { text: a.large_text })}</p>}
    </div>
  );
}

type Sample = "now" | "room" | "queue" | "match" | "watch" | "game";
const SAMPLES: Sample[] = ["now", "room", "queue", "match", "watch", "game"];

function Preview({ prefs }: { prefs: Prefs }) {
  const { t, lang } = useI18n();
  const { servers } = useApp();
  const current = useCurrentSituation();
  const [which, setWhich] = useState<Sample>("now");
  // The examples name a stage of the player's own game (read only when the player asked for stage names).
  const stages = useStagesView(prefs.mode_stage);
  const tr = (key: string, vars?: Record<string, string | number>) => t(key as Key, vars);
  const samples = useMemo(() => {
    const stage = stages?.stages.map((s) => pickName(s.name, lang)).find((n) => !!n) || t("dc.prev.stage");
    const server = heroServer(servers)?.name || t("dc.prev.server");
    return sampleSituations(Math.floor(Date.now() / 1000), { mode: t("mode.turf"), stage, server });
  }, [stages, servers, lang, t]);
  const s: Situation | null = which === "now" ? current : samples[which];
  const a = s ? buildActivity(s, { ...prefs, enabled: true }, tr, BRAND.name) : null;
  return (
    <div className="set-block dc-preview">
      <div className="set-block-head"><h3>{t("dc.prev.title")}</h3></div>
      <div className="seg dc-samples" role="radiogroup" aria-label={t("dc.prev.title")}>
        {SAMPLES.map((k) => (
          <button key={k} type="button" role="radio" aria-checked={which === k} className={`seg-btn ${which === k ? "on" : ""}`} data-sample={k} onClick={() => setWhich(k)}>
            {t(`dc.prev.${k}` as Key)}
          </button>
        ))}
      </div>
      {a && <PresenceCard a={a} off={!prefs.enabled} />}
      {!prefs.enabled && <p className="hint">{t("dc.prev.off")}</p>}
    </div>
  );
}

export function DiscordSection() {
  const { t } = useI18n();
  const { config, setConfig, fail, advanced } = useApp();
  const [view, setView] = useDiscordView();
  const prefs = view?.prefs ?? prefsOf(config);
  const save = (patch: Partial<Prefs>) => {
    const next = { ...prefs, ...patch };
    void discordApi.setPrefs(next).then((v) => {
      setView(v);
      if (config) setConfig({ ...config, discord: v.prefs });
    }).catch(fail);
  };
  const st = view?.status;
  let line: string;
  let tone = "neutral";
  if (!prefs.enabled) {
    line = t(view?.detected ? "dc.st.found" : "dc.st.absent");
  } else if (st?.link === "connected") {
    line = t("dc.st.connected");
    tone = "ok";
  } else if (st?.link === "busy") {
    line = t("dc.st.busy");
  } else if (st?.link === "refused") {
    line = t("dc.st.refused", { code: st.code ?? "?" });
    tone = "bad";
  } else {
    line = t("dc.st.waiting");
  }
  const activity = prefs.detail === "activity";
  return (
    <div className="stack dc-section">
      <label className="switch-row">
        <span className="switch"><input type="checkbox" checked={prefs.enabled} onChange={(e) => save({ enabled: e.target.checked })} /><span className="switch-ui" aria-hidden="true" /></span>
        <span className="switch-text"><span className="switch-label">{t("dc.switch")}</span><span className="hint">{t("dc.switch.hint")}</span></span>
      </label>
      <p className={`pill pill-${tone} dc-status`} role="status">{line}</p>
      {prefs.enabled && st?.link === "connected" && st.code !== null && <p className="hint warn-text">{t("dc.st.rejected", { code: st.code })}</p>}
      {advanced && prefs.enabled && st && <p className="tiny muted mono">{t("dc.st.sent", { n: st.sent })}</p>}
      <p className="hint">{t("dc.how")}</p>

      <div className="set-block">
        <div className="set-block-head"><h3 id="dc-level">{t("dc.level")}</h3></div>
        <div className="dc-levels" role="radiogroup" aria-labelledby="dc-level">
          {(["minimal", "activity"] as const).map((d) => (
            <label key={d} className="check dc-level">
              <input type="radio" name="dc-level" checked={prefs.detail === d} onChange={() => save({ detail: d })} />
              <span><span className="dc-level-name">{t(`dc.level.${d}`)}</span><span className="hint">{t(`dc.level.${d}.hint`)}</span></span>
            </label>
          ))}
        </div>
        <label className={`check dc-opt ${activity ? "" : "is-disabled"}`}>
          <input type="checkbox" checked={prefs.mode_stage} disabled={!activity} onChange={(e) => save({ mode_stage: e.target.checked })} />
          <span><span>{t("dc.modestage")}</span><span className="hint">{t("dc.modestage.hint")}</span></span>
        </label>
        <label className={`check dc-opt ${activity ? "" : "is-disabled"}`}>
          <input type="checkbox" checked={prefs.server} disabled={!activity} onChange={(e) => save({ server: e.target.checked })} />
          <span><span>{t("dc.server")}</span><span className="hint">{t("dc.server.hint")}</span></span>
        </label>
      </div>

      <Preview prefs={prefs} />

      <p className="hint">{t("dc.never")}</p>
      <p className="hint">{t("dc.game")}</p>
    </div>
  );
}
