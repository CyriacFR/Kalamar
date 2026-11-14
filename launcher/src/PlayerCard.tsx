// The player card of the rail: the splashtag composed in the launcher (or the name), a way into the Locker.
import { Suspense, lazy } from "react";
import { useApp } from "./ctx";
import { useI18n } from "./i18n";

// The splashtag needs the game catalogue and its icons, so it only exists once the Locker has loaded them; the card
// shows the player's name until then (and its code is not part of the first screen).
const TagPreview = lazy(() => import("./pages/profile/Splashtag").then((m) => ({ default: m.TagPreview })));

export function PlayerCard() {
  const { t, lang } = useI18n();
  const { player, go } = useApp();
  const p = player.profile;
  const name = p?.name?.trim();
  const cat = player.view?.catalogue;
  const plain = (
    <div className="stag stag-compact" aria-hidden="true">
      <span className="stag-bg stag-bg-none" />
      <span className="stag-name">{name || t("profile.name.placeholder")}</span>
    </div>
  );
  return (
    <button className="player" onClick={() => go("profile")} title={t("nav.locker")} aria-label={`${t("nav.locker")} — ${name || t("profile.name.placeholder")}`}>
      {cat && p ? (
        <Suspense fallback={plain}><TagPreview cat={cat} profile={p} lang={lang} t={t} compact /></Suspense>
      ) : (
        plain
      )}
    </button>
  );
}
