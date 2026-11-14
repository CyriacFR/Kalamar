// What an update of the installed game will redo, said before it starts: on the wizard's plan screen, and on the Play
// page when this launcher carries other engine code than the one installed. The truth first: a short update or a full
// recompilation, how long, and why in one sentence. A long one (over two minutes) asks when: now (the installed game
// stays playable meanwhile), when the launcher closes, or later; a short one is one click.
import { useEffect, useState } from "react";
import { installApi, wizardIntent, type InstallRequest, type StepId, type UpdateCheck, type UpdatePreview } from "../api";
import { useApp } from "../ctx";
import { IconRefresh } from "../icons";
import { approx, useW } from "./texts";
import { minutes, updateClass, updateWhy } from "./wait";

type W = ReturnType<typeof useW>["w"];

/** The one sentence for an update: what is recompiled and about how long the whole run takes (advanced mode). */
export function updateSentence(w: W, u: UpdatePreview, eta: number, etaMax: number): string {
  const reason = u.reasons.map((r) => w(`upd.reason.${r}`)).join(", ");
  return w(`upd.kind.${u.kind}`, {
    t: approx(eta), tmax: approx(etaMax), lift: approx(u.lift_secs), files: u.changed_files, objects: u.objects,
    gen: u.generated_objects, reason: reason || w("upd.reason.lift_missing"),
  });
}

/** The class of the update ("Short update", "Full recompilation"), its duration and why, in plain words. */
export function UpdateTruth({ u, eta, etaMax }: { u: UpdatePreview; eta: number; etaMax: number }) {
  const { w } = useW();
  const cls = updateClass(u, eta);
  const ranged = u.kind === "generator" && etaMax > eta + 30;
  return (
    <div className={`upd-truth upd-${cls}`}>
      <p className="upd-truth-head">
        <span className="upd-badge">{w(`upd.class.${cls}`)}</span>
        <span className="upd-dur">{ranged ? w("upd.about.range", { t: approx(eta), tmax: approx(etaMax) }) : w("upd.about", { t: approx(eta) })}</span>
      </p>
      <p className="upd-why">{w(`upd.why.${updateWhy(u)}`, { objects: u.objects, gen: u.generated_objects, total: u.objects_total || u.generated_objects })}</p>
    </div>
  );
}

const SOURCE_STEPS: StepId[] = ["lift", "compile", "xlate", "keys"];
const short = (c: string) => (/^[0-9a-f]{40}$/.test(c) ? c.slice(0, 10) : c);

/** The plan screen's card: the truth, then (advanced mode) what is redone and what is kept as it is. */
export function UpdateSummary({ u, eta, etaMax, redo, from, to }: { u: UpdatePreview; eta: number; etaMax: number; redo: StepId[]; from: string | null; to: string | null }) {
  const { w } = useW();
  const { advanced } = useApp();
  const redone = SOURCE_STEPS.filter((s) => redo.includes(s));
  const kept = SOURCE_STEPS.filter((s) => !redo.includes(s));
  return (
    <div className="card upd-summary">
      <h2 className="card-title">{w("upd.title")}</h2>
      <UpdateTruth u={u} eta={eta} etaMax={etaMax} />
      {updateClass(u, eta) !== "short" && <p className="small muted">{w("upd.play.note")}</p>}
      {advanced && (
        <ul className="wiz-list small">
          <li>{updateSentence(w, u, eta, etaMax)}</li>
          {redone.length > 0 && <li>{w("upd.redo", { list: redone.map((s) => w(`upd.step.${s}`)).join(", ") })}</li>}
          {kept.length > 0 && <li className="muted">{w("upd.kept", { list: kept.map((s) => w(`upd.step.${s}`)).join(", ") })}</li>}
          {from && to && from !== to && <li className="muted tiny mono">{w("upd.source", { from: short(from), to: short(to) })}</li>}
        </ul>
      )}
    </div>
  );
}

/** Bundled commits whose notice was put off ("Later"), until the launcher restarts. */
const putOff = new Set<string>();

/** The installed game against the launcher's code, asked once per page that shows it. */
export function useUpdateCheck(): UpdateCheck | null {
  const [check, setCheck] = useState<UpdateCheck | null>(null);
  useEffect(() => {
    let alive = true;
    void installApi.updateCheck().then((c) => { if (alive) setCheck(c); }).catch(() => {});
    return () => { alive = false; };
  }, []);
  return check;
}

/** Opens the wizard straight on the plan of the update of `dir`. */
export function startUpdate(dir: string, go: (p: "setup") => void) {
  wizardIntent.updateDir = dir;
  go("setup");
}

/** The request of an update of `dir` (the remembered one, with that folder; the player's files are not needed). */
async function requestFor(dir: string): Promise<InstallRequest> {
  const info = await installApi.info();
  return { ...info.request, install_dir: dir, allow_prereqs: false, force_rebuild: false };
}

/** Play page: "Game update available", its truth, and when to run it. */
export function GameUpdateNotice() {
  const { w } = useW();
  const { go, advanced, fail, toast } = useApp();
  const check = useUpdateCheck();
  const [, setTick] = useState(0);
  const [scheduled, setScheduled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  if (!check?.available || !check.update || putOff.has(check.bundled_commit)) return null;
  const cls = updateClass(check.update, check.eta);
  const atClose = scheduled ?? check.at_close;
  const later = () => { putOff.add(check.bundled_commit); setTick((n) => n + 1); };

  // Now: straight to the install (it shows on the Play page and in the rail; the game stays playable). Only an update
  // that needs the player's files or a confirmation goes through the plan screen.
  const now = async () => {
    setBusy(true);
    try {
      const req = await requestFor(check.install_dir);
      const plan = await installApi.plan(req);
      if (plan.needs_files || plan.needs_prereqs) return startUpdate(check.install_dir, go);
      if (atClose) await installApi.atClose(null, null);
      await installApi.start(req);
      toast({ kind: "info", text: w("upd.started") });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  const atCloseOn = async (on: boolean) => {
    try {
      const req = on ? await requestFor(check.install_dir) : null;
      setScheduled(await installApi.atClose(req, on ? { tip: w("upd.tray.tip"), open: w("upd.tray.open"), quit: w("upd.tray.quit") } : null));
    } catch (e) {
      fail(e);
    }
  };

  return (
    <div className={`upd-line upd-${cls}`} role="status">
      <IconRefresh className="upd-ico" />
      <div className="grow upd-text">
        <p className="upd-title">{w("upd.avail.title")}</p>
        <UpdateTruth u={check.update} eta={check.eta} etaMax={check.eta_max} />
        {atClose ? <p className="small upd-sched">{w("upd.atclose.on", { t: approx(check.eta) })}</p>
          : cls !== "short" && <p className="tiny muted">{w("upd.play.note")}</p>}
        {advanced && <p className="small muted">{updateSentence(w, check.update, check.eta, check.eta_max)}</p>}
      </div>
      <div className="upd-actions">
        {cls === "short" ? (
          // A short update keeps its usual way: the button opens its plan, one more click starts it.
          <>
            <button className="btn btn-small" onClick={() => startUpdate(check.install_dir, go)}>{w("upd.avail.go")}</button>
            <button className="btn btn-small btn-ghost" onClick={later}>{w("upd.avail.later")}</button>
          </>
        ) : atClose ? (
          <>
            <button className="btn btn-small btn-ghost" disabled={busy} onClick={() => void now()}>{w("upd.now", { m: minutes(check.eta) })}</button>
            <button className="btn btn-small btn-ghost" onClick={() => void atCloseOn(false)}>{w("upd.atclose.cancel")}</button>
          </>
        ) : (
          <>
            <button className="btn btn-small" disabled={busy} onClick={() => void now()}>{w("upd.now", { m: minutes(check.eta) })}</button>
            <button className="btn btn-small btn-alt" onClick={() => void atCloseOn(true)}>{w("upd.atclose")}</button>
            <button className="btn btn-small btn-ghost" onClick={later}>{w("upd.avail.later")}</button>
          </>
        )}
        {cls !== "short" && <button className="link small upd-details" onClick={() => startUpdate(check.install_dir, go)}>{w("upd.details")}</button>}
      </div>
    </div>
  );
}
