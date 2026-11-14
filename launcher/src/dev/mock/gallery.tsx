// Browser preview only (`?gallery=stages`): every look of the stage cards on the made-up stages, including the states
// no screen uses yet (banned), so that they can be captured and checked. Mounted over the app by ./core.ts.
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { I18nContext, isLang, makeT, type Lang } from "../../i18n";
import { StageCard, StagePicker, StageStrip, StageTag } from "../../stages";

const IDS = ["Vss_Docks00", "Vss_Park", "Vss_Market01", "Vss_GlassTower", "Vss_Garden02", "Vss_Quarry", "Vss_LiftBridge00", "Vss_MistStation", "Vss_Lab00", "Vss_Mystery02"];

function Gallery() {
  const [pick, setPick] = useState<string | null>("Vss_Market01");
  const [banned, setBanned] = useState<string[]>(["Vss_Park", "Vss_Quarry"]);
  return (
    <div style={{ padding: "1.5rem", display: "flex", flexDirection: "column", gap: "1.4rem", maxWidth: "70rem" }}>
      <h1 className="display page-title" style={{ fontSize: "1.8rem" }}>StageCard · StagePicker</h1>
      <section className="card stack">
        <h2>sizes · states</h2>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(12rem, 1fr))", gap: "1rem", alignItems: "end" }}>
          <StageCard id="Vss_Docks00" size="lg" />
          <StageCard id="Vss_Docks00" />
          <div style={{ width: "9rem" }}><StageCard id="Vss_Docks00" size="sm" /></div>
          <StageCard id="Vss_Garden02" mark="selected" onClick={() => {}} />
          <StageCard id="Vss_Park" mark="banned" />
          <StageCard id="Vss_Quarry" mark="unavailable" note="Not in this mode" />
          <StageCard id={null} />
          <StageCard id={null} mark="selected" />
          <StageCard id="Vss_Lab00" />
          <StageCard id="Vss_Mystery02" />
          <StageCard id="Vss_Mystery02" mark="banned" />
        </div>
      </section>
      <section className="card stack">
        <h2>picker · choose</h2>
        <StagePicker ids={IDS} value={pick} onPick={setPick} nullable label="choose" unavailable={{ Vss_LiftBridge00: "Not in this mode" }} />
      </section>
      <section className="card stack">
        <h2>picker · ban (compact)</h2>
        <StagePicker ids={IDS} compact banned={banned} onBan={(id, on) => setBanned((b) => (on ? [...b, id] : b.filter((x) => x !== id)))} label="ban" unavailable={{ Vss_Mystery02: "" }} />
      </section>
      <section className="card stack">
        <h2>tag · strip</h2>
        <p>Text <StageTag id="Vss_MistStation" /> · <StageTag id="Vss_Lab00" /> · <StageTag id={null} /></p>
        <StageStrip ids={["Vss_Docks00", "Vss_Park", "Vss_Market01", "Vss_GlassTower", "Vss_Garden02", null]} />
      </section>
    </div>
  );
}

export function mount(): void {
  const q = new URLSearchParams(location.search);
  const lang: Lang = isLang(q.get("lang")) ? (q.get("lang") as Lang) : "en";
  const host = document.createElement("div");
  host.style.cssText = "position:fixed;inset:0;z-index:500;overflow:auto;background:var(--bg)";
  document.body.appendChild(host);
  createRoot(host).render(
    <I18nContext.Provider value={{ lang, t: makeT(lang) }}>
      <Gallery />
    </I18nContext.Provider>,
  );
}
