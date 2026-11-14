// Asked when the window is closed while an installation runs: never cut it off silently.
import { useEffect, useState } from "react";
import { installApi, type InstallSnapshot } from "../api";
import { Modal } from "../ui";
import { useW } from "./texts";

export function CloseDuringInstall({ onClose }: { onClose: () => void }) {
  const { w } = useW();
  const [snap, setSnap] = useState<InstallSnapshot | null>(null);
  const [stopping, setStopping] = useState(false);
  useEffect(() => { void installApi.status().then(setSnap); }, []);
  const step = snap?.current ? w(`step.${snap.current}`) : "…";
  return (
    <Modal title={w("close.title")} onClose={onClose}>
      <p>{w("close.text", { step })}</p>
      <div className="stack">
        <button className="btn" data-autofocus onClick={onClose}>{w("close.stay")}</button>
        <button className="btn btn-alt" onClick={() => { void installApi.close("background"); onClose(); }}>{w("close.background")}</button>
        <button className="btn btn-danger" disabled={stopping} onClick={() => { setStopping(true); void installApi.close("stop"); }}>
          {stopping ? w("wiz.run.cancelling") : w("close.stop")}
        </button>
      </div>
      <p className="hint">{w("close.hint")}</p>
    </Modal>
  );
}
