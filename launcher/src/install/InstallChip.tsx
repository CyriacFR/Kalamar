// The installation, always in sight from the rail while the player is elsewhere: a ring with the percentage, the step
// and the time left; one click opens the install screen. Once it has ended, until that screen is seen: "ready" or
// "stopped". Part of the first screen's code (the rail), so it stays small.
import type { InstallSnapshot } from "../api";
import { useI18n } from "../i18n";
import { lookup } from "../locales";
import { IconAlert, IconCheck } from "../icons";

/** "8 min", "40 s": rough, for a glance. */
function rough(secs: number): string {
  return secs < 90 ? `${Math.max(10, Math.round(secs / 10) * 10)} s` : `${Math.round(secs / 60)} min`;
}

export function InstallChip({ snap, onOpen }: { snap: InstallSnapshot; onOpen: () => void }) {
  const { t, lang } = useI18n();
  const pct = Math.floor(snap.overall * 100);
  const r = 15.5;
  const c = 2 * Math.PI * r;
  const failed = !snap.active && snap.outcome === "failed";
  const done = !snap.active && snap.outcome === "done";
  const what = snap.update ? t("rail.inst.update") : t("rail.inst.install");
  // The step in the wizard's words, without the remark in brackets ("Compilation (la plus longue)" -> "Compilation").
  const step = snap.current ? lookup("install", lang, `step.${snap.current}`).replace(/\s*[(（].*$/, "") : "";
  const state = done ? t(snap.update ? "rail.inst.done.update" : "rail.inst.done")
    : failed ? t("rail.inst.failed")
    : !snap.active ? t("rail.inst.stopped")
    : snap.waiting === "game_running" ? t("rail.inst.wait")
    : snap.replacing ? t("rail.inst.swap")
    : snap.eta != null ? t("rail.inst.left", { t: rough(snap.eta) }) : t("rail.inst.going");
  const tone = done ? "ok" : failed || !snap.active ? "bad" : snap.waiting ? "wait" : "run";
  const label = snap.active ? `${what} — ${pct} % — ${step} — ${state}` : `${what} — ${state}`;
  return (
    <button type="button" className={`inst-chip inst-${tone}`} onClick={onOpen} title={`${label}. ${t("rail.inst.open")}`} aria-label={`${label}. ${t("rail.inst.open")}`}>
      <span className="inst-ring" aria-hidden="true">
        {done ? <IconCheck /> : failed ? <IconAlert /> : (
          <>
            <svg className="inst-prog" viewBox="0 0 36 36">
              <circle cx="18" cy="18" r={r} className="inst-ring-bg" />
              <circle cx="18" cy="18" r={r} className="inst-ring-fg" strokeDasharray={c} strokeDashoffset={c * (1 - Math.max(0.02, snap.overall))} />
            </svg>
            <span className="inst-pct">{pct}<small>%</small></span>
          </>
        )}
      </span>
      <span className="inst-text">
        <span className="inst-what">{what}</span>
        {snap.active && step && <span className="inst-step">{step}</span>}
        <span className="inst-line">{state}</span>
      </span>
    </button>
  );
}
