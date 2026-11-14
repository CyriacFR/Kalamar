import { createContext, useContext } from "react";
import { formatDateTime } from "./locales";
import type { AppInfo, CatalogueView, ConfigView, GameStatus, InstallSnapshot, InstallStatus, Profile, QuickMode, ServersView } from "./api";

/** The four entries of the rail (`community` only with a server), and `setup`, the install wizard. The former
 *  installation, LAN and about pages are sections of the settings (`go("settings", "installed")`). */
export type PageId = "play" | "profile" | "community" | "settings" | "setup";

export interface Notice { kind: "ok" | "error" | "info"; text: string }

/** The local player as the rail's card shows it. */
export interface PlayerData { profile: Profile | null; view: CatalogueView | null }

export interface AppCtx {
  info: AppInfo | null;
  config: ConfigView | null;
  status: InstallStatus | null;
  game: GameStatus;
  /** The advanced second local player. */
  game2: GameStatus;
  lastRun: GameStatus | null;
  player: PlayerData;
  /** Loads the game catalogue for the rail's splashtag (opening the Locker does it; nothing at start-up). */
  reloadPlayer: () => Promise<void>;
  /** `--play <mode>` on the command line, waiting for the Play screen to start it (once). */
  cliPlay: QuickMode | null;
  clearCliPlay: () => void;
  /** Community servers and what they make visible (none: offline features only). */
  servers: ServersView | null;
  reloadServers: () => Promise<void>;
  refresh: () => Promise<void>;
  setConfig: (c: ConfigView) => void;
  /** Opens a page; `arg` is the section to show (a settings section, for example). */
  go: (p: PageId, arg?: string) => void;
  /** The `arg` of the last `go`, for the page that was opened with it. */
  pageArg: string | null;
  fail: (e: unknown) => void;
  toast: (n: Notice) => void;
  /** Advanced mode: logs, paths, environment, reports and the tools for tinkerers (off: the simple mode). */
  advanced: boolean;
  setAdvanced: (on: boolean) => void;
  /** An installation runs (the wizard may be closed): its overall progress, 0 to 1, else null. */
  installing: number | null;
  /** The installation of this session, live (running or ended), for the pages that follow it; null before one. */
  install: InstallSnapshot | null;
}

export const Ctx = createContext<AppCtx | null>(null);

export function useApp(): AppCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error("AppCtx missing");
  return c;
}

export function fmtDate(secs: number | null | undefined, lang: string): string {
  if (!secs) return "—";
  return formatDateTime(new Date(secs * 1000), lang);
}

export function fmtMB(bytes: number | null | undefined): string {
  return bytes == null ? "" : (bytes / 1048576).toFixed(bytes > 10485760 ? 0 : 1);
}
