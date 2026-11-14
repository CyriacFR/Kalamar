// The advanced part of the Play screen: last launch (process, exit code, log), what the game did with the profile,
// launch options (full game without the profile, LAN, extra arguments) and the second local player. Only loaded and
// shown in the advanced mode.
import { useEffect, useState } from "react";
import { api, type GameStatus, type LaunchReport, type ModeId, type Preflight } from "../../api";
import { useApp, type Notice } from "../../ctx";
import { useI18n, type Key } from "../../i18n";
import { refusalText, fieldLabel } from "../../refusal";
import { NoticeBar, Pill, Section, useErrorNotice } from "../../ui";
import { keyLabel } from "../settings/fields";

const MODES: ModeId[] = ["hub", "range", "lan", "original"];

function modeTitle(t: (k: Key) => string, id: string): string {
  return MODES.includes(id as ModeId) ? t(`play.mode.${id}.title` as Key) : id === "match" ? t("tile.match.title") : id;
}

/** Status of a game process (the first player's by default): state, PID, exit code, mode. */
export function GameState({ status }: { status?: GameStatus }) {
  const { t } = useI18n();
  const ctx = useApp();
  const game = status ?? ctx.game;
  if (game.state === "idle") return <p className="muted">{t("play.state.idle")}</p>;
  if (game.state === "running") {
    return <p><Pill ok={true}>{t("play.state.running", { pid: game.pid ?? "?" })}</Pill> <span className="muted small">{t("play.mode")} : {game.mode ? modeTitle(t, game.mode) : "?"}</span></p>;
  }
  if (game.restarted) return <p><Pill ok={true}>{t("play.restarted")}</Pill></p>;
  const code = game.exit_code ?? t("play.state.exited.nocode");
  const text = game.stopped_by_user ? t("play.state.exited.stopped") : game.abnormal ? t("play.state.exited.bad", { code }) : t("play.state.exited.ok", { code });
  return <p><Pill ok={!game.abnormal}>{text}</Pill> <span className="muted small">{t("play.mode")} : {game.mode ? modeTitle(t, game.mode) : "?"}</span></p>;
}

/** What the runtime did with the profile at the last launch, from the "save profile:" lines of its log. */
export function ProfileReport({ second = false }: { second?: boolean }) {
  const { t } = useI18n();
  const { game, game2, go, status } = useApp();
  const g = second ? game2 : game;
  const [rep, setRep] = useState<LaunchReport | null>(null);
  const settled = g.state === "exited" ? `${g.ended_at}` : g.state;
  const started = status !== null;
  useEffect(() => {
    if (!started) return;
    let alive = true;
    const load = () => api.launchReport(second).then((r) => alive && setRep(r)).catch(() => {});
    void load();
    // The line is printed a few seconds after start-up; a first launch restarts the game once.
    const id = g.state === "running" || g.restarted ? window.setInterval(load, 3000) : undefined;
    return () => { alive = false; if (id) window.clearInterval(id); };
  }, [started, settled, g.state, g.restarted, second]);
  const r = rep?.record;
  if (!r) return null;
  const f = rep.feedback;
  const fields = (f.state === "needs_appearance" && f.missing.length ? f.missing : r.missing).map((k) => t(`profile.cat.${k}` as "profile.cat.style")).join(", ");
  let text: string;
  let ok: boolean | null = true;
  if (r.profile !== "passed") {
    ok = r.profile === "disabled" || r.profile === "skipped" ? null : false;
    text = t(`play.report.${r.profile}` as "play.report.disabled", { fields, detail: r.detail ?? "" });
  } else {
    ok = f.state === "failed" || f.state === "needs_appearance" ? false : f.state === "none" ? null : true;
    const refusal = f.refusal ? refusalText(f.refusal, t) : f.reason ?? "";
    text = t(`play.report.${f.state}` as "play.report.applied", { n: f.changes ?? 0, backup: f.backup ?? "", reason: refusal, fields });
  }
  const field = f.refusal?.field ?? "";
  const b = rep.backups;
  return (
    <article className="adv-card adv-wide">
      <h3>{t("play.report.title")}</h3>
      <p className="small muted">{t("play.report.what", { mode: modeTitle(t, r.mode), scene: r.scene ?? "-", save: r.save_existed ? t("play.report.save.yes") : t("play.report.save.no") })}</p>
      <p className={`report ${ok === false ? "bad-text" : ""}`}><Pill ok={ok}>{""}</Pill> {text}</p>
      {f.state === "failed" && f.refusal && (
        <div className="refusal">
          {field && <p className="small">{t("play.report.field", { field: fieldLabel(field, t) })} <button className="link" onClick={() => go("profile")}>{t("nav.locker")}</button></p>}
          <p className="tiny muted">{t("play.report.verbatim")}</p>
          <pre className="log">{f.refusal.raw}</pre>
        </div>
      )}
      {r.profile === "passed" && <p className="small muted">{t("play.report.options", { skip: r.skip_first_launch ? t("common.yes") : t("common.no"), unlock: r.unlock_all ? t("common.yes") : t("common.no") })}</p>}
      {b && b.exists && (
        <p className="small" title={b.dir}>
          {t("play.report.backups", { n: b.count })} <span className="mono wrapany muted">{b.dir}</span>
          {f.deleted_backups ? <> · {t("play.report.pruned", { n: f.deleted_backups, keep: f.keep_backups ?? "?" })}</> : null}
        </p>
      )}
      {f.lines.length > 0 && <pre className="log">{f.lines.map((l) => l.slice(0, 220)).join(String.fromCharCode(10))}</pre>}
      <details className="advanced">
        <summary>{t("play.report.env")}</summary>
        <table className="envtable"><tbody>{r.env.map(([k, v]) => <tr key={k}><td className="mono">{k}</td><td className="mono wrapany">{v}</td></tr>)}</tbody></table>
        <p className="tiny muted mono wrapany">{r.userdata} · {t("play.logname")} : {r.log_path}</p>
      </details>
    </article>
  );
}

/** The key that is the Switch L button on this PC: the first `key:` entry of `bind_l`, else the default layout's (Z). */
export function lButtonKey(bindL: string | undefined): string | null {
  if (!bindL) return "Z";
  const keys = bindL.split(",").map((s) => s.trim().toLowerCase()).filter((s) => s.startsWith("key:")).map((s) => s.slice(4));
  if (keys.length === 0) return null; // bound to the controller only
  if (keys.includes("default")) return "Z";
  return keys[0];
}

/** Short in-app help after a LAN launch: what to press in the lobby, the current mode and room. */
export function LanHelp({ second = false }: { second?: boolean }) {
  const { t } = useI18n();
  const { game, game2 } = useApp();
  const g = second ? game2 : game;
  const [rep, setRep] = useState<LaunchReport | null>(null);
  const [key, setKey] = useState<string | null>("Z");
  const live = g.state === "running" || g.restarted;
  useEffect(() => {
    void api.launchReport(second).then(setRep).catch(() => {});
    void api.settingsLoad().then((s) => setKey(lButtonKey(s.values["bind_l"]))).catch(() => {});
  }, [g.state, g.started_at, second]);
  const r = rep?.record;
  if (!r || r.mode !== "lan") return null;
  const relay = (r.lan ?? "").startsWith("relay:");
  return (
    <article className="adv-card adv-wide">
      <h3>{t("lanhelp.title")} <span className="badge badge-hot">{t("nav.experimental")}</span></h3>
      <p><Pill ok={live ? true : null}>{live ? t("lanhelp.live") : t("lanhelp.ended")}</Pill></p>
      <p className="small">
        {t("lanhelp.mode", { mode: relay ? t("lan.mode.relay") : t("lan.mode.lan"), value: r.lan ?? "-" })}
        {relay && <> · {t("lanhelp.room", { room: r.room ?? "DEFAULT" })}</>}
        {r.nickname && <> · {t("lanhelp.nick", { name: r.nickname })}</>}
      </p>
      <ol className="steps">
        <li>{t("lanhelp.s1")}</li>
        <li>{key ? t("lanhelp.s2key", { key: keyLabel(key, t) }) : t("lanhelp.s2pad")}</li>
        <li>{t("lanhelp.s3")}</li>
        <li>{relay ? t("lanhelp.s4relay", { room: r.room ?? "DEFAULT" }) : t("lanhelp.s4lan")}</li>
      </ol>
      <p className="hint">{t("lanhelp.fallback")}</p>
    </article>
  );
}

function SecondPlayer({ pre, reload }: { pre: Preflight | null; reload: () => void }) {
  const { t } = useI18n();
  const { game, game2, config, setConfig } = useApp();
  const errNotice = useErrorNotice();
  const [name, setName] = useState("");
  const [dir, setDir] = useState("");
  const [mode, setMode] = useState<ModeId>("lan");
  const [notice, setNotice] = useState<Notice | null>(null);
  useEffect(() => {
    if (config) { setName(config.second_name); setDir(config.second_dir); }
  }, [config?.second_name, config?.second_dir]); // eslint-disable-line react-hooks/exhaustive-deps
  const running2 = game2.state === "running" || game2.restarted;
  const run = async (f: () => Promise<unknown>) => {
    try {
      setNotice(null);
      await f();
      reload();
    } catch (e) {
      setNotice(errNotice(e));
    }
  };
  const save = () => run(async () => setConfig(await api.setSecondPlayer(name, dir)));
  const dirty = !!config && (name !== config.second_name || dir !== config.second_dir);
  return (
    <article className="adv-card adv-wide">
      <h3>{t("second.title")}</h3>
      <p className="warn-text small">{t("second.warn")}</p>
      <p className="hint">{t("second.what")}</p>
      <div className="form-row">
        <label className="field-label" htmlFor="p2-name">{t("second.name")}</label>
        <input id="p2-name" value={name} maxLength={20} onChange={(e) => setName(e.target.value)} style={{ width: "10rem" }} disabled={running2} />
        <label className="field-label" htmlFor="p2-dir">{t("second.dir")}</label>
        <input id="p2-dir" className="mono" value={dir} onChange={(e) => setDir(e.target.value)} style={{ width: "11rem" }} disabled={running2} />
        <button className="btn btn-ghost btn-small" disabled={!dirty || running2} onClick={() => void save()}>{t("common.save")}</button>
      </div>
      {pre?.second.path && <p className="tiny muted mono wrapany">{pre.second.path} · {pre.second.exists ? (pre.second.save_exists ? t("second.hassave") : t("second.nosave")) : t("second.new")}</p>}
      <p className="hint">{t("second.lan", { addr: pre?.second.lan_address ?? "127.0.0.3/8" })}</p>
      <div className="form-row">
        <select value={mode} onChange={(e) => setMode(e.target.value as ModeId)} aria-label={t("play.mode")} disabled={running2} style={{ width: "14rem" }}>
          {(["lan", "hub", "range"] as ModeId[]).map((m) => <option key={m} value={m}>{t(`play.mode.${m}.title` as Key)}</option>)}
        </select>
        <button className="btn btn-small" disabled={running2 || dirty || !!pre && !pre.second.dir_ok || (mode === "lan" && !(pre?.lan.enabled && pre.lan.ready))} onClick={() => void run(() => api.launchSecond({ mode }))}>{t("second.launch")}</button>
        {running2 && <button className="btn btn-danger btn-small" onClick={() => void run(api.game2Stop)}>{t("play.stop")}</button>}
      </div>
      {game.state === "running" && game2.state !== "running" && <p className="hint">{t("second.first.running")}</p>}
      <NoticeBar notice={notice} />
      <GameState status={game2} />
      {game2.state !== "idle" && <ProfileReport second />}
      {game2.mode === "lan" && game2.state === "running" && <LanHelp second />}
    </article>
  );
}

export default function AdvancedPlay({ pre, reload, launch, args, setArgs }: {
  pre: Preflight | null; reload: () => void; launch: (mode: ModeId, ignore?: boolean, untouched?: boolean) => Promise<void>; args: string; setArgs: (s: string) => void;
}) {
  const { t } = useI18n();
  const { game, status, config, setConfig, fail, go } = useApp();
  const errNotice = useErrorNotice();
  const [notice, setNotice] = useState<Notice | null>(null);
  const [tail, setTail] = useState<string | null>(null);
  const running = game.state === "running" || game.restarted;
  const ready = status?.ready ?? false;
  const run = async (f: () => Promise<unknown>) => {
    try {
      setNotice(null);
      await f();
    } catch (e) {
      setNotice(errNotice(e));
    }
  };
  const untouched = config?.ui.full_untouched ?? false;
  const choose = (v: boolean) => { if (config) void api.setUiPrefs({ ...config.ui, full_untouched: v }).then(setConfig).catch(fail); };
  const lan = pre?.modes.find((m) => m.id === "lan");
  const lanWhy = !lan?.enabled ? t("play.mode.off") : !pre?.lan.enabled ? t("play.mode.lan.whyoff") : !pre.lan.ready ? t("play.mode.lan.whyconf", { problem: pre.lan.problem ?? "" }) : null;
  return (
    <Section title={t("play.adv.title")} hint={t("play.adv.hint")} className="play-adv">
      <div className="adv-grid">
        <article className="adv-card">
          <h3>{t("play.run.title")}</h3>
          <GameState />
          {game.log_path && <p className="mono tiny muted wrapany">{game.log_path}</p>}
          <div className="row wrap adv-actions">
            <button className="btn btn-ghost btn-small" onClick={() => void run(api.openLogsFolder)}>{t("play.logs")}</button>
            <button className="btn btn-ghost btn-small" onClick={() => void run(async () => setTail((await api.readLogTail(80)) ?? ""))}>{t("play.showlog")}</button>
          </div>
          {tail !== null && <pre className="log">{tail || "—"}</pre>}
        </article>

        <article className="adv-card">
          <h3>{t("play.adv.launch")}</h3>
          <fieldset className="radios">
            <legend className="field-label">{t("tile.full.profile")}</legend>
            <label className="check"><input type="radio" name="full-profile" checked={!untouched} onChange={() => choose(false)} /> {t("tile.full.mine")}</label>
            <label className="check"><input type="radio" name="full-profile" checked={untouched} onChange={() => choose(true)} /> {t("tile.full.none")}</label>
          </fieldset>
          <p className="hint">{untouched ? t("tile.full.state.none") : t(config?.skip_first_launch ? "tile.full.state.mine.skip" : "tile.full.state.mine")}</p>
          <label className="field-label" htmlFor="extra-args">{t("play.args")}</label>
          <input id="extra-args" className="mono" value={args} onChange={(e) => setArgs(e.target.value)} placeholder="--seconds 30" disabled={running} />
          <p className="hint">{t("play.args.hint")}</p>
        </article>

        <article className="adv-card">
          <h3>{t("play.mode.lan.title")} <span className="badge badge-hot">{t("nav.experimental")}</span></h3>
          <p className="adv-desc">{t("play.mode.lan.desc", { mode: pre?.lan.mode === "relay" ? t("lan.mode.relay") : t("lan.mode.lan"), value: pre?.lan.value ?? "-" })}</p>
          {lanWhy ? <p className="adv-why">{lanWhy}</p> : <p className="tiny muted mono wrapany">{lan ? `${lan.var}=${lan.value}${pre?.lan.value ? `  KLM_LAN=${pre.lan.value}` : ""}` : ""}</p>}
          <div className="row wrap adv-actions">
            <button className="btn btn-small" disabled={!!lanWhy || running || !ready} onClick={() => void launch("lan")}>{t("play.launch")}</button>
            <button className="btn btn-small btn-ghost" onClick={() => go("settings", "lan")}>{lanWhy ? t("play.mode.lan.setup") : t("adv.configure")}</button>
          </div>
        </article>

        <NoticeBar notice={notice} />
        {game.mode === "lan" && <LanHelp />}
        <ProfileReport />
        <SecondPlayer pre={pre} reload={reload} />
      </div>
    </Section>
  );
}
