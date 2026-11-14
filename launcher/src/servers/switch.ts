// Changing the server in use (the one the servers file marks selected: Play's main block, the Community page): the one
// way every place does it (the Play page's cards, the Community page's menu, Settings > Servers). Refused, with one
// sentence, while the player is in a room, a search or a match of the current server.
import { useCallback } from "react";
import { serversApi, type GameStatus } from "../api";
import { useApp } from "../ctx";
import { proto } from "../community/api";
import { useLive, type Live } from "../community/store";
import { useS } from "./texts";

export type SwitchBlock = "room" | "search" | "match";

const OVER = ["completed", "void", "cancelled"];
const RUNNER_IDLE = ["done", "refused", "failed", "closed"];

/** What keeps the player on `used`: a match (a game started for a match too), a room, a search. */
export function switchBlocker(live: Live, used: string | null, game: GameStatus): SwitchBlock | null {
  if ((game.state === "running" || game.restarted) && game.mode === "match") return "match";
  if (!used || live.server !== used) return null;
  if (live.match && !OVER.includes(live.match.state)) return "match";
  if (live.runner && !RUNNER_IDLE.includes(live.runner.phase)) return "match";
  if (live.room) return "room";
  if (live.ticket) return "search";
  return null;
}

export function useServerSwitch() {
  const { servers, reloadServers, game } = useApp();
  const { s } = useS();
  const live = useLive();
  const used = servers?.features.provider_id ?? null;
  const block = switchBlocker(live, used, game);
  const blocked = block ? s(`srv.switch.${block}`) : null;
  /** Makes `id` the server in use (the previous one's connection stops, as in Settings). `false` when refused. */
  const choose = useCallback(async (id: string): Promise<boolean> => {
    if (id === used) return true;
    if (blocked) return false;
    if (used) await proto.disconnect(used).catch(() => {});
    await serversApi.select(id);
    await reloadServers();
    return true;
  }, [used, blocked, reloadServers]);
  return { used, blocked, choose };
}
