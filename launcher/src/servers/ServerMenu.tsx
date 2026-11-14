// The server's name at the top of the Community page, as a button: the list of servers (one click makes one the server
// in use), "Add a server" and "Manage" (Settings > Servers). Refused, with its sentence, during a room, a search or a
// match.
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useApp } from "../ctx";
import { useI18n } from "../i18n";
import { proto, type Peek } from "../community/api";
import { IconAlert, IconCheck, IconChevron, IconFriends, IconPlus, IconRelay, IconSettings } from "../icons";
import { AddServerModal } from "./AddServer";
import { useServerSwitch } from "./switch";
import { useS } from "./texts";

export function ServerSwitch({ name }: { name: string }) {
  const { s } = useS();
  const { t } = useI18n();
  const { servers, go, reloadServers, fail } = useApp();
  const { used, blocked, choose } = useServerSwitch();
  const [open, setOpen] = useState(false);
  const [adding, setAdding] = useState(false);
  const [busy, setBusy] = useState(false);
  const [peeks, setPeeks] = useState<Peek[]>([]);
  const box = useRef<HTMLSpanElement>(null);
  const list = servers?.servers ?? [];

  useEffect(() => {
    if (!open) return;
    void proto.peek().then(setPeeks).catch(() => {});
    const away = (e: MouseEvent) => { if (box.current && !box.current.contains(e.target as Node)) setOpen(false); };
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", away);
    document.addEventListener("keydown", esc);
    return () => { document.removeEventListener("mousedown", away); document.removeEventListener("keydown", esc); };
  }, [open]);

  const pick = async (id: string) => {
    setBusy(true);
    try {
      if (await choose(id)) setOpen(false);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <span className="srv-switch-box" ref={box}>
      <button type="button" className="srv-switch" aria-haspopup="menu" aria-expanded={open} title={s("srv.menu.label")} onClick={() => setOpen(!open)}>
        <span className="srv-switch-name">{name}</span>
        <IconChevron className="srv-switch-chev" aria-hidden="true" />
      </button>
      {open && (
        <div className="srv-menu" role="menu" aria-label={s("srv.menu.label")}>
          {blocked && <p className="srv-menu-note" role="status"><IconAlert /> {blocked}</p>}
          {list.map((x) => {
            const on = x.id === used;
            const p = peeks.find((y) => y.id === x.id);
            const state = !x.api ? t("play.on.card.relay") : p?.session.logged_in ? t("play.on.card.in", { name: p.session.display_name || "—" }) : p ? t("play.on.card.out") : "";
            return (
              <button key={x.id} type="button" role="menuitemradio" aria-checked={on} className={`srv-menu-item ${on ? "on" : ""}`}
                disabled={busy || (!on && !!blocked)} onClick={() => void pick(x.id)}>
                <span className={`srv-card-ico ${x.api ? "" : "relay"}`} aria-hidden="true">{x.api ? <IconFriends /> : <IconRelay />}</span>
                <span className="srv-card-text">
                  <span className="srv-card-name">{x.name}</span>
                  {state && <span className="srv-card-state">{state}</span>}
                </span>
                {on && <IconCheck className="srv-menu-check" aria-label={s("srv.used")} />}
              </button>
            );
          })}
          <div className="srv-menu-foot">
            <button type="button" className="btn btn-small btn-ghost" onClick={() => { setOpen(false); setAdding(true); }}><IconPlus /> {s("srv.add.cta")}</button>
            <button type="button" className="btn btn-small btn-ghost" onClick={() => { setOpen(false); go("settings", "servers"); }}><IconSettings /> {s("srv.menu.manage")}</button>
          </div>
        </div>
      )}
      {adding && createPortal(<AddServerModal onClose={() => setAdding(false)} onAdded={() => { setAdding(false); void reloadServers(); }} />, document.body)}
    </span>
  );
}
