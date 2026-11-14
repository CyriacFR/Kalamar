// "Add a community server": a host name (preferred) or an IPv4 address with an optional port, or a server card file.
// A typed name is looked up as the community protocol says (its discovery document, then its service's description);
// the player sees what the server is and offers before it is saved. When no service answers, the line can still be
// added as a relay only (private matches by code), as before the protocol existed.
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { serversApi, toAppError, type CardPreview, type ServersView } from "../api";
import { proto, toFail, type Discovered, type Preview } from "../community/api";
import { pick, useI18n } from "../i18n";
import { Modal } from "../ui";
import { IconFolder } from "../icons";
import { useApp } from "../ctx";
import { useS } from "./texts";

const AREAS = ["rooms", "queues", "parties", "friends", "presence", "ratings", "leaderboards", "stats", "moderation", "activities", "series", "population"];

/** What a discovered server is and offers, before it is saved. */
function ServicePreview({ p }: { p: Preview }) {
  const { s } = useS();
  const { lang } = useI18n();
  const { advanced } = useApp();
  return (
    <div className="card stack">
      <p><strong>{p.name}</strong></p>
      {pick(p.description, lang) && <p className="small">{pick(p.description, lang)}</p>}
      <p className="small muted">
        {p.operator_name && <span>{s("srv.found.operator", { name: p.operator_name })}{p.operator_contact ? ` (${p.operator_contact})` : ""}. </span>}
        {p.region && <span>{s("srv.found.region", { region: p.region })}. </span>}
        {p.languages.length > 0 && <span>{s("srv.found.langs", { langs: p.languages.join(", ") })}.</span>}
      </p>
      {p.compat.ok ? (
        <p className="small"><strong>{s("srv.found.features")} :</strong> {p.features.filter((f) => AREAS.includes(f)).map((f) => s(`feat.${f}`)).join(", ")}.</p>
      ) : (
        <p className="bad-text">{s("srv.found.incompatible", { reason: p.compat.reason ?? "?" })}</p>
      )}
      {pick(p.rules_summary, lang) && <p className="small">{pick(p.rules_summary, lang)}</p>}
      {advanced && p.typed_host && <p className="hint">{s("srv.found.api", { host: p.api_host })}</p>}
      {p.features_mismatch && <p className="hint">{s("srv.found.mismatch")}</p>}
      {p.insecure_loopback && <p className="hint warn-text">{s("srv.found.loopback")}</p>}
      {advanced && p.relay && <p className="mono small">{s("srv.relay")} : {p.relay}</p>}
      <p className="mono tiny wrapany">
        {p.privacy_url && <span>{s("srv.found.privacy", { url: p.privacy_url })} </span>}
        {p.rules_url && <span>· {s("srv.found.rules", { url: p.rules_url })}</span>}
      </p>
    </div>
  );
}

export function AddServerModal({ onClose, onAdded }: { onClose: () => void; onAdded: (v: ServersView) => void }) {
  const { s } = useS();
  const { t } = useI18n();
  const { advanced, config } = useApp();
  const discordAvailable = !!config?.discord_available;
  const [tab, setTab] = useState<"line" | "card">("line");
  const [line, setLine] = useState("");
  const [name, setName] = useState("");
  const [apiUrl, setApiUrl] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [card, setCard] = useState<{ path: string; preview: CardPreview } | null>(null);
  const [found, setFound] = useState<Discovered | null>(null);
  const [busy, setBusy] = useState(false);

  const lineError = (e: unknown) => {
    const f = toFail(e);
    if (f.kind === "validation" && f.code && s(`srv.err.${f.code}`) !== `srv.err.${f.code}`) return s(`srv.err.${f.code}`);
    const a = toAppError(e);
    return a.kind === "validation" && a.fields.length ? a.fields.map((x) => s(`srv.err.${x.message}`)).join(" ") : f.message || a.message;
  };
  const wrap = async (f: () => Promise<void>) => {
    setBusy(true);
    setErr(null);
    try {
      await f();
    } catch (e) {
      setErr(lineError(e));
    } finally {
      setBusy(false);
    }
  };
  // A typed service address (advanced) is checked directly; otherwise the host is looked up.
  const find = () => void wrap(async () => {
    setFound(null);
    if (apiUrl.trim()) {
      // The relay comes from the service itself (its meta), never from the typed line (protocol 1.1.1).
      const id = await proto.add({ name: name.trim() || line.trim(), api: apiUrl.trim(), relay: null });
      if (id) onAdded(await serversApi.view());
      return;
    }
    setFound(await proto.discover(line));
  });
  const addService = (p: Preview) => void wrap(async () => {
    await proto.add({ name: name.trim() || p.name, api: p.api, relay: p.relay });
    onAdded(await serversApi.view());
  });
  const addRelayOnly = (l: string) => void wrap(async () => onAdded(await serversApi.addLine(l, name.trim() || null, null)));
  const pickCard = async () => {
    setErr(null);
    setFound(null);
    setCard(null);
    const p = await open({ multiple: false, directory: false, filters: [{ name: s("srv.card.filter"), extensions: ["json"] }] });
    if (typeof p !== "string") return;
    await wrap(async () => {
      const d = await proto.card(p);
      if (d.kind === "legacy") setCard({ path: p, preview: await serversApi.cardPreview(p) });
      else setFound(d);
    });
  };
  const importCard = async () => {
    if (!card) return;
    await wrap(async () => {
      onAdded(await serversApi.cardImport(card.path));
    });
  };

  const result = found && (
    found.kind === "service" ? (
      <ServicePreview p={found.preview} />
    ) : found.kind === "relay_only" ? (
      <div className="card stack">
        <p><strong>{s("srv.relayonly.title")}</strong></p>
        <p className="small">{s("srv.relayonly.text")}</p>
        {advanced && found.reason && <p className="mono tiny wrapany muted">{found.reason}</p>}
        {advanced && found.problems.length > 0 && <ul className="tiny muted">{found.problems.map((x) => <li key={x}>{x}</li>)}</ul>}
      </div>
    ) : null
  );

  return (
    <Modal title={s("srv.add.title")} onClose={onClose}>
      <div className="seg" role="tablist">
        <button role="tab" aria-selected={tab === "line"} className={`seg-btn ${tab === "line" ? "on" : ""}`} onClick={() => { setTab("line"); setFound(null); }}>{s("srv.tab.line")}</button>
        <button role="tab" aria-selected={tab === "card"} className={`seg-btn ${tab === "card" ? "on" : ""}`} onClick={() => { setTab("card"); setFound(null); }}>{s("srv.tab.card")}</button>
      </div>
      {tab === "line" ? (
        <div className="stack">
          <label className="field-label" htmlFor="srv-line">{s("srv.line.label")}</label>
          <input id="srv-line" className="mono" data-autofocus value={line} placeholder="community.example.org" onChange={(e) => { setLine(e.target.value); setFound(null); }}
            onKeyDown={(e) => e.key === "Enter" && line.trim() && !busy && find()} />
          <p className="hint">{s("srv.line.hint")}</p>
          <label className="field-label" htmlFor="srv-name">{s("srv.name.label")}</label>
          <input id="srv-name" value={name} maxLength={40} placeholder={s("srv.name.placeholder")} onChange={(e) => setName(e.target.value)} />
          {advanced && <details className="advanced">
            <summary>{t("common.advanced")}</summary>
            <label className="field-label" htmlFor="srv-api">{s("srv.api.label")}</label>
            <input id="srv-api" className="mono" value={apiUrl} placeholder="https://community.example.org/api" onChange={(e) => setApiUrl(e.target.value)} />
            <p className="hint">{s("srv.api.hint")}</p>
          </details>}
        </div>
      ) : (
        <div className="stack">
          <p className="hint">{s("srv.card.hint")}</p>
          <button className="btn btn-alt" disabled={busy} onClick={() => void pickCard()}><IconFolder /> {s("srv.card.pick")}</button>
          {card && (
            <div className="card">
              <p><strong>{card.preview.name}</strong></p>
              {card.preview.relay && <p className="mono small">{s("srv.relay")} : {card.preview.relay}{card.preview.relay_port ? `:${card.preview.relay_port}` : ""}</p>}
              {card.preview.api && <p className="mono small">{s("srv.api")} : {card.preview.api}</p>}
            </div>
          )}
        </div>
      )}
      {result}
      <p className="hint">{s("srv.privacy")}</p>
      {discordAvailable && <p className="hint">{s("srv.privacy.discord")}</p>}
      {err && <p className="bad-text" role="alert">{err}</p>}
      <div className="row" style={{ justifyContent: "flex-end" }}>
        <button className="btn btn-ghost" onClick={onClose}>{s("srv.cancel")}</button>
        {found?.kind === "service" && <button className="btn" disabled={busy || !found.preview.compat.ok} onClick={() => addService(found.preview)}>{s("srv.found.add")}</button>}
        {found?.kind === "relay_only" && found.line && <button className="btn btn-alt" disabled={busy} onClick={() => addRelayOnly(found.line)}>{s("srv.relayonly.add")}</button>}
        {!found && tab === "line" && <button className="btn" disabled={busy || (!line.trim() && !apiUrl.trim())} onClick={find}>{busy ? s("srv.find") + "…" : apiUrl.trim() ? s("srv.add.do") : s("srv.find")}</button>}
        {!found && tab === "card" && <button className="btn" disabled={busy || !card} onClick={() => void importCard()}>{s("srv.add.do")}</button>}
      </div>
    </Modal>
  );
}
