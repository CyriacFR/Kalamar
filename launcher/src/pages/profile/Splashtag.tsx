// Splashtag: banner, title (adjective + subject) and up to three badges, with a live preview.
import { useMemo, useState } from "react";
import type { GameCatalogue, GameEntry, Profile } from "../../api";
import { GameIcon, VirtualGrid } from "../../grid";
import { byId, entryName, normalise, titleText } from "../../gamedata";
import type { TFn } from "../../i18n";
import { FieldNote, FilterBar, Segmented, SearchBox, Select, type TabProps } from "./common";

/** The tag drawn by the launcher: banner image behind the text, title above the name, badges on the right. */
export function TagPreview({ cat, profile, lang, t, compact }: { cat: GameCatalogue; profile: Profile; lang: string; t: TFn; compact?: boolean }) {
  const banner = byId(cat.categories.banner).get(profile.banner ?? -1);
  const adj = byId(cat.categories.adjective).get(profile.title.adjective ?? -1);
  const sub = byId(cat.categories.subject).get(profile.title.subject ?? -1);
  const badges = byId(cat.categories.badge);
  const title = titleText(lang, adj ? entryName(adj, lang) : "", sub ? entryName(sub, lang) : "");
  return (
    <div className={`stag ${compact ? "stag-compact" : ""}`} role="img" aria-label={`${profile.name} - ${title}`}>
      {banner?.icon ? <GameIcon icon={banner.icon} className="stag-bg" /> : <span className="stag-bg stag-bg-none" />}
      <span className="stag-scrim" />
      <span className="stag-title">{title || t("profile.splash.notitle")}</span>
      <span className="stag-name">{profile.name || t("profile.name.placeholder")}</span>
      <span className="stag-badges">
        {profile.badges.map((id, i) => {
          const b = id != null ? badges.get(id) : undefined;
          return <span key={i} className={`stag-badge ${b ? "" : "stag-badge-empty"}`} title={b ? entryName(b, lang) : ""}>{b && <GameIcon icon={b.icon} className="stag-badge-img" />}</span>;
        })}
      </span>
    </div>
  );
}

type Part = "banner" | "title" | "badges";

export function Splashtag({ cat, profile, setProfile, lang, t, notes, advanced }: TabProps) {
  const [part, setPart] = useState<Part>("banner");
  const sub = { cat, profile, setProfile, lang, t, game: true, notes, advanced };
  return (
    <div className="tab-body">
      <FieldNote note={notes.splashtag} t={t} />
      <FieldNote note={notes.badges} t={t} />
      <FilterBar>
        <Segmented<Part> value={part} onChange={setPart} label={t("profile.tab.splashtag")}
          options={[{ id: "banner", label: t("profile.splash.banner") }, { id: "title", label: t("profile.splash.title") }, { id: "badges", label: t("profile.splash.badges") }]} />
      </FilterBar>
      {part === "banner" && <Banners {...sub} />}
      {part === "title" && <Titles {...sub} />}
      {part === "badges" && <Badges {...sub} />}
    </div>
  );
}

function seriesLabel(c: string, t: TFn): string {
  // Npl_Catalog_Season01 -> "Catalog - season 1"
  const m = /^Npl_(\w+?)_Season(\d+)$/.exec(c);
  if (m) return `${m[1]} · ${t("profile.season")} ${Number(m[2])}`;
  return c.replace(/^Npl_?/, "") || "Npl";
}

function Banners({ cat, profile, setProfile, t, advanced }: TabProps) {
  const [series, setSeries] = useState("");
  const all = useMemo(() => (cat.categories.banner ?? []).filter((e) => e.selectable), [cat]);
  const seriesList = useMemo(() => [...new Set(all.map((e) => e.class ?? ""))].sort(), [all]);
  const items = useMemo(() => all.filter((e) => !series || e.class === series), [all, series]);
  return (
    <>
      <FilterBar>
        <Select label={t("profile.filter.series")} value={series} onChange={setSeries} options={[{ id: "", label: t("profile.filter.all") }, ...seriesList.map((s) => ({ id: s, label: seriesLabel(s, t) }))]} />
        <span className="small muted">{t("profile.count", { n: items.length, total: all.length })}</span>
        {profile.banner != null && <button className="btn btn-small btn-ghost" onClick={() => setProfile((p) => ({ ...p, banner: null }))}>{t("profile.clear")}</button>}
      </FilterBar>
      <VirtualGrid
        className="vg-fill"
        items={items}
        keyOf={(e) => e.id}
        label={t("profile.splash.banner")}
        cellWidth={216}
        cellHeight={96}
        isSelected={(e) => e.id === profile.banner}
        onPick={(e) => setProfile((p) => ({ ...p, banner: p.banner === e.id ? null : e.id }))}
        empty={t("profile.empty")}
        render={(e) => (
          <>
            <GameIcon icon={e.icon} className="banner-icon" />
            <span className="cell-sub tiny muted">{seriesLabel(e.class ?? "", t)}{advanced ? ` · ${e.row.slice(-4)}` : ""}</span>
          </>
        )}
      />
    </>
  );
}

function WordList({ list, value, onPick, label, lang, t, advanced }: { list: GameEntry[]; value: number | null; onPick: (id: number | null) => void; label: string; lang: string; t: TFn; advanced: boolean }) {
  const [q, setQ] = useState("");
  const items = useMemo(() => {
    const nq = normalise(q.trim());
    return list.filter((e) => !nq || normalise(entryName(e, lang)).includes(nq) || String(e.id) === nq);
  }, [list, q, lang]);
  return (
    <div className="wordlist">
      <SearchBox value={q} onChange={setQ} label={label} />
      <div className="small muted">{t("profile.count", { n: items.length, total: list.length })}</div>
      <VirtualGrid
        className="vg-words"
        items={items}
        keyOf={(e) => e.id}
        label={label}
        fill
        cellWidth={300}
        cellHeight={34}
        gap={4}
        isSelected={(e) => e.id === value}
        onPick={(e) => onPick(e.id === value ? null : e.id)}
        empty={t("profile.empty")}
        render={(e) => <span className="word">{entryName(e, lang)}{advanced && <span className="tiny muted mono"> #{e.id}</span>}</span>}
      />
    </div>
  );
}

function Titles({ cat, profile, setProfile, lang, t, advanced }: TabProps) {
  const adjectives = useMemo(() => (cat.categories.adjective ?? []).filter((e) => e.selectable).sort((a, b) => entryName(a, lang).localeCompare(entryName(b, lang))), [cat, lang]);
  const subjects = useMemo(() => (cat.categories.subject ?? []).filter((e) => e.selectable).sort((a, b) => entryName(a, lang).localeCompare(entryName(b, lang))), [cat, lang]);
  return (
    <div className="two-cols">
      <WordList list={adjectives} value={profile.title.adjective} onPick={(id) => setProfile((p) => ({ ...p, title: { ...p.title, adjective: id } }))} label={t("profile.splash.adjective")} lang={lang} t={t} advanced={advanced} />
      <WordList list={subjects} value={profile.title.subject} onPick={(id) => setProfile((p) => ({ ...p, title: { ...p.title, subject: id } }))} label={t("profile.splash.subject")} lang={lang} t={t} advanced={advanced} />
    </div>
  );
}

function Badges({ cat, profile, setProfile, lang, t }: TabProps) {
  const [slot, setSlot] = useState(0);
  const [q, setQ] = useState("");
  const [kind, setKind] = useState("");
  const all = useMemo(() => cat.categories.badge ?? [], [cat]);
  const kinds = useMemo(() => [...new Set(all.map((e) => e.class ?? ""))].sort(), [all]);
  const items = useMemo(() => {
    const nq = normalise(q.trim());
    return all.filter((e) => (!kind || e.class === kind) && (!nq || normalise(`${entryName(e, lang)} ${e.row}`).includes(nq)));
  }, [all, q, kind, lang]);
  const badges = byId(cat.categories.badge);
  const pick = (e: GameEntry) =>
    e.selectable && setProfile((p) => {
      const b = [...p.badges];
      const existing = b.indexOf(e.id);
      if (existing >= 0) b[existing] = null;
      else b[slot] = e.id;
      return { ...p, badges: b };
    });
  const used = new Set(profile.badges.filter((b): b is number => b != null));
  return (
    <>
      <p className="notice notice-info">{t("profile.badges.note")}</p>
      <div className="slots row wrap" role="radiogroup" aria-label={t("profile.splash.slots")}>
        {profile.badges.map((id, i) => {
          const b = id != null ? badges.get(id) : undefined;
          return (
            <div key={i} className={`slot ${slot === i ? "slot-on" : ""}`}>
              <button type="button" role="radio" aria-checked={slot === i} className="slot-btn" onClick={() => setSlot(i)}>
                {b ? <GameIcon icon={b.icon} className="slot-img" /> : <span className="slot-empty">{i + 1}</span>}
              </button>
              <span className="tiny">{b ? entryName(b, lang) : t("profile.splash.slot", { n: i + 1 })}</span>
              {b && <button type="button" className="btn btn-small btn-ghost" onClick={() => setProfile((p) => ({ ...p, badges: p.badges.map((x, j) => (j === i ? null : x)) }))}>{t("profile.clear")}</button>}
            </div>
          );
        })}
      </div>
      <FilterBar>
        <SearchBox value={q} onChange={setQ} label={t("profile.search")} />
        <Select label={t("profile.filter.category")} value={kind} onChange={setKind} options={[{ id: "", label: t("profile.filter.all") }, ...kinds.map((k) => ({ id: k, label: k }))]} />
        <span className="small muted">{t("profile.count", { n: items.length, total: all.length })}</span>
      </FilterBar>
      <VirtualGrid
        className="vg-fill"
        items={items}
        keyOf={(e) => e.id}
        label={t("profile.splash.badges")}
        cellWidth={150}
        cellHeight={150}
        isSelected={(e) => used.has(e.id)}
        onPick={pick}
        empty={t("profile.empty")}
        render={(e) => (
          <>
            <GameIcon icon={e.icon} className="badge-icon" />
            <span className="cell-name cell-name-3" title={entryName(e, lang)}>{entryName(e, lang)}</span>
            {e.unavailable && <span className="tiny bad-text" title={t(`profile.unavail.${e.unavailable}` as "profile.unavail.badge_record")}>{t("profile.unavail.badge_record").split(":")[0]}</span>}
          </>
        )}
      />
    </>
  );
}
