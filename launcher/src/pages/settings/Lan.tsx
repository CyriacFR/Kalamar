// Settings > Local network (advanced mode): the experimental LAN play, direct or through a relay, and hosting a relay
// on this PC. Formerly its own page.
import { useEffect, useRef, useState } from "react";
import { api, type LanMode, type LanRequest, type RelayStatus } from "../../api";
import { useApp, type Notice } from "../../ctx";
import { useI18n } from "../../i18n";
import { NoticeBar, Pill, useErrorNotice } from "../../ui";
import { GameState, LanHelp } from "../play/AdvancedPlay";

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;
const isIpv4 = (s: string) => {
  const m = IPV4.exec(s.trim());
  return !!m && m.slice(1).every((x) => Number(x) <= 255);
};

function RelayBox({ setNotice }: { setNotice: (n: Notice | null) => void }) {
  const { t } = useI18n();
  const { config, setConfig } = useApp();
  const errNotice = useErrorNotice();
  const [st, setSt] = useState<RelayStatus | null>(null);
  const [port, setPort] = useState(String(config?.relay_port ?? 47313));
  const [exe, setExe] = useState(config?.relay_exe ?? "");

  const refresh = () => api.relayStatus().then(setSt).catch(() => {});
  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), 2000);
    return () => window.clearInterval(id);
  }, []);

  const run = async (f: () => Promise<RelayStatus | void>) => {
    try {
      setNotice(null);
      const r = await f();
      if (r) setSt(r);
    } catch (e) {
      setNotice(errNotice(e));
    }
  };
  const portNum = Number(port);
  const portOk = Number.isInteger(portNum) && portNum >= 1 && portNum <= 65535;

  return (
    <div className="set-block">
      <div className="set-block-head"><h3>{t("lan.relaybox.title")}</h3></div>
      <label className="field-label" htmlFor="relay-exe">{t("lan.relaybox.exe")}</label>
      <div className="row">
        <input id="relay-exe" className="mono grow" value={exe} placeholder={st?.exe ?? "...\\relay\\target\\release\\splat3-relay.exe"} onChange={(e) => setExe(e.target.value)} />
        <button type="button" className="btn btn-ghost btn-small" onClick={() => void run(async () => { setConfig(await api.setRelayExe(exe.trim() || null)); return api.relayStatus(); })}>{t("lan.relaybox.set")}</button>
      </div>
      <p className="hint mono wrapany">{st?.exe ? t("lan.relaybox.found", { path: st.exe }) : t("lan.relaybox.missing")}</p>
      <div className="form-row">
        <label className="field-label" htmlFor="relay-port">{t("lan.relay.port")}</label>
        <input id="relay-port" className="mono" style={{ width: 96 }} value={port} inputMode="numeric" onChange={(e) => setPort(e.target.value)} aria-invalid={!portOk} disabled={st?.running} />
        {!st?.running ? (
          <button type="button" className="btn btn-small" disabled={!portOk || !st?.exe} onClick={() => void run(() => api.relayStart(portNum))}>{t("lan.relaybox.start")}</button>
        ) : (
          <button type="button" className="btn btn-danger btn-small" onClick={() => void run(api.relayStop)}>{t("lan.relaybox.stop")}</button>
        )}
      </div>
      <p>
        {st?.running ? <Pill ok={true}>{t("lan.relaybox.running", { pid: st.pid ?? "?", port: st.port ?? port })}</Pill> : <Pill ok={null}>{t("lan.relaybox.stopped")}</Pill>}
        {st && !st.running && st.exited !== null && <span className="bad-text"> {t("lan.relaybox.exited", { code: st.exited })}</span>}
      </p>
      <p className="hint">{t("lan.relaybox.firewall", { port: st?.port ?? (portOk ? portNum : 47313) })}</p>
      <p className="hint">{t("lan.relaybox.stays")}</p>
      {st?.tail && <pre className="log">{st.tail}</pre>}
    </div>
  );
}

export function LanSection() {
  const { t } = useI18n();
  const { game, status, config, setConfig } = useApp();
  const errNotice = useErrorNotice();
  const [mode, setMode] = useState<LanMode>("lan");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("47313");
  const [room, setRoom] = useState("");
  const [netLog, setNetLog] = useState(false);
  const [bindAny, setBindAny] = useState(false);
  const [address, setAddress] = useState("");
  const [env, setEnv] = useState<[string, string][] | null>(null);
  const [envErr, setEnvErr] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [copied, setCopied] = useState(false);
  const loaded = useRef(false);

  const enabled = config?.lan_enabled ?? false;

  useEffect(() => {
    if (!config || loaded.current) return;
    loaded.current = true;
    const l = config.lan;
    setMode(l.mode);
    setHost(l.relay_host);
    setPort(String(l.relay_port));
    setRoom(l.room);
    setNetLog(l.net_log);
    setBindAny(l.bind_any);
    setAddress(l.lan_address);
  }, [config]);

  const portNum = Number(port);
  const request: LanRequest = { mode, relay_host: host.trim(), relay_port: Number.isInteger(portNum) ? portNum : 0, room: room.trim(), net_log: netLog, bind_any: bindAny, lan_address: address.trim() };
  const key = JSON.stringify(request);

  useEffect(() => {
    let alive = true;
    api.lanPreview(JSON.parse(key) as LanRequest).then((e) => { if (alive) { setEnv(e); setEnvErr(null); } }).catch((e) => { if (alive) { setEnv(null); setEnvErr(errNotice(e).text); } });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  // The LAN card of the Play screen launches with these settings: keep them saved as they are edited.
  useEffect(() => {
    if (!loaded.current) return;
    const id = window.setTimeout(() => { void api.setLanPrefs(JSON.parse(key) as LanRequest).catch(() => {}); }, 500);
    return () => window.clearTimeout(id);
  }, [key]);

  const toggle = async (on: boolean) => {
    try {
      setConfig(await api.setLanEnabled(on));
    } catch (e) {
      setNotice(errNotice(e));
    }
  };

  const generate = async () => setRoom(await api.lanGenerateRoom());
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(room);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      setNotice({ kind: "error", text: t("diag.copy.fail") });
    }
  };
  const launch = async () => {
    try {
      setNotice(null);
      await api.setLanPrefs(request);
      await api.launchMode({ mode: "lan" });
    } catch (e) {
      setNotice(errNotice(e));
    }
  };

  const running = game.state === "running";

  return (
    <div className="stack">
      <p className="warn-text small">{t("lan.exp2")}</p>
      <label className="switch-row">
        <span className="switch"><input type="checkbox" checked={enabled} onChange={(e) => void toggle(e.target.checked)} /><span className="switch-ui" aria-hidden="true" /></span>
        <span>{t("lan.enable")}</span>
      </label>

      <fieldset className="plain" disabled={!enabled}>
        <div className="set-block">
          <div className="set-block-head">
            <h3>{t("lan.mode")}</h3>
            <div className="seg" role="radiogroup" aria-label={t("lan.mode")}>
              {(["lan", "relay"] as const).map((m) => (
                <button key={m} type="button" role="radio" aria-checked={mode === m} className={`seg-btn ${mode === m ? "on" : ""}`} onClick={() => setMode(m)}>
                  {t(m === "lan" ? "lan.mode.lan" : "lan.mode.relay")}
                </button>
              ))}
            </div>
          </div>

          {mode === "relay" && (
            <>
              <label className="field-label" htmlFor="relay-host">{t("lan.relay.host")}</label>
              <div className="row">
                <input id="relay-host" className="mono grow" value={host} placeholder="203.0.113.7" onChange={(e) => setHost(e.target.value)} aria-invalid={host.trim() !== "" && !isIpv4(host)} />
                <label className="field-label nomargin" htmlFor="relay-port-use">{t("lan.relay.port")}</label>
                <input id="relay-port-use" className="mono" style={{ width: 96 }} value={port} inputMode="numeric" onChange={(e) => setPort(e.target.value)} />
              </div>
              <p className="hint">{t("lan.relay.host.hint")}</p>

              <label className="field-label" htmlFor="room">{t("lan.room2")}</label>
              <div className="row">
                <input id="room" className="mono grow code" value={room} maxLength={32} onChange={(e) => setRoom(e.target.value)} />
                <button type="button" className="btn btn-ghost btn-small" onClick={() => void generate()}>{t("lan.room.gen")}</button>
                <button type="button" className="btn btn-ghost btn-small" disabled={!room} onClick={() => void copy()}>{copied ? t("lan.room.copied") : t("lan.room.copy")}</button>
              </div>
              <p className="hint">{t("lan.room.hint")}</p>
            </>
          )}

          {mode === "lan" && (
            <>
              <label className="field-label" htmlFor="lan-address">{t("lan.address")}</label>
              <input id="lan-address" className="mono" value={address} placeholder="192.168.1.20/24" onChange={(e) => setAddress(e.target.value)} />
              <p className="hint">{t("lan.address.hint")}</p>
              <label className="check"><input type="checkbox" checked={bindAny} onChange={(e) => setBindAny(e.target.checked)} /> {t("lan.bindany")}</label>
            </>
          )}
          <label className="check"><input type="checkbox" checked={netLog} onChange={(e) => setNetLog(e.target.checked)} /> {t("lan.netlog")}</label>
        </div>

        <div className="set-block">
          <div className="set-block-head"><h3>{t("lan.env2")}</h3></div>
          {env ? (
            <table className="envtable"><tbody>{env.map(([k, v]) => <tr key={k}><td className="mono">{k}</td><td className="mono">{v}</td></tr>)}</tbody></table>
          ) : (
            <p className="bad-text">{envErr}</p>
          )}
          <div className="row wrap">
            <button type="button" className="btn" disabled={!enabled || running || !(status?.ready) || !env} onClick={() => void launch()}>{t("lan.launch")}</button>
          </div>
        </div>
      </fieldset>
      <NoticeBar notice={notice} />
      {game.mode === "lan" && <GameState />}
      {game.mode === "lan" && <LanHelp />}
      <RelayBox setNotice={setNotice} />
    </div>
  );
}
