// The community page of the selected server (it has a service): login, then one tab per area the server declares, in
// order of importance (rooms, queues, then the rest). Nothing a server does not declare is shown.
import { useEffect, useState } from "react";
import { useApp } from "../ctx";
import { Page } from "../ui";
import { proto } from "./api";
import { activate, has, reload, setLive, useLive } from "./store";
import { failText, useC, type CKey } from "./texts";
import { Login } from "./Login";
import { Rooms } from "./Rooms";
import { Queues } from "./Queues";
import { MatchCard } from "./Match";
import { Friends, Party } from "./Social";
import { Account, Profile, Ratings, Stats } from "./Standing";
import { Activities } from "./Activities";
import { Watch } from "./Stands";
import { SeriesCard } from "./view";
import { IconAlert, IconRefresh } from "../icons";
import { useI18n } from "../i18n";
import { ServerSwitch } from "../servers/ServerMenu";
import { populationText, usePopulation } from "./population";
import "./community.css";

type Tab = "rooms" | "queues" | "activities" | "watch" | "party" | "friends" | "ratings" | "stats" | "profile" | "account";
const AREA: Record<Tab, string | null> = { rooms: "rooms", queues: "queues", activities: "activities", watch: "spectators", party: "parties", friends: "friends", ratings: "ratings", stats: "stats", profile: "profiles", account: null };
const ORDER: Tab[] = ["rooms", "queues", "activities", "watch", "party", "friends", "ratings", "stats", "profile", "account"];

function StreamChip() {
  const { c } = useC();
  const live = useLive();
  const s = live.stream;
  if (!s) return null;
  const text = s.state === "retrying" ? c("c.stream.retrying", { s: s.retry_in_s ?? "?" }) : c(`c.stream.${s.state}` as CKey);
  const conn = live.server ? live.server : null;
  return (
    <span className={`cm-stream cm-stream-${s.state}`} title={s.reason ?? ""}>
      <span className="status-dot" /> {text}
      {s.state === "stopped" && conn && live.overview?.session.logged_in && (
        <button className="link small" onClick={() => void activate(conn)}>{c("c.reconnect")}</button>
      )}
    </span>
  );
}

export default function CommunityPage({ server }: { server: string }) {
  const { c, L } = useC();
  const { reloadServers, go, toast, pageArg, servers } = useApp();
  const { t, lang } = useI18n();
  const live = useLive();
  const [tab, setTab] = useState<Tab>(() => (ORDER.includes(pageArg as Tab) ? (pageArg as Tab) : "rooms"));
  const mine = live.server === server;
  useEffect(() => { if (!mine) void activate(server); }, [server, mine]);
  const o = live.server === server ? live.overview : null;
  const name = o?.server.name ?? "";
  const meta = o?.meta;
  const tabs = ORDER.filter((t) => AREA[t] === null || has(AREA[t] as string));
  // How many players the server has, when it declares it (every 30 s while this page is on screen).
  const pop = usePopulation(o?.check?.features?.includes("population") ? [server] : [])[server] ?? null;
  const savedName = servers?.servers.find((x) => x.id === server)?.name ?? "";
  // Once the server's areas are known, the first declared one is shown (the tab is kept when it still exists).
  const known = !!o?.check;
  useEffect(() => { if (known && tabs.length && !tabs.includes(tab)) setTab(tabs[0]); }, [known, tabs.join(","), tab]); // eslint-disable-line react-hooks/exhaustive-deps
  const forget = async () => {
    if (!window.confirm(c("acct.forget.confirm", { name }))) return;
    try {
      await proto.forget(server);
      setLive({ server: null, overview: null });
      await reloadServers();
      go("play");
    } catch (e) {
      toast({ kind: "error", text: String((e as { message?: string }).message ?? e) });
    }
  };

  let body;
  if (!o) {
    body = live.error ? (
      <div className="state state-bad"><IconAlert /><h3>{c("c.unreachable")}</h3><p>{failText(live.error, c)}</p><button className="btn" onClick={() => void activate(server)}>{c("c.retry")}</button></div>
    ) : (
      <div className="state" aria-busy="true"><span className="skeleton" style={{ width: "12rem", height: "1.4rem" }} /><p>{c("c.loading")}</p></div>
    );
  } else if (!meta) {
    body = (
      <div className="state state-bad"><IconAlert /><h3>{c("c.unreachable")}</h3><p>{o.meta_error ? failText(o.meta_error, c) : c("c.unreachable.hint")}</p><p className="hint">{c("c.unreachable.hint")}</p><button className="btn" onClick={() => void activate(server)}><IconRefresh /> {c("c.retry")}</button></div>
    );
  } else if (o.check && !o.check.compat.ok) {
    body = <div className="state state-bad"><IconAlert /><h3>{c("c.incompatible")}</h3><p>{c(`c.incompatible.${o.check.compat.reason ?? "invalid"}` as CKey)}</p></div>;
  } else if (!o.session.logged_in) {
    body = (
      <>
        {live.error && live.error.code !== "network" && <p className="notice notice-info">{failText(live.error, c)}</p>}
        <Login server={server} name={name} meta={meta} nameMax={o.check?.display_name_max ?? 10} />
      </>
    );
  } else {
    const showMatch = live.match && !(tab === "rooms" && live.room);
    body = (
      <>
        {live.stream?.state === "stopped" && live.stream.reason === "replaced" && <p className="notice notice-info">{c("c.stream.replaced")}</p>}
        {live.stream?.state === "stopped" && live.stream.reason === "banned" && <p className="notice notice-error">{c("c.stream.banned")}</p>}
        {showMatch && <MatchCard match={live.match} />}
        {live.match?.series?.id && has("series") && <SeriesCard id={String(live.match.series.id)} />}
        {!live.match && live.ended && (
          <div className="cm-ended">
            <MatchCard match={live.ended} />
            <button className="btn btn-small btn-ghost" onClick={() => setLive({ ended: null })}>{c("m.dismiss")}</button>
          </div>
        )}
        <div className="tabs cm-tabs" role="tablist" aria-label={c("c.title")}>
          {tabs.map((t) => (
            <button key={t} role="tab" data-tab={t} aria-selected={tab === t} className={`tab ${tab === t ? "on" : ""}`} onClick={() => setTab(t)}>
              {c(`c.tab.${t}` as CKey)}
              {t === "queues" && live.ticket && <span className="cm-dot" aria-hidden="true" />}
              {t === "rooms" && (live.room || live.roomInvites.length > 0) && <span className="cm-dot" aria-hidden="true" />}
              {t === "party" && live.partyInvites.length > 0 && <span className="cm-dot" aria-hidden="true" />}
            </button>
          ))}
        </div>
        <div className="tabpanel" role="tabpanel">
          {tabs.length === 1 && <p className="muted">{c("c.nothing")}</p>}
          {tab === "rooms" && <Rooms />}
          {tab === "queues" && <Queues />}
          {tab === "party" && <Party />}
          {tab === "friends" && <Friends />}
          {tab === "ratings" && <Ratings />}
          {tab === "stats" && <Stats />}
          {tab === "activities" && <Activities />}
          {tab === "watch" && <Watch />}
          {tab === "profile" && <Profile />}
          {tab === "account" && <Account onForget={() => void forget()} />}
        </div>
      </>
    );
  }
  return (
    <Page
      title={
        <>
          <ServerSwitch name={name || savedName || c("c.title")} />
          {pop && <span className="hero-pop cm-pop"><span className="pop-dot" />{populationText(pop, lang, t)}</span>}
        </>
      }
      eyebrow={c("c.title")}
      subtitle={L(meta?.description) || (name ? c("c.sub", { name }) : undefined)}
      wide
      actions={
        <div className="cm-head">
          <StreamChip />
          {o?.session.logged_in && <span className="cm-me-name">{live.me?.display_name ?? o.session.display_name}</span>}
          <button className="icon-btn" title={c("c.retry")} aria-label={c("c.retry")} onClick={() => void reload(true).then(() => activate(server))}><IconRefresh /></button>
        </div>
      }
    >
      {body}
    </Page>
  );
}
