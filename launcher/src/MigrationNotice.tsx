// Data of an earlier launcher: the one-time notice (with details) and the manual import box of Settings > Launcher.
import { useEffect, useState } from "react";
import { api, type Migration } from "./api";
import { useApp } from "./ctx";
import { errorText, useI18n } from "./i18n";
import { toAppError } from "./api";
import { Modal, NoticeBar } from "./ui";
import type { Notice } from "./ctx";

function Details({ m, onClose }: { m: Migration; onClose: () => void }) {
  const { t } = useI18n();
  return (
    <Modal title={t("mig.title")} onClose={onClose}>
      <p className="small muted mono wrapany">{m.source}</p>
      <h3>{t("mig.imported", { n: m.imported.length })}</h3>
      {m.imported.length ? <ul className="notes mono small">{m.imported.map((f) => <li key={f}>{f}</li>)}</ul> : <p className="muted">{t("mig.none")}</p>}
      {m.rejected.length > 0 && (
        <>
          <h3 className="bad-text">{t("mig.rejected", { n: m.rejected.length })}</h3>
          <ul className="notes mono small">{m.rejected.map((f) => <li key={f}>{f}</li>)}</ul>
        </>
      )}
      {m.skipped.length > 0 && (
        <>
          <h3>{t("mig.skipped")}</h3>
          <ul className="notes mono small">{m.skipped.map((f) => <li key={f}>{f}</li>)}</ul>
        </>
      )}
      <p className="hint">{t("mig.safe")}</p>
      <div className="row"><button className="btn" data-autofocus onClick={onClose}>{t("mig.ok")}</button></div>
    </Modal>
  );
}

/** Shown once, never blocks: appears at the corner after an import and goes away on "OK". */
export function MigrationNotice({ initial }: { initial: Migration | null }) {
  const { t } = useI18n();
  const [m, setM] = useState<Migration | null>(initial);
  const [open, setOpen] = useState(false);
  // The start-up call already read it; the notice only follows what that returned.
  useEffect(() => { setM((cur) => cur ?? initial); }, [initial]);
  if (!m) return null;
  const dismiss = () => { void api.migrationAck().catch(() => {}); setM(null); setOpen(false); };
  return (
    <>
      <div className="import-notice" role="status">
        <span>{m.rejected.length ? t("mig.notice.rej", { n: m.files, r: m.rejected.length }) : t("mig.notice", { n: m.files })}</span>
        <button className="link" onClick={() => setOpen(true)}>{t("mig.details")}</button>
        <button className="btn btn-small btn-ghost" onClick={dismiss}>{t("mig.ok")}</button>
      </div>
      {open && <Details m={m} onClose={dismiss} />}
    </>
  );
}

/** Settings > Launcher: import from the data folder of an earlier launcher (a custom path, for example). */
export function ImportBox() {
  const { t } = useI18n();
  const { setConfig, reloadPlayer, toast } = useApp();
  const [path, setPath] = useState("");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [done, setDone] = useState<Migration | null>(null);
  const go = async () => {
    try {
      setNotice(null);
      const m = await api.migrationImport(path.trim());
      setConfig(await api.getConfig());
      void reloadPlayer();
      setDone(m);
      toast({ kind: "ok", text: t("mig.notice", { n: m.files }) });
    } catch (e) {
      setNotice({ kind: "error", text: errorText(toAppError(e), t) });
    }
  };
  return (
    <div className="server-field">
      <label className="field-label" htmlFor="mig-path">{t("mig.import.title")}</label>
      <div className="row">
        <input id="mig-path" className="mono grow" value={path} placeholder="D:\Kalamar\launcher-data" onChange={(e) => setPath(e.target.value)} onKeyDown={(e) => e.key === "Enter" && path.trim() && void go()} />
        <button className="btn btn-ghost" disabled={!path.trim()} onClick={() => void go()}>{t("mig.import.go")}</button>
      </div>
      <p className="hint">{t("mig.import.hint")}</p>
      <NoticeBar notice={notice} />
      {done && <Details m={done} onClose={() => setDone(null)} />}
    </div>
  );
}
