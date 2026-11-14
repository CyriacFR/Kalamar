// Settings > Servers: the community servers added, which one is used, add or remove one. The advanced mode shows the
// relay and service addresses and the name check.
import { useState } from "react";
import { createPortal } from "react-dom";
import { serversApi, toAppError, type ServerView } from "../api";
import { proto } from "../community/api";
import { useApp } from "../ctx";
import { IconPlus } from "../icons";
import { AddServerModal } from "./AddServer";
import { relayText } from "./ServerPicker";
import { useServerSwitch } from "./switch";
import { useS } from "./texts";

export function ServerList() {
  const { s } = useS();
  const { servers, reloadServers, advanced } = useApp();
  const [adding, setAdding] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const { used, blocked, choose } = useServerSwitch();
  const list = servers?.servers ?? [];
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
  // A service keeps a session on this PC: removing it logs out and forgets the session (the account stays on the server).
  const remove = (x: ServerView) => {
    if (!window.confirm(s("srv.remove.confirm", { name: x.name }))) return;
    void run(() => (x.api ? proto.forget(x.id) : serversApi.remove(x.id)));
  };
  return (
    <div className="stack">
      {list.length === 0 && <p className="empty-line">{s("srv.none")}</p>}
      {list.length > 0 && (
        <ul className="srv-list">
          {list.map((x) => {
            const on = x.id === used;
            return (
              <li key={x.id} className={`srv ${on ? "srv-on" : ""}`}>
                <div className="srv-main">
                  <div className="grow">
                    <p className="srv-name">{x.name}{on && <span className="srv-used">{s("srv.used")}</span>}</p>
                    <p className="small muted">{x.api ? s("srv.kind.service") : s("srv.kind.relay")}</p>
                  </div>
                  <div className="row wrap">
                    {!on && <button className="btn btn-small" disabled={busy} onClick={() => (blocked ? setMsg({ ok: false, text: blocked }) : void run(() => choose(x.id)))}>{s("srv.use")}</button>}
                    <button className="btn btn-small btn-ghost" disabled={busy} onClick={() => remove(x)}>{s("srv.remove")}</button>
                  </div>
                </div>
                {x.api && on && (
                  <label className="check srv-bg">
                    <input type="checkbox" checked={!!x.background} onChange={(e) => void run(() => proto.setBackground(x.id, e.target.checked))} />
                    <span>{s("srv.background")}</span>
                  </label>
                )}
                {advanced && (
                  <p className="tiny muted srv-tech">
                    {x.relay_host && <>{s("srv.relay")} : <span className="mono">{relayText(x)}</span></>}
                    {x.last_ip && x.last_ip !== x.relay_host && <> · {s("srv.last", { ip: x.last_ip })}</>}
                    {x.api && <> · {s("srv.api")} : <span className="mono wrapany">{x.api}</span></>}
                    {x.relay_host && <> · <button className="link" disabled={busy} onClick={() => void run(() => serversApi.resolve(x.id), (r) => s("srv.check.ok", { host: (r as { host: string }).host, ip: (r as { ip: string }).ip }))}>{s("srv.check")}</button></>}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      )}
      <div className="row">
        <button className={`btn ${list.length ? "btn-ghost" : ""}`} onClick={() => setAdding(true)}><IconPlus /> {s("srv.add.cta")}</button>
      </div>
      {msg && <p className={msg.ok ? "hint" : "bad-text"} role={msg.ok ? "status" : "alert"}>{msg.text}</p>}
      <p className="hint">{s("srv.privacy")}</p>
      {adding && createPortal(<AddServerModal onClose={() => setAdding(false)} onAdded={() => { setAdding(false); void reloadServers(); }} />, document.body)}
    </div>
  );
}
