// The community servers: which one provides private matches (its relay), check its name, add or remove one.
import { useState } from "react";
import { serversApi, toAppError, type ServerView, type ServersView } from "../api";
import { useApp } from "../ctx";
import { IconPlus, IconRelay } from "../icons";
import { AddServerModal } from "./AddServer";
import { useS } from "./texts";

export const DEFAULT_RELAY_PORT = 47313;

/** `host` or `host:port`, as the match code screens show it. */
export function relayText(s: Pick<ServerView, "relay_host" | "relay_port">): string {
  if (!s.relay_host) return "";
  return s.relay_port === DEFAULT_RELAY_PORT ? s.relay_host : `${s.relay_host}:${s.relay_port}`;
}

export function selectedServer(v: ServersView | null): ServerView | null {
  const list = v?.servers ?? [];
  return list.find((s) => s.selected && s.relay_host) ?? list.find((s) => s.relay_host) ?? null;
}

export function ServerPicker() {
  const { s } = useS();
  const { servers, reloadServers } = useApp();
  const [adding, setAdding] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const list = servers?.servers ?? [];
  const sel = selectedServer(servers);
  const run = async (f: () => Promise<unknown>, ok?: (r: unknown) => string) => {
    setBusy(true);
    setMsg(null);
    try {
      const r = await f();
      await reloadServers();
      if (ok) setMsg({ ok: true, text: ok(r) });
    } catch (e) {
      setMsg({ ok: false, text: toAppError(e).message });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="server-field">
      {list.length > 0 ? (
        <>
          <label className="field-label" htmlFor="srv-pick">{s("srv.pick.label")}</label>
          <div className="row">
            <select id="srv-pick" className="grow" value={sel?.id ?? ""} onChange={(e) => void run(() => serversApi.select(e.target.value))}>
              {list.map((x) => <option key={x.id} value={x.id}>{x.name}{x.relay_host ? ` — ${relayText(x)}` : ""}</option>)}
            </select>
            <IconRelay width="1.2rem" height="1.2rem" />
          </div>
          {sel && (
            <p className="hint">
              {s("srv.relay")} : <span className="mono">{relayText(sel)}</span>
              {sel.last_ip && sel.last_ip !== sel.relay_host && <> · {s("srv.last", { ip: sel.last_ip })}</>}
              {sel.api && <> · {s("srv.api")} : <span className="mono">{sel.api}</span></>}
            </p>
          )}
          <div className="row wrap">
            {sel && <button className="btn btn-small btn-ghost" disabled={busy} onClick={() => void run(() => serversApi.resolve(sel.id), (r) => s("srv.check.ok", { host: (r as { host: string }).host, ip: (r as { ip: string }).ip }))}>{s("srv.check")}</button>}
            <button className="btn btn-small btn-ghost" disabled={busy} onClick={() => setAdding(true)}><IconPlus /> {s("srv.add.cta")}</button>
            {sel && <button className="btn btn-small btn-ghost" disabled={busy} onClick={() => void run(() => serversApi.remove(sel.id))}>{s("srv.remove")}</button>}
          </div>
        </>
      ) : (
        <button className="btn" onClick={() => setAdding(true)}><IconPlus /> {s("srv.add.cta")}</button>
      )}
      {msg && <p className={msg.ok ? "hint" : "bad-text"} role={msg.ok ? "status" : "alert"}>{msg.text}</p>}
      <p className="hint">{s("srv.privacy")}</p>
      {adding && <AddServerModal onClose={() => setAdding(false)} onAdded={() => { setAdding(false); void reloadServers(); }} />}
    </div>
  );
}
