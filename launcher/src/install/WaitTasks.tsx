// "While it compiles": cards of what the player can set up during an installation, checked as it gets done, on the
// install screen (under the progress) and, once the game is ready, as the recap of the "Ready" screen. None of it
// touches the installation: the profile is the launcher's own file, the game's settings file is not written by any
// step, and the Locker reads the game data only once the merge has finished (Snapshot.game_data).
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { api, type InstallSnapshot } from "../api";
import { proto } from "../community/api";
import { useApp } from "../ctx";
import { LANGS, useI18n } from "../i18n";
import { IconBolt, IconCheck, IconFriends, IconLocker, IconLock, IconMouse, IconScreen } from "../icons";
import { AddServerModal } from "../servers/AddServer";
import { approx, useW } from "./texts";
import { markSeen, SETTINGS_SAVED, useSeen } from "./seen";
import { progressOf, taskStates, type TaskFacts, type TaskId, type TaskState } from "./wait";

const APPEARANCE = ["style", "hair", "eyebrows", "legwear", "skin", "eyes"] as const;

/** What the cards are decided from: the profile and servers of the launcher, the game's settings file, the install. */
export function useWaitFacts(snap: InstallSnapshot | null): TaskFacts & { serverName: string | null } {
  const { player, servers, status } = useApp();
  const seen = useSeen();
  const [settings, setSettings] = useState<Record<string, string> | null>(null);
  const [loggedIn, setLoggedIn] = useState<boolean | null>(null);
  useEffect(() => {
    const load = () => void api.settingsLoad().then((s) => setSettings(s.values)).catch(() => setSettings(null));
    load();
    window.addEventListener(SETTINGS_SAVED, load);
    window.addEventListener("focus", load);
    return () => { window.removeEventListener(SETTINGS_SAVED, load); window.removeEventListener("focus", load); };
  }, []);
  const provider = servers?.servers.find((s) => s.id === servers.features.provider_id) ?? null;
  useEffect(() => {
    if (!provider?.api) { setLoggedIn(null); return; }
    let alive = true;
    void proto.peek().then((list) => { if (alive) setLoggedIn(list.find((p) => p.id === provider.id)?.session.logged_in ?? false); }).catch(() => {});
    return () => { alive = false; };
  }, [provider?.id, provider?.api]);
  const p = player.profile;
  return {
    gameData: snap ? snap.game_data : !!status?.ready,
    hasName: !!p?.name.trim(),
    profileComplete: !!p?.name.trim() && APPEARANCE.every((k) => p[k] != null),
    settings,
    seen,
    servers: servers?.features.servers ?? 0,
    loggedIn,
    serverName: provider?.name ?? null,
  };
}

function Card({ s, icon, title, state, children }: { s: TaskState; icon: ReactNode; title: string; state: ReactNode; children?: ReactNode }) {
  const { w } = useW();
  const cls = s.done ? "is-done" : s.locked ? "is-locked" : s.partial ? "is-partial" : "";
  return (
    <li className={`wait-card ${cls}`} data-task={s.id}>
      <span className="wait-ico" aria-hidden="true">{s.done ? <IconCheck /> : s.locked ? <IconLock /> : icon}</span>
      <div className="wait-text">
        <h3 className="wait-name">
          {title}
          {s.done && <span className="sr-only"> · {w("wait.done")}</span>}
          {s.optional && !s.done && <span className="wait-tag">{w("wait.optional")}</span>}
        </h3>
        <p className="wait-state">{state}</p>
      </div>
      {children && <div className="wait-act">{children}</div>}
    </li>
  );
}

/** The cards. `variant`: under the progress of a running install, or the recap of the "Ready" screen. */
export function WaitTasks({ snap, update }: { snap: InstallSnapshot | null; update?: boolean }) {
  const { w } = useW();
  const { lang } = useI18n();
  const { go, player, setConfig, fail, reloadServers } = useApp();
  const facts = useWaitFacts(snap);
  const states = useMemo(() => taskStates(facts), [facts]);
  const by = Object.fromEntries(states.map((s) => [s.id, s])) as Record<TaskId, TaskState>;
  const count = progressOf(states);
  const [adding, setAdding] = useState(false);
  const [native, setNative] = useState(facts.seen.has("native"));
  const name = player.profile?.name.trim() ?? "";
  const keys = Object.keys(facts.settings ?? {});
  const mouse = keys.some((k) => k.startsWith("mouse_gyro") || k === "mouse_sens") || facts.seen.has("set.mouse");
  const eta = snap?.game_data_eta;
  // The merge's own bar while it runs beside the other steps.
  const merge = snap?.background.find((b) => b.id === "romfs") ?? null;

  const locker = by.locker.locked
    ? (eta != null && eta > 0 ? w("wait.locker.locked", { t: approx(eta) }) : w("wait.locker.locked0"))
    : by.locker.done ? w("wait.locker.done", { name }) : by.locker.partial ? w("wait.locker.partial", { name }) : w("wait.locker.todo");
  const controls = by.controls.done ? w("wait.controls.done") : by.controls.partial ? w(mouse ? "wait.controls.partial.mouse" : "wait.controls.partial.keys") : w("wait.controls.todo");

  return (
    <section className="wait" aria-labelledby="wait-title">
      <header className="wait-head">
        <div className="grow">
          <div className="eyebrow wait-eyebrow">{w(update ? "wait.eyebrow.update" : "wait.eyebrow")}</div>
          <h2 id="wait-title" className="wait-title">{w("wait.title")}</h2>
          <p className="wait-lead">{w("wait.lead")}</p>
        </div>
        <div className="wait-count" aria-label={w("wait.count", { done: count.done, total: count.total })}>
          <span className="wait-count-n display">{count.done}</span><span className="wait-count-of">/{count.total}</span>
        </div>
      </header>
      <ul className="wait-list">
        <Card s={by.locker} icon={<IconLocker />} title={w("wait.locker.title")} state={<>
          {locker}
          {by.locker.locked && merge && <span className="wait-mini" aria-hidden="true"><span style={{ width: `${Math.round((merge.fraction ?? 0) * 100)}%` }} /></span>}
        </>}>
          {by.locker.locked
            ? <span className="wait-eta" title={locker}>{eta != null && eta > 0 ? `~${approx(eta)}` : w("wait.locker.soon")}</span>
            : <button className={`btn btn-small ${by.locker.done ? "btn-ghost" : ""}`} onClick={() => go("profile")}>{w("wait.locker.go")}</button>}
        </Card>
        <Card s={by.controls} icon={<IconMouse />} title={w("wait.controls.title")} state={controls}>
          <button className="btn btn-small btn-ghost" onClick={() => { markSeen("set.mouse"); go("settings", "mouse"); }}>{w("wait.controls.mouse")}</button>
          <button className="btn btn-small btn-ghost" onClick={() => { markSeen("set.controls"); go("settings", "controls"); }}>{w("wait.controls.keys")}</button>
        </Card>
        <Card s={by.display} icon={<IconScreen />} title={w("wait.display.title")} state={by.display.done ? w("wait.display.done") : w("wait.display.todo")}>
          <label className="wait-lang">
            <span className="sr-only">{w("wait.display.lang")}</span>
            <select value={lang} aria-label={w("wait.display.lang")} onChange={(e) => { markSeen("language"); void api.setLanguage(e.target.value).then(setConfig).catch(fail); }}>
              {LANGS.map((l) => <option key={l.code} value={l.code} lang={l.code}>{l.name}</option>)}
            </select>
          </label>
          <button className="btn btn-small btn-ghost" onClick={() => { markSeen("set.display"); go("settings", "display"); }}>{w("wait.display.go")}</button>
        </Card>
        <Card s={by.server} icon={<IconFriends />} title={w("wait.server.title")}
          state={by.server.done ? w("wait.server.done", { name: facts.serverName ?? "" }) : by.server.partial ? w("wait.server.partial", { name: facts.serverName ?? "" }) : w("wait.server.todo")}>
          {facts.servers === 0 && <button className="btn btn-small btn-ghost" onClick={() => setAdding(true)}>{w("wait.server.add")}</button>}
          {by.server.partial && <button className="btn btn-small" onClick={() => go("community")}>{w("wait.server.login")}</button>}
        </Card>
        <Card s={by.native} icon={<IconBolt />} title={w("wait.native.title")} state={native ? (
          <span className="wait-native">{[1, 2, 3].map((n) => <span key={n}>{w(`wait.native.${n}`)}</span>)}</span>
        ) : w("wait.native.todo")}>
          {!native && <button className="btn btn-small btn-ghost" aria-expanded={false} onClick={() => { setNative(true); markSeen("native"); }}>{w("wait.native.read")}</button>}
        </Card>
      </ul>
      {adding && <AddServerModal onClose={() => setAdding(false)} onAdded={() => { setAdding(false); void reloadServers(); }} />}
    </section>
  );
}

/** One line of the "Ready" recap. */
interface RecapItem { id: string; done: boolean; text: string; optional?: boolean; open?: () => void }

/** The "Ready" screen's recap: what is set (the mouse and the keys apart), what is left, each with its button. */
export function ReadyRecap() {
  const { w } = useW();
  const { lang } = useI18n();
  const { go, player } = useApp();
  const facts = useWaitFacts(null);
  const by = Object.fromEntries(taskStates({ ...facts, gameData: true }).map((s) => [s.id, s])) as Record<TaskId, TaskState>;
  const name = player.profile?.name.trim() ?? "";
  const keys = Object.keys(facts.settings ?? {});
  const mouse = keys.some((k) => k.startsWith("mouse_gyro") || k === "mouse_sens") || facts.seen.has("set.mouse");
  const binds = keys.some((k) => k.startsWith("bind_")) || facts.seen.has("set.controls");
  const settings = (section: string) => () => { markSeen(`set.${section}`); go("settings", section); };
  const items: RecapItem[] = [
    { id: "lang", done: true, text: w("ready.lang", { lang: LANGS.find((l) => l.code === lang)?.name ?? lang }) },
    { id: "locker", done: by.locker.done, text: by.locker.done ? w("ready.item.locker", { name }) : by.locker.partial ? w("ready.item.looks", { name }) : w("wait.locker.title"), open: () => go("profile") },
    { id: "mouse", done: mouse, text: w("ready.item.mouse"), open: settings("mouse") },
    { id: "keys", done: binds, text: w("ready.item.keys"), open: settings("controls") },
    { id: "display", done: by.display.done, text: w("ready.item.display"), open: settings("display") },
    {
      id: "server", done: by.server.done, optional: !by.server.partial,
      text: by.server.done ? w("ready.item.server", { name: facts.serverName ?? "" }) : by.server.partial ? w("ready.item.login", { name: facts.serverName ?? "" }) : w("wait.server.title"),
      open: () => (facts.servers > 0 ? go("community") : go("settings", "servers")),
    },
  ];
  const done = items.filter((i) => i.done);
  const left = items.filter((i) => !i.done);
  return (
    <div className="ready-recap">
      <div className="card ready-col">
        <h2 className="card-title">{w("ready.set")}</h2>
        <ul className="ready-list">
          {done.map((i) => <li key={i.id}><IconCheck /><span>{i.text}</span></li>)}
        </ul>
      </div>
      <div className="card ready-col">
        <h2 className="card-title">{w("ready.left")}</h2>
        {left.length === 0 ? <p className="small muted ready-none">{w("ready.left.none")}</p> : (
          <ul className="ready-list">
            {left.map((i) => (
              <li key={i.id}>
                <span className="ready-dot" aria-hidden="true" />
                <span className="grow">{i.text}{i.optional && <span className="wait-tag">{w("wait.optional")}</span>}</span>
                {i.open && <button className="btn btn-small btn-ghost" onClick={i.open}>{w("ready.do")}</button>}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
