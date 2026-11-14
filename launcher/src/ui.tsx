// Shared presentational pieces of the design system.
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { errorText, useI18n } from "./i18n";
import { api, toAppError } from "./api";
import { useApp, type Notice } from "./ctx";
import { copyText } from "./diagnostic";
import { IconAlert, IconBack, IconCheck, IconClose, IconCopy, IconFolder, IconInfo } from "./icons";

export function Page({ title, subtitle, children, badge, wide, eyebrow, actions, back, className }: {
  title: ReactNode; subtitle?: ReactNode; children: ReactNode; badge?: string; wide?: boolean; eyebrow?: ReactNode; actions?: ReactNode;
  back?: { label: string; onClick: () => void }; className?: string;
}) {
  return (
    <section className={`page ${wide ? "page-wide" : ""} ${className ?? ""}`}>
      {back && (
        <button className="back" onClick={back.onClick}><IconBack /> {back.label}</button>
      )}
      <header className="page-head">
        <div className="page-head-text">
          {eyebrow && <div className="eyebrow">{eyebrow}</div>}
          <h1 className="display page-title">
            {title}
            {badge && <span className="badge badge-hot">{badge}</span>}
          </h1>
          {subtitle && <p className="subtitle">{subtitle}</p>}
        </div>
        {actions && <div className="page-actions">{actions}</div>}
      </header>
      {children}
    </section>
  );
}

export function Card({ title, children, tone, className }: { title?: string; children: ReactNode; tone?: "warn" | "ok" | "bad"; className?: string }) {
  return (
    <div className={`card ${tone ? `card-${tone}` : ""} ${className ?? ""}`}>
      {title && <h2 className="card-title">{title}</h2>}
      {children}
    </div>
  );
}

/** A heading inside a page: the second level of the hierarchy (page title, section, card title). */
export function Section({ title, hint, children, className, actions }: { title: string; hint?: ReactNode; children: ReactNode; className?: string; actions?: ReactNode }) {
  return (
    <section className={`section ${className ?? ""}`}>
      <header className="section-head">
        <div>
          <h2 className="section-title">{title}</h2>
          {hint && <p className="section-hint">{hint}</p>}
        </div>
        {actions}
      </header>
      {children}
    </section>
  );
}

export function NoticeBar({ notice }: { notice: Notice | null }) {
  if (!notice) return null;
  const Icon = notice.kind === "error" ? IconAlert : notice.kind === "ok" ? IconCheck : IconInfo;
  return (
    <div className={`notice notice-${notice.kind}`} role={notice.kind === "error" ? "alert" : "status"}>
      <Icon width="1.2em" height="1.2em" style={{ flex: "none", marginTop: "0.1em" }} />
      <span>{notice.text}</span>
    </div>
  );
}

/** Turns anything thrown by an IPC call into a user-facing notice (translated by error kind). */
export function useErrorNotice() {
  const { t } = useI18n();
  return (e: unknown): Notice => ({ kind: "error", text: errorText(toAppError(e), t) });
}

/** A state in words with a colored dot (no box): ok, bad or neutral. */
export function Pill({ ok, children }: { ok: boolean | null; children: ReactNode }) {
  return <span className={`pill ${ok === null ? "pill-neutral" : ok ? "pill-ok" : "pill-bad"}`}>{children}</span>;
}

/** Empty or error state with an optional action. */
export function State({ icon, title, children, tone, action }: { icon: ReactNode; title: string; children?: ReactNode; tone?: "bad"; action?: ReactNode }) {
  return (
    <div className={`state ${tone ? `state-${tone}` : ""}`} role={tone === "bad" ? "alert" : undefined}>
      {icon}
      <h3>{title}</h3>
      {children && <p>{children}</p>}
      {action}
    </div>
  );
}

export function Skeleton({ w, h = "1rem" }: { w?: string; h?: string }) {
  return <span className="skeleton" style={{ width: w ?? "100%", height: h }} aria-hidden="true" />;
}

/** Shows its content only in the advanced mode. */
export function Adv({ children }: { children: ReactNode }) {
  const { advanced } = useApp();
  return advanced ? <>{children}</> : null;
}

/** A button that copies text made on demand (a diagnostic, a code), then says "Copied" for a moment. */
export function CopyButton({ text, label, small, ghost = true }: { text: () => string | Promise<string>; label: string; small?: boolean; ghost?: boolean }) {
  const { t } = useI18n();
  const [state, setState] = useState<"idle" | "ok" | "bad">("idle");
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const run = async () => {
    const ok = await copyText(await text());
    setState(ok ? "ok" : "bad");
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setState("idle"), 2200);
  };
  return (
    <button type="button" className={`btn ${ghost ? "btn-ghost" : ""} ${small ? "btn-small" : ""}`} onClick={() => void run()} aria-live="polite">
      {state === "ok" ? <IconCheck /> : <IconCopy />} {state === "ok" ? t("diag.copied") : state === "bad" ? t("diag.copy.fail") : label}
    </button>
  );
}

/** A problem said in one sentence, what to do, and the actions: retry or fix (the caller's), "Copy the diagnostic"
 *  (the details that used to be on screen) and, in the advanced mode, the logs folder and the raw details. */
export function Problem({ title, fix, tone = "bad", actions, diag, logs, details, onClose }: {
  title: string; fix?: ReactNode; tone?: "bad" | "warn"; actions?: ReactNode; diag?: () => Promise<string> | string; logs?: () => Promise<unknown>;
  details?: ReactNode; onClose?: () => void;
}) {
  const { t } = useI18n();
  const { advanced, fail } = useApp();
  return (
    <div className={`problem problem-${tone}`} role={tone === "bad" ? "alert" : "status"}>
      <IconAlert className="problem-ico" />
      <div className="problem-body">
        <p className="problem-title">{title}</p>
        {fix && <p className="problem-fix">{fix}</p>}
        {(actions || diag || (advanced && logs)) && (
          <div className="row wrap problem-actions">
            {actions}
            {diag && <CopyButton text={diag} label={t("diag.copy")} small />}
            {advanced && logs && <button type="button" className="btn btn-ghost btn-small" onClick={() => void logs().catch(fail)}><IconFolder /> {t("play.logs")}</button>}
          </div>
        )}
        {advanced && details}
      </div>
      {onClose && <button type="button" className="icon-btn problem-close" onClick={onClose} aria-label={t("common.close")} title={t("common.close")}><IconClose /></button>}
    </div>
  );
}

/** The logs folder of the game (advanced actions). */
export const openLogs = () => api.openLogsFolder();

/** Modal dialog: Escape and the scrim close it, focus moves in and returns to the opener. */
export function Modal({ title, children, onClose, labelledBy = "modal-title", wide, actions }: {
  title: string; children: ReactNode; onClose: () => void; labelledBy?: string;
  /** A large dialog (a grid of choices) instead of a message box. */
  wide?: boolean;
  /** Shown at the right of the title (a close button, a count). */
  actions?: ReactNode;
}) {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    (box.current?.querySelector<HTMLElement>("[data-autofocus]:not(:disabled)") ?? box.current?.querySelector<HTMLElement>("button, input, select"))?.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      if (e.key === "Tab" && box.current) {
        const f = [...box.current.querySelectorAll<HTMLElement>("button, input, select, textarea, [href]")].filter((x) => !x.hasAttribute("disabled"));
        if (!f.length) return;
        const first = f[0], last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    };
    document.addEventListener("keydown", key);
    return () => { document.removeEventListener("keydown", key); prev?.focus(); };
  }, [onClose]);
  // In the document's body: a dialog opened from inside a card (whose backdrop filter would make it the box of a fixed
  // element) still covers the window.
  return createPortal(
    <div className="scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`modal ${wide ? "modal-wide" : ""}`} role="dialog" aria-modal="true" aria-labelledby={labelledBy} ref={box}>
        {actions ? (
          <div className="modal-head">
            <h2 id={labelledBy}>{title}</h2>
            {actions}
          </div>
        ) : (
          <h2 id={labelledBy}>{title}</h2>
        )}
        {children}
      </div>
    </div>,
    document.body,
  );
}
