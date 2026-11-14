// Logging in with the providers the server declares: a form of its fields (`direct`), or the system browser
// (`oauth_loopback`). Unknown kinds are not offered.
import { useState } from "react";
import { proto, type J } from "./api";
import { FieldForm, fieldBody, fieldDefaults, unsupportedField, ErrorLine, useAction } from "./ui";
import { getLive, loggedIn } from "./store";
import { useC } from "./texts";

function DirectProvider({ server, p, meta, nameMax }: { server: string; p: J; meta: J; nameMax: number }) {
  const { c, L } = useC();
  const fields: J[] = Array.isArray(p.fields) ? p.fields : [];
  const [values, setValues] = useState<Record<string, unknown>>(() => fieldDefaults(fields));
  const { busy, error, run } = useAction();
  const unknown = unsupportedField(fields);
  const missing = fields.some((f: J) => f.required && (values[f.key] === undefined || values[f.key] === ""));
  const go = () => void run(async () => loggedIn(await proto.login(server, p.id, fieldBody(fields, values))), () => ({ min: getLive().overview?.check?.display_name_min ?? 1, max: nameMax }));
  return (
    <article className="card cm-provider">
      <h3>{L(p.label) || p.id}</h3>
      {L(p.warning) && <p className="hint warn-text">{L(p.warning)}</p>}
      {unknown ? (
        <p className="bad-text">{c("field.too_old", { key: unknown })}</p>
      ) : (
        <form onSubmit={(e) => { e.preventDefault(); if (!missing) go(); }}>
          <FieldForm fields={fields} values={values} set={(k, v) => setValues((o) => ({ ...o, [k]: v }))} meta={meta} idPrefix={`login-${p.id}`} nameMax={nameMax} />
          <ErrorLine text={error} />
          <button className="btn" type="submit" disabled={busy || missing}>{c("login.go")}</button>
        </form>
      )}
    </article>
  );
}

function OAuthProvider({ server, p }: { server: string; p: J }) {
  const { c, L } = useC();
  const { busy, error, run } = useAction();
  return (
    <article className="card cm-provider">
      <h3>{L(p.label) || p.id}</h3>
      {L(p.warning) && <p className="hint warn-text">{L(p.warning)}</p>}
      <button className="btn btn-alt" disabled={busy} onClick={() => void run(async () => loggedIn(await proto.oauth(server, p.id)))}>{L(p.label) || c("login.go")}</button>
      {busy && <p className="hint" role="status">{c("login.oauth.wait")}</p>}
      <ErrorLine text={error} />
    </article>
  );
}

export function Login({ server, name, meta, nameMax }: { server: string; name: string; meta: J; nameMax: number }) {
  const { c, L } = useC();
  const providers: J[] = (meta?.auth?.providers ?? []).filter((p: J) => p && (p.kind === "direct" || p.kind === "oauth_loopback"));
  // Two columns: what the account is and the server's rules on the left, the ways to log in on the right.
  return (
    <section className="cm-login">
      <div className="stack">
        <h2 className="section-title">{c("login.title", { name })}</h2>
        <p className="muted">{c("login.intro")}</p>
        {L(meta?.rules?.summary) && <p className="cm-rules">{L(meta.rules.summary)}</p>}
        <p className="hint wrapany">
          {meta?.privacy_url && <span>{c("login.privacy", { url: meta.privacy_url })} </span>}
          {meta?.rules?.url && <span>· {c("login.rules", { url: meta.rules.url })}</span>}
        </p>
      </div>
      <div className="stack">
        {providers.length === 0 && <p className="bad-text">{c("login.none")}</p>}
        {providers.map((p) => (p.kind === "direct" ? <DirectProvider key={p.id} server={server} p={p} meta={meta} nameMax={nameMax} /> : <OAuthProvider key={p.id} server={server} p={p} />))}
      </div>
    </section>
  );
}
