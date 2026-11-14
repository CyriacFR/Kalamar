// Where the sounds that come from state (not from a click) are decided. Clicks call `play` from their own handler and
// errors come from the launcher's toast; no other audio logic lives outside src/sound.
import { useEffect, useRef } from "react";
import type { ConfigView, GameStatus } from "../api";
import { configureSound, isSpectating, play, setGameRunning } from "./engine";

const OFFLINE_MODES = ["original", "range", "hub"];

/** Preferences, game in front, the game of a match starting. Mounted once, in the app. */
export function useAppSounds(config: ConfigView | null, game: GameStatus): void {
  const on = config?.ui.sounds ?? true;
  const volume = config?.ui.sound_volume ?? 35;
  useEffect(() => { configureSound({ enabled: on, volume }); }, [on, volume]);
  useEffect(() => { setGameRunning(game.state === "running"); }, [game.state]);

  const last = useRef(game.state);
  useEffect(() => {
    // The game of a match (private match, room, queue) starts: the player may be looking elsewhere.
    // 1.1.7: not for a spectator: the game coming up is not a call to play.
    if (last.current !== "running" && game.state === "running" && game.mode && !OFFLINE_MODES.includes(game.mode) && !isSpectating()) play("found");
    last.current = game.state;
  }, [game.state, game.mode]);
}

interface Member { user?: { id?: string }; ready?: boolean }

/** A match found by a queue, somebody joining the room or becoming ready. Mounted with the community overlay. */
export function useCommunitySounds(match: { id?: string; state?: string } | null, accept: string | undefined, room: { id?: string; members?: Member[] } | null, me: string | null): void {
  const found = useRef<string | null>(null);
  useEffect(() => {
    if (match?.id && match.state === "pending_accept" && accept === "pending" && found.current !== match.id) { found.current = match.id; play("found"); }
  }, [match?.id, match?.state, accept]);

  const seen = useRef<{ id?: string; people: Set<string>; ready: Set<string> } | null>(null);
  useEffect(() => {
    if (!room) { seen.current = null; return; }
    const members = room.members ?? [];
    const people = new Set(members.map((x) => String(x.user?.id)));
    const ready = new Set(members.filter((x) => x.ready).map((x) => String(x.user?.id)));
    const prev = seen.current;
    // Not for the first state read of a room, nor for oneself.
    if (prev && prev.id === room.id && ([...people].some((id) => !prev.people.has(id) && id !== me) || [...ready].some((id) => !prev.ready.has(id) && id !== me))) play("soft");
    seen.current = { id: room.id, people, ready };
  }, [room, me]);
}
