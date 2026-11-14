// Character: style, skin tone, eye color, hair, eyebrows, legwear.
import { useMemo, type ReactNode } from "react";
import type { CategoryId, GameEntry, Profile } from "../../api";
import { GameIcon } from "../../grid";
import { byId, pickName } from "../../gamedata";
import { SKIN_APPROX, EYES_APPROX } from "./swatches";
import { FieldNote, type TabProps } from "./common";

type Field = "style" | "skin" | "eyes" | "hair" | "eyebrows" | "legwear";

function RadioGrid({ label, entries, value, onPick, cell, swatches }: { swatches?: boolean; label: string; entries: GameEntry[]; value: number | null; onPick: (id: number) => void; cell: (e: GameEntry, i: number) => ReactNode }) {
  const idx = Math.max(0, entries.findIndex((e) => e.id === value));
  return (
    <div className={`choice-grid ${swatches ? "choice-grid-sw" : ""}`} role="radiogroup" aria-label={label}>
      {entries.map((e, i) => (
        <button
          key={e.id}
          type="button"
          role="radio"
          aria-checked={e.id === value}
          tabIndex={i === idx ? 0 : -1}
          className={`choice ${swatches ? "choice-sw" : ""} ${e.id === value ? "choice-on" : ""}`}
          title={swatches ? `${label} ${e.id + 1}` : undefined}
          onClick={() => onPick(e.id)}
          onKeyDown={(ev) => {
            const d = ev.key === "ArrowRight" || ev.key === "ArrowDown" ? 1 : ev.key === "ArrowLeft" || ev.key === "ArrowUp" ? -1 : 0;
            if (!d) return;
            ev.preventDefault();
            const n = (i + d + entries.length) % entries.length;
            onPick(entries[n].id);
            (ev.currentTarget.parentElement?.children[n] as HTMLElement | undefined)?.focus();
          }}
        >
          {cell(e, i)}
        </button>
      ))}
    </div>
  );
}

export function Character({ cat, profile, setProfile, lang, t, game, notes, advanced }: TabProps) {
  const set = (f: Field) => (id: number) => setProfile((p: Profile) => ({ ...p, [f]: p[f] === id ? null : id, ...(f === "legwear" ? { legwear_variation: null } : {}) }));
  const list = (id: CategoryId) => (cat.categories[id] ?? []).filter((e) => e.selectable);
  const styleEntry = byId(cat.categories.style).get(profile.style ?? -1);
  const species = styleEntry?.species ?? null;
  const bySpecies = (id: CategoryId) => list(id).filter((e) => !species || !e.species || e.species === species);
  const styles = useMemo(() => list("style"), [cat]); // eslint-disable-line react-hooks/exhaustive-deps
  const speciesName = (s: string | null) => (s === "octoling" ? t("profile.species.octoling") : t("profile.species.inkling"));
  const styleLabel = (i: number) => `${t("profile.styleword")} ${i + 1}`;
  // Colors read from the player's game files; the hand-written table only when they are missing.
  const swatch = (table: string[]) => (e: GameEntry) => {
    const [c1, c2] = e.colors ?? [];
    const bg = c1 ? (c2 ? `radial-gradient(circle, ${c1} 0 50%, ${c2} 52%)` : c1) : (table[e.id] ?? "#888");
    return <span className="swatch" style={{ background: bg }} />;
  };
  const approxColours = (id: CategoryId) => list(id).some((e) => !e.colors?.length);
  const iconCell = (e: GameEntry, i: number) => (
    <>
      {e.icon ? <GameIcon icon={e.icon} className="choice-icon" /> : <span className="choice-num">{i + 1}</span>}
      {e.icon_approx && <span className="approx" title={t("profile.approx")}>~</span>}
    </>
  );

  return (
    <div className="character">
      <FieldNote note={notes.appearance} t={t} />
      <section className="pblock">
        <h3 className="pblock-title">{t("profile.cat.style")}</h3>
        <FieldNote note={notes.style} t={t} />
        <RadioGrid label={t("profile.cat.style")} entries={styles} value={profile.style} onPick={set("style")} cell={(e, i) => (
          <>
            <GameIcon icon={e.icon} className="choice-icon" />
            <span className="choice-label">{speciesName(e.species)} {(i % 2) + 1}</span>
          </>
        )} />
      </section>
      <section className="pblock">
        <h3 className="pblock-title">{t("profile.cat.skin")}</h3>
        <FieldNote note={notes.skin} t={t} />
        <RadioGrid label={t("profile.cat.skin")} entries={list("skin")} value={profile.skin} onPick={set("skin")} cell={swatch(SKIN_APPROX)} swatches />
        {approxColours("skin") && <p className="hint">{t("profile.nocolour")}</p>}
      </section>
      <section className="pblock">
        <h3 className="pblock-title">{t("profile.cat.eyes")}</h3>
        <FieldNote note={notes.eyes} t={t} />
        <RadioGrid label={t("profile.cat.eyes")} entries={list("eyes")} value={profile.eyes} onPick={set("eyes")} cell={swatch(EYES_APPROX)} swatches />
        {approxColours("eyes") && <p className="hint">{t("profile.nocolour")}</p>}
      </section>
      <section className="pblock">
        <h3 className="pblock-title">{t("profile.cat.hair")} {species && <span className="muted small">({speciesName(species)})</span>}</h3>
        <FieldNote note={notes.hair} t={t} />
        <RadioGrid label={t("profile.cat.hair")} entries={bySpecies("hair")} value={profile.hair} onPick={set("hair")} cell={(e, i) => (
          <>{iconCell(e, i)}<span className="choice-label tiny">{styleLabel(i)}</span></>
        )} />
        {game && advanced && <p className="hint">{t("profile.approx")}</p>}
      </section>
      <section className="pblock">
        <h3 className="pblock-title">{t("profile.cat.eyebrows")}</h3>
        <FieldNote note={notes.eyebrows} t={t} />
        <RadioGrid label={t("profile.cat.eyebrows")} entries={bySpecies("eyebrows")} value={profile.eyebrows} onPick={set("eyebrows")} cell={(e, i) => (
          <>{iconCell(e, i)}<span className="choice-label tiny">{styleLabel(i)}</span></>
        )} />
      </section>
      <section className="pblock">
        <h3 className="pblock-title">{t("profile.cat.legwear")}</h3>
        <FieldNote note={notes.legwear} t={t} />
        <FieldNote note={notes.legwear_variation} t={t} />
        <RadioGrid label={t("profile.cat.legwear")} entries={list("legwear")} value={profile.legwear} onPick={set("legwear")} cell={(e, i) => (
          <><span className="choice-label">{pickName(e.name, lang) || styleLabel(i)}</span></>
        )} />
        {game && advanced && <p className="hint">{t("profile.legwear.noicon")}</p>}
        {(() => {
          const n = byId(cat.categories.legwear).get(profile.legwear ?? -1)?.variations ?? 1;
          if (n <= 1) return null;
          const cur = Math.min(profile.legwear_variation ?? 0, n - 1);
          return (
            <>
              <h3>{t("profile.legwear.variation")}</h3>
              <div className="choice-grid" role="radiogroup" aria-label={t("profile.legwear.variation")}>
                {Array.from({ length: n }, (_, v) => (
                  <button key={v} type="button" role="radio" aria-checked={cur === v} className={`choice ${cur === v ? "choice-on" : ""}`}
                    onClick={() => setProfile((p: Profile) => ({ ...p, legwear_variation: v }))}><span className="choice-label">{t("profile.variantword")} {v + 1}</span></button>
                ))}
              </div>
            </>
          );
        })()}
      </section>
    </div>
  );
}
