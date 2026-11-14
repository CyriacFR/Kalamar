// Feeds the Discord presence (src-tauri/src/discord/) from what the launcher shows: the page, the game, and the live
// state of the community server in use. Mounted once by the shell (App.tsx). While the presence is off it only keeps
// the present situation for the Settings preview: nothing is handed to the back end, which would send nothing anyway.
// While on, each change of the resulting activity goes to `discord_publish` (the back end settles and rate-limits it).
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { discordApi, type ConfigView, type GameStatus, type ServersView } from "../api";
import { useLive } from "../community/store";
import { BRAND } from "../brand";
import { DICTS, type Key, type TFn } from "../i18n";
import { stageInfo, useStagesView } from "../stages";
import { DEFAULT_PREFS, buildActivity, situationOf, type Prefs, type Situation } from "./presence";

let current: Situation | null = null;
const listeners = new Set<() => void>();

/** The situation of this moment (the Settings preview's "Now"). */
export function useCurrentSituation(): Situation | null {
  return useSyncExternalStore((f) => { listeners.add(f); return () => { listeners.delete(f); }; }, () => current);
}

export function prefsOf(config: ConfigView | null): Prefs {
  return { ...DEFAULT_PREFS, ...(config?.discord ?? {}) };
}

const nowS = () => Math.floor(Date.now() / 1000);
/** The launcher's start (the page's first load), and when the player was first seen logged in on the server. */
const launcherSince = nowS();
const onlineSince: { current: number | null } = { current: null };

/** Rendered once by the shell; draws nothing. */
export function DiscordPresence(props: { config: ConfigView | null; game: GameStatus; page: string; servers: ServersView | null; lang: string; t: TFn }): null {
  useDiscordPresence(props);
  return null;
}

export function useDiscordPresence({ config, game, page, servers, lang, t }: {
  config: ConfigView | null; game: GameStatus; page: string; servers: ServersView | null; lang: string; t: TFn;
}): void {
  // A build without a Discord application has no presence: a choice kept in config.json from another build is ignored.
  const prefs = { ...prefsOf(config), ...(config?.discord_available ? {} : { enabled: false }) };
  const live = useLive();
  // The result of a match leaves the presence after a while: the situation is looked at again every 20 s.
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => setTick((n) => n + 1), 20_000);
    return () => window.clearInterval(id);
  }, []);

  const loggedIn = !!live.server && !!live.overview?.session.logged_in;
  if (loggedIn && onlineSince.current === null) onlineSince.current = nowS();
  if (!loggedIn) onlineSince.current = null;

  // Stage names come from the player's game data, read only when the player asked for them and a stage is involved.
  const stageId = [live.match?.rules?.stage, live.ended?.rules?.stage, live.room?.options?.stage].find((x) => typeof x === "string") ?? null;
  const stages = useStagesView(prefs.enabled && prefs.mode_stage && !!stageId);

  const situation = useMemo(() => {
    const serverName = servers?.servers.find((s) => s.id === live.server)?.name ?? null;
    const modeName = (id: string): string | null => {
      const own = live.names.modes[id];
      if (own) return own;
      const key = `mode.${id}` as Key;
      return key in DICTS.en ? t(key) : null;
    };
    const stageName = (id: string): string | null => {
      const s = stageInfo(stages, id, lang);
      return (s.named ? s.name : null) || live.names.stages[id] || null;
    };
    return situationOf({
      page,
      game: { state: game.state, mode: game.mode, started_at: game.started_at, restarted: game.restarted },
      launcherSince,
      onlineSince: onlineSince.current,
      server: loggedIn ? serverName : null,
      live: loggedIn ? { me: live.me?.id ?? live.overview?.session.user_id ?? null, room: live.room, ticket: live.ticket, party: live.party, match: live.match, ended: live.ended, queues: live.queueFacts } : null,
      modeName,
      stageName,
      now: nowS(),
    });
    // `tick` makes the result expire; the rest is what the situation is made of.
  }, [page, game, servers, live, loggedIn, stages, lang, t, tick]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (JSON.stringify(current) === JSON.stringify(situation)) return;
    current = situation;
    listeners.forEach((f) => f());
  }, [situation]);

  // While on: the activity, when it changed. Switching on again sends the present one whatever was sent before.
  const sent = useRef<string | null>(null);
  const tr = (key: string, vars?: Record<string, string | number>) => t(key as Key, vars);
  const activity = prefs.enabled ? buildActivity(situation, prefs, tr, BRAND.name) : null;
  const text = activity ? JSON.stringify(activity) : null;
  useEffect(() => {
    if (!prefs.enabled) {
      sent.current = null;
      return;
    }
    if (text === sent.current) return;
    sent.current = text;
    void discordApi.publish(activity).catch(() => { sent.current = null; });
  }, [prefs.enabled, text]); // eslint-disable-line react-hooks/exhaustive-deps
}
