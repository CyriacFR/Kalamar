// How many players the servers have, for the pages that show it (Play, Community): read when the page opens, then every
// 30 s at most while it stays on screen, never while the window is hidden or the game runs. A failure shows nothing.
import { useEffect, useState } from "react";
import { useApp } from "../ctx";
import { localeOf } from "../locales";
import { proto, type Population } from "./api";

const EVERY_MS = 30_000;

/** The population of each of `ids` (servers that declare the area; the others are simply absent). */
export function usePopulation(ids: string[]): Record<string, Population> {
  const { game } = useApp();
  const running = game.state === "running" || game.restarted;
  const [pop, setPop] = useState<Record<string, Population>>({});
  const key = ids.join("\n");
  useEffect(() => {
    if (!key || running) {
      setPop({});
      return;
    }
    const list = key.split("\n");
    let alive = true;
    let last = 0;
    const tick = async () => {
      if (document.visibilityState !== "visible" || Date.now() - last < EVERY_MS - 1000) return;
      last = Date.now();
      const got = await Promise.all(list.map((id) => proto.population(id).catch(() => null)));
      if (alive) setPop(Object.fromEntries(list.flatMap((id, i) => (got[i] ? [[id, got[i] as Population]] : []))));
    };
    void tick();
    const timer = window.setInterval(() => void tick(), EVERY_MS);
    const shown = () => void tick();
    document.addEventListener("visibilitychange", shown);
    return () => {
      alive = false;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", shown);
    };
  }, [key, running]);
  return pop;
}

type T = (key: "pop.online" | "pop.match" | "pop.search", vars: Record<string, string>) => string;

/** "12 en ligne · 4 en partie · 2 en recherche": playing when the server says it, searching only when above zero. */
export function populationText(p: Population, lang: string, t: T, short = false): string {
  const n = (x: number) => new Intl.NumberFormat(localeOf(lang).tag).format(x);
  const parts = [t("pop.online", { n: n(p.online) })];
  if (!short && p.in_match != null) parts.push(t("pop.match", { n: n(p.in_match) }));
  if (!short && p.searching != null && p.searching > 0) parts.push(t("pop.search", { n: n(p.searching) }));
  return parts.join(" · ");
}
