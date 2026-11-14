// Settings, by use: mouse and aim, keyboard and controller, display, language, servers, the installed game, the
// advanced mode and about. The advanced mode adds the local network, the save backups and the rare fields of each section.
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { configureSound, play } from "../sound/engine";
import { LANGS, errorText, useI18n, type Key, type TFn } from "../i18n";
import { api, installApi, toAppError, updateApi, type BundleManifest, type Change, type DataFolder, type Schema, type SchemaField, type SettingsState, type UpdateChannel } from "../api";
import { useApp, type Notice } from "../ctx";
import { NoticeBar } from "../ui";
import { ImportBox } from "../MigrationNotice";
import { Bindings, FieldRow, MOUSE_KEYS, MouseAim, type Values } from "./settings/fields";
import { Installed } from "./settings/Installed";
import { LanSection } from "./settings/Lan";
import { ServerList } from "../servers/ServerList";
import { LauncherUpdateAbout, useLauncherUpdate } from "../update/LauncherUpdate";
import { markSeen, SETTINGS_SAVED } from "../install/seen";
import { DiscordSection } from "./settings/Discord";
import { GAME_LANGS, currentChoice, gameLangName, launcherToGame, needsFonts, optionDisabled, type GameChoice } from "../gameLang";

type SectionId = "mouse" | "controls" | "display" | "language" | "servers" | "discord" | "installed" | "lan" | "save" | "advanced" | "about";
const ORDER: SectionId[] = ["mouse", "controls", "display", "language", "servers", "discord", "installed", "lan", "save", "advanced", "about"];
const ADVANCED_ONLY: SectionId[] = ["lan", "save"];
/** Sections a build may not have: Discord exists only in a build that names a Discord application. */
const BUILD_ONLY: SectionId[] = ["discord"];

/** Which fields of settings.ini each section shows, in the schema's order; `simple` lists those of the simple mode
 *  (the advanced mode shows them all). */
const GAME_FIELDS: Partial<Record<SectionId, { sections: string[]; simple: string[] | "all"; skip?: string[] }>> = {
  mouse: { sections: ["mouse"], simple: ["mouse_gyro", "menu_cursor"], skip: MOUSE_KEYS },
  controls: { sections: ["controller"], simple: "all" },
  display: { sections: ["display", "performance"], simple: ["fullscreen", "window_size", "output_res", "fps_limit", "perf_overlay"] },
  save: { sections: ["save"], simple: "all" },
};

function Switch({ checked, onChange, label, hint }: { checked: boolean; onChange: (v: boolean) => void; label: ReactNode; hint?: ReactNode }) {
  return (
    <label className="switch-row">
      <span className="switch"><input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} /><span className="switch-ui" aria-hidden="true" /></span>
      <span className="switch-text"><span className="switch-label">{label}</span>{hint && <span className="hint">{hint}</span>}</span>
    </label>
  );
}

/** Settings > About, the launcher's updates: automatic check and channel (the block itself is in update/). */
function UpdatePrefsRows() {
  const { t } = useI18n();
  const { config, setConfig, fail } = useApp();
  const prefs = config?.update ?? { auto_check: true, channel: "stable" as UpdateChannel };
  const save = (patch: Partial<typeof prefs>) => void updateApi.setPrefs({ ...prefs, ...patch }).then(setConfig).catch(fail);
  return (
    <>
      <Switch checked={prefs.auto_check} onChange={(v) => save({ auto_check: v })} label={t("lup.auto")} hint={t("lup.auto.hint")} />
      <div className="row lup-channel">
        <span id="lup-channel-label">{t("lup.channel")}</span>
        <div className="seg" role="radiogroup" aria-labelledby="lup-channel-label">
          {(["stable", "beta"] as const).map((c) => (
            <button key={c} type="button" role="radio" aria-checked={prefs.channel === c} className={`seg-btn ${prefs.channel === c ? "on" : ""}`} onClick={() => save({ channel: c })}>
              {t(`lup.channel.${c}`)}
            </button>
          ))}
        </div>
      </div>
      <p className="hint">{t("lup.channel.hint")}</p>
    </>
  );
}

const FOLDER_WHY: Record<Extract<DataFolder, { kind: "deferred" }>["reason"], Key> = {
  old_launcher: "about.data.why.old_launcher",
  update_trial: "about.data.why.update_trial",
  in_use: "about.data.why.in_use",
  busy: "about.data.why.busy",
  game_in_folder: "about.data.why.game_in_folder",
  failed: "about.data.why.failed",
};

/** A line under the data folder when this start moved it from an earlier launcher's, kept that one for the session, or
 *  found both (datadir.rs). */
function dataFolderNote(f: DataFolder | undefined, t: TFn): string | null {
  if (f?.kind === "moved") return t(f.mode === "split" ? "about.data.moved.split" : "about.data.moved", { from: f.from });
  if (f?.kind === "deferred") return t("about.data.deferred", { from: f.from, why: t(FOLDER_WHY[f.reason]) });
  if (f?.kind === "both" && f.left === "data") return t("about.data.both", { from: f.from });
  return null;
}

function About() {
  const { t } = useI18n();
  const { info, advanced, config } = useApp();
  const folderNote = dataFolderNote(info?.data_folder, t);
  const upd = useLauncherUpdate();
  const [bundle, setBundle] = useState<BundleManifest | null>(null);
  useEffect(() => { if (advanced) void installApi.info().then((i) => setBundle(i.bundle)).catch(() => setBundle(null)); }, [advanced]);
  return (
    <div className="stack about">
      <p className="about-notice">{t("about.notice")}</p>
      <p>{t("about.privacy")}</p>
      {upd && <p>{t(upd.enabled ? "about.privacy.update" : "about.privacy.noupdate")}</p>}
      {config?.discord_available && <p>{t("about.privacy.discord")}</p>}
      <LauncherUpdateAbout><UpdatePrefsRows /></LauncherUpdateAbout>
      <dl className="facts">
        <dt>{t("about.version")}</dt><dd>{info?.version ?? "—"} · {t("about.license")}</dd>
        <dt>{t("app.name")}</dt><dd>{t("about.name.placeholder")}</dd>
        {advanced && <><dt>{t("about.bundle")}</dt><dd className="mono">{bundle ? `${bundle.commit.slice(0, 12)} · ${t("about.bundle.files", { n: bundle.files })}` : "—"}</dd></>}
        {(advanced || folderNote) && <><dt>{t("about.data")}</dt><dd className="mono wrapany">{info?.data_dir}</dd></>}
      </dl>
      {folderNote && <p className="hint">{folderNote}</p>}
    </div>
  );
}

function AdvancedMode() {
  const { t } = useI18n();
  const { advanced, setAdvanced } = useApp();
  return (
    <div className="stack">
      <Switch checked={advanced} onChange={setAdvanced} label={t("set.adv.switch")} hint={t("set.adv.what")} />
      {advanced && (
        <>
          <div className="set-block">
            <div className="set-block-head"><h3>{t("settings.shortcut")}</h3></div>
            <p className="hint">{t("settings.shortcut.hint")}</p>
            <p className="mono small">Kalamar.exe --play training --minimized</p>
          </div>
          <div className="set-block">
            <div className="set-block-head"><h3>{t("mig.title")}</h3></div>
            <ImportBox />
          </div>
        </>
      )}
    </div>
  );
}

/** The game's language, under the launcher's: "like the launcher" (default), "like Windows", then each language of the
 *  game in its own name. Applied at once (a language is not a field to review), like the launcher's own. */
function GameLanguage({ iniLang, ready, onPick }: { iniLang: string | undefined; ready: boolean; onPick: (c: GameChoice) => void }) {
  const { t, lang } = useI18n();
  const { config } = useApp();
  const [fonts, setFonts] = useState<boolean | null>(null);
  useEffect(() => { void api.gameLanguageInfo().then((i) => setFonts(i.fonts_ok)).catch(() => setFonts(false)); }, [config?.root, iniLang]);
  const choice = currentChoice(iniLang, config?.game_lang_follow);
  const fontsOk = fonts !== false;
  const missingFonts = fonts === false;
  const needsMissing = missingFonts && choice !== "launcher" && choice !== "windows" && needsFonts(choice);
  const special = (id: "launcher" | "windows", sub: string) => (
    <button key={id} type="button" role="radio" aria-checked={choice === id} className={`lang-opt lang-opt-sub ${choice === id ? "on" : ""}`} onClick={() => onPick(id)}>
      <span className="lang-text"><span className="lang-name">{t(`set.gamelang.${id}`)}</span><span className="lang-sub">{sub}</span></span>
    </button>
  );
  return (
    <div className="set-block" id="game-language">
      <div className="set-block-head"><h3 id="game-lang-title">{t("set.gamelang.title")}</h3></div>
      <p className="hint">{t("set.gamelang.hint")}</p>
      <div className="lang-list lang-list-special" role="radiogroup" aria-labelledby="game-lang-title">
        {special("launcher", t("set.gamelang.launcher.sub", { name: gameLangName(launcherToGame(lang)) }))}
        {special("windows", t("set.gamelang.windows.sub"))}
      </div>
      <div className="lang-list" role="radiogroup" aria-labelledby="game-lang-title" data-testid="game-langs">
        {GAME_LANGS.map((l) => {
          const off = !ready || (optionDisabled(l.code, fontsOk) && choice !== l.code);
          return (
            <button key={l.code} type="button" role="radio" aria-checked={choice === l.code} disabled={off} lang={l.code} data-game-lang={l.code}
              className={`lang-opt ${choice === l.code ? "on" : ""} ${off ? "off" : ""}`} onClick={() => onPick(l.code)}>
              <span className="lang-name">{l.name}</span>
            </button>
          );
        })}
      </div>
      {missingFonts && <p className="hint" role="note">{t("set.gamelang.fonts")}</p>}
      {needsMissing && <p className="hint warn-text" role="alert">{t("set.gamelang.fonts.chosen")}</p>}
      {!ready && <p className="hint">{t("set.gamelang.nogame")}</p>}
    </div>
  );
}

function Language({ iniLang, ready, onPick }: { iniLang: string | undefined; ready: boolean; onPick: (c: GameChoice) => void }) {
  const { t, lang } = useI18n();
  const { setConfig, fail } = useApp();
  return (
    <div className="stack">
      <div className="set-block">
        <div className="set-block-head"><h3 id="launcher-lang-title">{t("set.launcher.title")}</h3></div>
        <div className="lang-list" role="radiogroup" aria-labelledby="launcher-lang-title">
          {LANGS.map((l) => (
            <button key={l.code} type="button" role="radio" aria-checked={lang === l.code} lang={l.code} className={`lang-opt ${lang === l.code ? "on" : ""}`}
              onClick={() => void api.setLanguage(l.code).then(setConfig).catch(fail)}>
              <span className="lang-name">{l.name}</span>
              <span className="lang-code">{l.code.toUpperCase()}</span>
            </button>
          ))}
        </div>
      </div>
      <GameLanguage iniLang={iniLang} ready={ready} onPick={onPick} />
    </div>
  );
}

export default function Settings({ notify }: { notify: (n: Notice | null) => void }) {
  const { t } = useI18n();
  const { config, setConfig, status, go, pageArg, advanced, fail, install } = useApp();
  const [schema, setSchema] = useState<Schema | null>(null);
  const [loaded, setLoaded] = useState<SettingsState | null>(null);
  const [values, setValues] = useState<Values>({});
  const [section, setSection] = useState<SectionId>(() => (ORDER.includes(pageArg as SectionId) ? (pageArg as SectionId) : "mouse"));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [notice, setNotice] = useState<Notice | null>(null);
  const [noRoot, setNoRoot] = useState(false);
  // The volume slider moves a local value; it is saved when released.
  const [volume, setVolume] = useState(config?.ui.sound_volume ?? 35);
  useEffect(() => { if (config) setVolume(config.ui.sound_volume); }, [config?.ui.sound_volume]); // eslint-disable-line react-hooks/exhaustive-deps
  const commitVolume = () => { if (config && volume !== config.ui.sound_volume) void api.setUiPrefs({ ...config.ui, sound_volume: volume }).then(setConfig).catch(fail); };

  useEffect(() => { if (pageArg && ORDER.includes(pageArg as SectionId)) setSection(pageArg as SectionId); }, [pageArg]);
  // A section looked at counts for the "while it compiles" cards (keys checked, display seen).
  useEffect(() => { markSeen(`set.${section}`); }, [section]);
  // Leaving the advanced mode while on one of its sections.
  useEffect(() => { if (!advanced && ADVANCED_ONLY.includes(section)) setSection("advanced"); }, [advanced, section]);
  // A section this build does not have (opened by name).
  useEffect(() => { if (section === "discord" && config && !config.discord_available) setSection("mouse"); }, [config, section]);

  const load = useCallback(async () => {
    try {
      const [s, st] = await Promise.all([api.settingsSchema(), api.settingsLoad()]);
      setSchema(s);
      setLoaded(st);
      setValues(st.values);
      setErrors({});
      setNotice(null);
      setNoRoot(false);
    } catch (e) {
      const err = toAppError(e);
      setLoaded(null);
      if (err.kind === "no_root") setNoRoot(true);
      else setNotice({ kind: "error", text: errorText(err, t) });
      try {
        setSchema(await api.settingsSchema());
      } catch {
        /* the schema is embedded; nothing more to do */
      }
    }
  }, [t]);

  // Reload when the game folder changes (and when an installation of this session starts or ends: its folder is the
  // game's settings folder from its start).
  useEffect(() => { void load(); }, [load, config?.root, status?.root, install?.install_dir, install?.active]);

  const changes = useMemo<Change[]>(() => {
    if (!loaded) return [];
    const keys = new Set([...Object.keys(loaded.values), ...Object.keys(values)]);
    const out: Change[] = [];
    for (const k of keys) if ((values[k] ?? null) !== (loaded.values[k] ?? null)) out.push({ key: k, value: values[k] ?? null });
    return out;
  }, [loaded, values]);

  const setValue = (k: string, v: string | undefined) => {
    setValues((cur) => {
      const next = { ...cur };
      if (v === undefined) delete next[k];
      else next[k] = v;
      return next;
    });
    setErrors((cur) => {
      if (!(k in cur)) return cur;
      const next = { ...cur };
      delete next[k];
      return next;
    });
  };

  const save = async () => {
    try {
      const st = await api.settingsSave(changes);
      setLoaded(st);
      setValues(st.values);
      setErrors({});
      setNotice(null);
      notify({ kind: "ok", text: t("common.saved") });
      window.dispatchEvent(new Event(SETTINGS_SAVED));
    } catch (e) {
      const err = toAppError(e);
      if (err.kind === "validation") setErrors(Object.fromEntries(err.fields.map((f) => [f.key, f.message])));
      setNotice({ kind: "error", text: errorText(err, t) });
    }
  };

  // The game's language is applied at once: `lang` of the settings file for a language, nothing for "like the
  // launcher" / "like Windows", whose choice is the launcher's (the other unsaved edits of the page stay as they are).
  const pickGameLang = async (c: GameChoice) => {
    const explicit = c !== "launcher" && c !== "windows";
    try {
      if (loaded) {
        const st = await api.settingsSave([{ key: "lang", value: explicit ? c : null }]);
        const merge = (cur: Values): Values => {
          const next = { ...cur };
          if (st.values.lang === undefined) delete next.lang;
          else next.lang = st.values.lang;
          return next;
        };
        setLoaded((l) => (l ? { ...l, exists: st.exists, values: merge(l.values) } : st));
        setValues(merge);
      }
      if (!explicit) setConfig(await api.setGameLangFollow(c));
      notify({ kind: "ok", text: t("set.gamelang.set") });
    } catch (e) {
      const err = toAppError(e);
      setNotice({ kind: "error", text: errorText(err, t) });
    }
  };

  const fieldsOf = (id: SectionId): SchemaField[] => {
    const g = GAME_FIELDS[id];
    if (!g || !schema) return [];
    return schema.fields.filter((f) => g.sections.includes(f.section) && !(g.skip ?? []).includes(f.key) && (advanced || g.simple === "all" || g.simple.includes(f.key)));
  };
  const rows = (id: SectionId) => fieldsOf(id).map((f) => <FieldRow key={f.key} f={f} value={values[f.key]} error={errors[f.key]} onChange={(v) => setValue(f.key, v)} />);
  const dirty = changes.length > 0;
  const isGame = !!GAME_FIELDS[section];
  const built = (s: SectionId) => !BUILD_ONLY.includes(s) || (s === "discord" && !!config?.discord_available);
  const visible = ORDER.filter((s) => (advanced || !ADVANCED_ONLY.includes(s)) && built(s));
  const label = (s: SectionId) => t(`set.nav.${s}` as Key);
  const hint = (s: SectionId) => t(`set.hint.${s}` as Key);

  let body: ReactNode = null;
  if (isGame && noRoot) {
    body = (
      <div className="empty-line">
        <p>{t("set.noroot")}</p>
        <button className="btn btn-small" onClick={() => go("setup")}>{t("play.install")}</button>
      </div>
    );
  } else if (isGame && (!schema || !loaded)) {
    body = notice ? null : <span className="skeleton" style={{ width: "100%", height: "8rem" }} />;
  } else if (section === "mouse" && schema) {
    body = (
      <>
        <MouseAim values={values} errors={errors} setValue={setValue} />
        <div className="set-block">
          <div className="set-rows">{rows("mouse")}</div>
          <p className="hint">{t("settings.mouse.motion2")}</p>
        </div>
      </>
    );
  } else if (section === "controls" && schema) {
    body = (
      <>
        <Bindings schema={schema} values={values} errors={errors} setValue={setValue} />
        <div className="set-block">
          <div className="set-block-head"><h3>{t("set.controller")}</h3></div>
          <div className="set-rows">{rows("controls")}</div>
        </div>
      </>
    );
  } else if (section === "display") {
    body = (
      <>
        <div className="set-rows">{rows("display")}</div>
        <div className="set-block">
          <div className="set-block-head"><h3>{t("set.window")}</h3></div>
          <Switch checked={config?.ui.minimize_on_play ?? false} onChange={(v) => config && void api.setUiPrefs({ ...config.ui, minimize_on_play: v }).then(setConfig).catch(fail)}
            label={t("settings.minimize")} hint={t("settings.minimize.hint")} />
        </div>
        <div className="set-block">
          <div className="set-block-head"><h3>{t("set.sounds")}</h3></div>
          <Switch checked={config?.ui.sounds ?? true} onChange={(v) => config && void api.setUiPrefs({ ...config.ui, sounds: v }).then(setConfig).catch(fail)}
            label={t("settings.sounds")} hint={t("settings.sounds.hint")} />
          <div className="row">
            <label htmlFor="sound-volume">{t("settings.sounds.volume")}</label>
            <input id="sound-volume" type="range" min={0} max={100} step={5} value={volume} disabled={!(config?.ui.sounds ?? true)}
              aria-valuetext={`${volume} %`} onChange={(e) => setVolume(Number(e.target.value))}
              onPointerUp={commitVolume} onKeyUp={commitVolume} onBlur={commitVolume} />
            <span className="small muted" aria-hidden="true">{volume} %</span>
            <button type="button" className="btn btn-small btn-ghost" onClick={() => { configureSound({ volume }); play("confirm", { force: true }); }}>{t("settings.sounds.test")}</button>
          </div>
        </div>
        <div className="set-block">
          <div className="set-block-head"><h3>{t("set.notify")}</h3></div>
          <Switch checked={config?.ui.notify ?? true} onChange={(v) => config && void api.setUiPrefs({ ...config.ui, notify: v }).then(setConfig).catch(fail)}
            label={t("settings.notify")} hint={t("settings.notify.hint")} />
        </div>
      </>
    );
  } else if (section === "save") {
    body = <div className="set-rows">{rows("save")}</div>;
  } else if (section === "language") body = <Language iniLang={values.lang} ready={!!loaded} onPick={(c) => void pickGameLang(c)} />;
  else if (section === "servers") body = <ServerList />;
  else if (section === "discord" && config?.discord_available) body = <DiscordSection />;
  else if (section === "installed") body = <Installed />;
  else if (section === "lan") body = <LanSection />;
  else if (section === "advanced") body = <AdvancedMode />;
  else if (section === "about") body = <About />;

  return (
    <section className="page page-settings">
      <header className="page-head">
        <div className="page-head-text">
          <h1 className="display page-title">{t("set.title")}</h1>
        </div>
      </header>
      <div className="set-layout">
        <nav className="set-nav" aria-label={t("set.title")}>
          {visible.map((s, i) => (
            <button key={s} data-section={s} aria-current={section === s ? "true" : undefined}
              className={`set-tab ${section === s ? "on" : ""} ${ADVANCED_ONLY.includes(s) ? "adv-only" : ""} ${i > 0 && ADVANCED_ONLY.includes(s) && !ADVANCED_ONLY.includes(visible[i - 1]) ? "first-adv" : ""}`}
              onClick={() => setSection(s)}>
              {label(s)}
            </button>
          ))}
        </nav>
        <div className="set-panel" key={section}>
          <header className="set-head">
            <h2 className="section-title">{label(section)}</h2>
            <p className="section-hint">{hint(section)}</p>
            {advanced && isGame && loaded && <p className="tiny muted mono wrapany">{t("settings.subtitle", { path: loaded.path })}{!loaded.exists ? ` — ${t("settings.nofile")}` : ""}{loaded.unknown_keys.length > 0 ? ` — ${t("settings.unknown", { n: loaded.unknown_keys.length })}` : ""}</p>}
          </header>
          {isGame && install?.active && loaded && <p className="set-installing" role="status">{t(install.update ? "set.installing.update" : "set.installing")}</p>}
          {(isGame || section === "language") && <NoticeBar notice={notice} />}
          {body}
        </div>
      </div>
      {dirty && (
        <div className="savebar" role="region" aria-label={t("settings.dirty")}>
          <span className="savebar-text">{t("settings.dirty")} ({changes.length})</span>
          <button className="btn btn-ghost btn-small" onClick={() => { if (loaded) { setValues(loaded.values); setErrors({}); } }}>{t("settings.revert")}</button>
          <button className="btn btn-small" onClick={() => void save()}>{t("common.save")}</button>
        </div>
      )}
    </section>
  );
}
