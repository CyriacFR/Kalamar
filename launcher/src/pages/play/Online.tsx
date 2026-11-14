// The parts of the Play page around the main block: what the selected community server is and offers (from this
// computer only: the servers file, `proto_peek`, and the community page's live state once it was opened), the other
// servers, the solo block, the short note on what this launcher is, and the "add a server" card without a server.
// The only request to a server is its player count, when it declares one (`community/population.ts`). The server's
// texts are data, set as text.
import { lazy, Suspense, useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { ServerView, ServersView } from "../../api";
import { useApp } from "../../ctx";
import { useI18n } from "../../i18n";
import { lookup } from "../../locales";
import { IconAlert, IconChevron, IconCompass, IconFriends, IconPlay, IconPlus, IconRelay, IconTarget } from "../../icons";
import { proto, type J, type Peek, type Population, type SessionInfo } from "../../community/api";
import { populationText, usePopulation } from "../../community/population";
import { useLive } from "../../community/store";
import { useServerSwitch } from "../../servers/switch";
import { useS } from "../../servers/texts";

const AddServerModal = lazy(() => import("../../servers/AddServer").then((m) => ({ default: m.AddServerModal })));

/** What the player is doing on the server right now, when the community page's live state already says it. */
export type OnlineLive = { kind: "found" } | { kind: "match" } | { kind: "room"; code: string | null } | { kind: "search" };

export interface OnlineView {
  server: ServerView;
  /** A community service (accounts, rooms, queues...), else a relay alone (private match by code). */
  service: boolean;
  description: string;
  /** What the server declares it offers, in the words of its community page's tabs (at most four). */
  pills: string[];
  /** `null` while not known yet (the first answer of `proto_peek`). */
  session: SessionInfo | null;
  name: string | null;
  live: OnlineLive | null;
  /** Every saved server with a service, as this computer knows it. */
  peeks: Peek[];
  /** Player counts of the servers that publish one (read while this page is on screen). */
  pop: Record<string, Population>;
}

/** A server's localised text: only an object of strings counts (anything else the server sent is ignored). */
function text(v: J, lang: string): string {
  if (!v || typeof v !== "object" || Array.isArray(v)) return "";
  const s = v[lang] ?? v.en ?? Object.values(v).find((x) => typeof x === "string" && x.trim());
  return typeof s === "string" ? s.trim() : "";
}

// Declared areas -> the community page's tab that shows them; rankings once for ratings and leaderboards.
const PILLS: [string[], string][] = [
  [["rooms"], "c.tab.rooms"], [["queues"], "c.tab.queues"], [["activities"], "c.tab.activities"], [["ratings", "leaderboards"], "c.tab.ratings"],
  [["parties"], "c.tab.party"], [["friends"], "c.tab.friends"], [["stats"], "c.tab.stats"],
];

/** The server the Play page puts first: the one in use (its community page), else the selected one, else the first. */
export function heroServer(v: ServersView | null): ServerView | null {
  const list = v?.servers ?? [];
  return list.find((s) => s.id === v?.features.provider_id) ?? list.find((s) => s.selected) ?? list[0] ?? null;
}

export function useOnline(): OnlineView | null {
  const { servers } = useApp();
  const { lang, t } = useI18n();
  const live = useLive();
  const hero = heroServer(servers);
  const anyService = (servers?.servers ?? []).some((s) => !!s.api);
  const [peeks, setPeeks] = useState<Peek[] | null>(null);
  const loggedNow = live.overview?.session.logged_in;
  useEffect(() => {
    if (!anyService) { setPeeks([]); return; }
    let alive = true;
    void proto.peek().then((p) => { if (alive) setPeeks(p); }).catch(() => { if (alive) setPeeks([]); });
    return () => { alive = false; };
  }, [servers, anyService, loggedNow]);
  // The servers that declare their population: the main one by its live meta when the Community page has it, the
  // others by what this computer kept of theirs.
  const declares = (id: string) => ((live.server === id ? live.overview?.check?.features : null) ?? peeks?.find((p) => p.id === id)?.features ?? []).includes("population");
  const pop = usePopulation((servers?.servers ?? []).filter((x) => x.api && declares(x.id)).map((x) => x.id));
  if (!hero) return null;

  const service = !!hero.api;
  const peek = peeks?.find((p) => p.id === hero.id) ?? null;
  const here = live.server === hero.id ? live : null;
  const session = service ? here?.overview?.session ?? peek?.session ?? (peeks ? { logged_in: false, provider: null, user_id: null, display_name: null } : null) : null;
  const name = (here?.me?.display_name as string | undefined) || session?.display_name || null;
  const features: string[] = here?.overview?.check?.features ?? peek?.features ?? [];
  const pills = service
    ? PILLS.filter(([areas]) => areas.some((a) => features.includes(a))).slice(0, 4).map(([, key]) => lookup("community", lang, key))
    : [t("play.on.relay.p1"), t("play.on.relay.p3")];
  const description = service ? text(here?.overview?.meta?.description, lang) || text(peek?.description, lang) : t("play.online.relay");

  let now: OnlineLive | null = null;
  if (here && session?.logged_in) {
    const me = here.me?.id ?? session.user_id;
    const m = here.match;
    if (m && !["completed", "void", "cancelled"].includes(m.state)) {
      const mine = Array.isArray(m.participants) ? m.participants.find((p: J) => p?.user?.id === me) : null;
      now = m.state === "pending_accept" && mine?.accept === "pending" ? { kind: "found" } : { kind: "match" };
    } else if (here.room) {
      const code = typeof here.room.code === "string" && here.room.code.length <= 24 ? here.room.code : null;
      now = { kind: "room", code };
    } else if (here.ticket?.state === "match_found") {
      now = { kind: "found" };
    } else if (here.ticket?.state === "searching") {
      now = { kind: "search" };
    }
  }
  return { server: hero, service, description, pills, session, name, live: now, peeks: peeks ?? [], pop };
}

/** The other saved servers, as small cards: one click makes one the server in use (it becomes the main block). Refused,
 *  with its sentence, during a room, a search or a match. */
export function OtherServers({ online }: { online: OnlineView }) {
  const { t, lang } = useI18n();
  const { servers, fail } = useApp();
  const { blocked, choose } = useServerSwitch();
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<string | null>(null);
  useEffect(() => { if (!blocked) setRefused(null); }, [blocked]);
  const others = (servers?.servers ?? []).filter((s) => s.id !== online.server.id);
  if (!others.length) return null;
  const pick = async (id: string) => {
    if (blocked) { setRefused(blocked); return; }
    setBusy(true);
    try {
      await choose(id);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="srv-others" aria-label={t("play.on.others")}>
      <h2 className="srv-others-title">{t("play.on.others")}</h2>
      <div className="srv-cards">
        {others.map((s) => {
          const p = online.peeks.find((x) => x.id === s.id);
          const count = online.pop[s.id];
          const state = !s.api ? t("play.on.card.relay") : p?.session.logged_in ? t("play.on.card.in", { name: p.session.display_name || "—" }) : t("play.on.card.out");
          return (
            <button key={s.id} type="button" className="srv-card" disabled={busy} title={t("play.on.use", { name: s.name })} onClick={() => void pick(s.id)}>
              <span className={`srv-card-ico ${s.api ? "" : "relay"}`} aria-hidden="true">{s.api ? <IconFriends /> : <IconRelay />}</span>
              <span className="srv-card-text">
                <span className="srv-card-name">{s.name}</span>
                <span className="srv-card-state">
                  {count ? (
                    <><span className="pop-dot" />{populationText(count, lang, t, true)} · {state}</>
                  ) : (
                    <><span className={`srv-card-dot ${p?.session.logged_in ? "on" : ""}`} />{state}</>
                  )}
                </span>
              </span>
              <IconChevron className="srv-card-go" aria-hidden="true" />
            </button>
          );
        })}
      </div>
      {refused && <p className="srv-others-note" role="alert"><IconAlert /> {refused}</p>}
    </section>
  );
}

/** What this launcher is, in two sentences, with four points behind "Learn more" (a link after the text, or a small
 *  button under it). */
export function NativeNote({ button = false }: { button?: boolean }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const toggle = (
    <button type="button" className={button ? "btn btn-small btn-ghost native-more" : "link native-more"} aria-expanded={open} aria-controls="native-list" onClick={() => setOpen(!open)}>
      {open ? t("play.native.less") : t("play.native.more")}
    </button>
  );
  return (
    <div className="native">
      <p className="native-text">{t("play.native.text")}{!button && <>{" "}{toggle}</>}</p>
      {open && (
        <ul className="native-list" id="native-list">
          {(["p1", "p2", "p3", "p4"] as const).map((k) => <li key={k}>{t(`play.native.${k}`)}</li>)}
        </ul>
      )}
      {button && toggle}
    </div>
  );
}

/** With a server: the solo game, one click away under the online block. */
export function SoloCard({ disabled, modeOff, onLaunch }: { disabled: boolean; modeOff: (id: string) => boolean; onLaunch: (mode: "original" | "range" | "hub") => void }) {
  const { t } = useI18n();
  const btn = (mode: "original" | "range" | "hub", icon: ReactNode, label: string) => (
    <button type="button" className="btn btn-ghost" disabled={disabled || modeOff(mode)} onClick={() => onLaunch(mode)}>{icon} {label}</button>
  );
  return (
    <section className="solo" aria-labelledby="solo-title">
      <div className="solo-head">
        <div className="eyebrow">{t("play.solo.eyebrow")}</div>
        <h2 className="solo-title display" id="solo-title">{t("play.solo.title")}</h2>
      </div>
      <div className="solo-actions">
        {btn("original", <IconPlay />, t("play.solo.full"))}
        {btn("range", <IconTarget />, t("tile.range.title"))}
        {btn("hub", <IconCompass />, t("tile.hub.title"))}
      </div>
      <NativeNote />
    </section>
  );
}

/** Without a server: what this launcher is, and how online play comes (a server the player adds). */
export function OfflineExtras() {
  const { t } = useI18n();
  const { s } = useS();
  const { reloadServers } = useApp();
  const [adding, setAdding] = useState(false);
  return (
    <div className="play-more">
      <section className="more-card" aria-labelledby="native-title">
        <div className="eyebrow">{t("play.native.eyebrow")}</div>
        <h2 className="more-title" id="native-title">{t("play.native.title")}</h2>
        <NativeNote button />
      </section>
      <section className="more-card more-add" aria-labelledby="add-title">
        <div className="eyebrow">{t("play.add.eyebrow")}</div>
        <h2 className="more-title" id="add-title">{t("play.add.title")}</h2>
        <p className="more-text">{t("play.add.text")}</p>
        <button type="button" className="btn btn-small btn-ghost" onClick={() => setAdding(true)}><IconPlus /> {s("srv.add.cta")}</button>
      </section>
      {adding && createPortal(
        <Suspense fallback={null}>
          <AddServerModal onClose={() => setAdding(false)} onAdded={() => { setAdding(false); void reloadServers(); }} />
        </Suspense>,
        document.body,
      )}
    </div>
  );
}
