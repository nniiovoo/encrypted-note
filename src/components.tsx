// Small shared pieces for every screen. In-page only: no native popups while Unlocked (ADR-0002).

import { useEffect, useId, useRef, useState, type ComponentProps, type FormEvent, type KeyboardEvent as KeyEvent, type ReactNode } from "react";
import appIcon from "../src-tauri/icons/128x128@2x.png";
import { assessPassword, isApiError, listSafetyCopies, suggestPassphrase, type Assessment, type SafetyCopy } from "./api";
import { strings as s } from "./strings";

/** The Mac window draws its title bar over the page (tauri.conf.json), so the page provides the
 *  strip that moves the window. Elsewhere the OS title bar does that. */
export const mac = navigator.userAgent.includes("Mac");
export const dragRegion = mac ? { "data-tauri-drag-region": "deep" } : {};
export const TitlebarDrag = () => (mac ? <div className="titlebar-drag" {...dragRegion} /> : null);

/** Whether the shortcut modifier is held: Cmd alone on Mac (Ctrl+letter moves the cursor in Mac
 *  text boxes), Ctrl elsewhere (but not AltGr, which arrives as Ctrl+Alt). */
export const cmdKey = (e: { metaKey: boolean; ctrlKey: boolean; altKey: boolean }) =>
  mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.altKey;

export const AppIcon = ({ large }: { large?: boolean }) => (
  <img className={large ? "app-icon large" : "app-icon"} src={appIcon} alt="" />
);

/** For every text input: no autofill, autocorrect, spellcheck or writing suggestions (their
 *  popups are separate OS windows, and they must never learn a secret). */
export const noAssist = {
  autoComplete: "off",
  autoCorrect: "off",
  autoCapitalize: "off",
  spellCheck: false,
  writingsuggestions: "false",
} as const;

/** Rust timestamps are Unix seconds. */
export const formatDate = (t: number) =>
  new Date(t * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
export const seconds = (ms: number) => Math.ceil(ms / 1000);
export const minSec = (ms: number) => {
  const t = seconds(ms);
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, "0")}`;
};

/** Whole days from today, by this computer's own date, to a YYYY-MM-DD date: 0 on the day,
 *  negative once it is past. null for anything that isn't such a date. */
export const daysUntil = (date: string) => {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!m) return null;
  const today = new Date();
  const from = Date.UTC(today.getFullYear(), today.getMonth(), today.getDate());
  return Math.round((Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])) - from) / 86_400_000);
};
/** What to say about an API Key's expiry date once it is past or no more than 30 days away. */
export const expiry = (date: string | null | undefined) => {
  const days = date ? daysUntil(date) : null;
  return days === null || days > 30 ? null : { past: days < 0, text: days < 0 ? s.expired : s.expiresIn(days) };
};

/** Runs an API call and keeps its (already plain English) error. Resolves true on success.
 *  A cancelled native picker is not an error. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function run(fn: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await fn();
      return true;
    } catch (e) {
      if (!isApiError(e) || e.code !== "cancelled") setError(isApiError(e) ? e.message : s.somethingWentWrong);
      return false;
    } finally {
      setBusy(false);
    }
  }
  return { busy, error, run };
}

export const ErrorLine = ({ error }: { error: string | null }) =>
  error && (
    <p className="error" role="alert">
      {error}
    </p>
  );

/** The Safety Copies on this computer, listed by `reload`. `copies` is null until a list first
 *  arrives. A failed list sets `error` and keeps the last one: Safety Copies that can't be read
 *  are not the same as none. */
export function useSafetyCopies() {
  const [copies, setCopies] = useState<SafetyCopy[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const reload = () =>
    void listSafetyCopies().then(
      (list) => {
        setCopies(list);
        setError(null);
      },
      (e) => setError(isApiError(e) ? e.message : s.somethingWentWrong),
    );
  return { copies, error, reload };
}

/** Under a list of Safety Copies: still looking, none at all, or that they couldn't be read,
 *  with a way to try again. */
export function SafetyCopiesNote({ copies, error, reload }: ReturnType<typeof useSafetyCopies>) {
  if (error)
    return (
      <>
        <p className="error" role="alert">
          {copies?.length ? s.safetyCopiesStale : s.safetyCopiesUnreadable} {error}
        </p>
        <button onClick={reload}>{s.tryAgain}</button>
      </>
    );
  if (copies === null) return <p className="hint">{s.lookingForSafetyCopies}</p>;
  return copies.length === 0 && <p className="hint">{s.noSafetyCopies}</p>;
}

/** A labelled input. Says so when Caps Lock is on while typing into a password box. `hint`
 *  explains the field; `check` is what the page says about what is in it now (its length, a
 *  warning), one line each, and goes above the hint. */
export function Field({ label, hint, check, children }: {
  label: string;
  hint?: string | undefined;
  check?: ReactNode;
  children: ReactNode;
}) {
  const [caps, setCaps] = useState(false);
  const onKey = (e: KeyEvent) => setCaps((e.target as HTMLInputElement).type === "password" && e.getModifierState("CapsLock"));
  return (
    <label className="field" onKeyDown={onKey} onKeyUp={onKey}>
      <span className="label">{label}</span>
      {children}
      {caps && <span className="warn">{s.capsLockOn}</span>}
      {check}
      {hint && <span className="hint">{hint}</span>}
    </label>
  );
}

export const SecretInput = ({ show, ...props }: ComponentProps<"input"> & { show?: boolean }) => (
  <input {...props} {...noAssist} type={show ? "text" : "password"} />
);

/** A radio group drawn as a segmented control (native radios are in-page, unlike <select>).
 *  Not a fieldset: WebKit laid out a fieldset with a legend taller than it drew it. */
export function Choice<T extends string | number>(props: {
  label: string;
  options: readonly (readonly [T, string])[];
  value: T;
  onChange: (value: T) => void;
}) {
  const name = useId();
  return (
    <div className="choice" role="radiogroup" aria-labelledby={`${name}-label`}>
      <span className="label" id={`${name}-label`}>
        {props.label}
      </span>
      <div className="segmented">
        {props.options.map(([value, text]) => (
          <label key={value}>
            <input type="radio" name={name} checked={value === props.value} onChange={() => props.onChange(value)} />
            {text}
          </label>
        ))}
      </div>
    </div>
  );
}

/** A sheet; `alert` draws a short question centered under the app icon, like a Mac alert. */
export function Modal(props: { title: string; onClose: () => void; alert?: boolean; children: ReactNode }) {
  const { title, onClose, children } = props;
  const dialog = useRef<HTMLDivElement>(null);
  // Read during the first render, before any autoFocus inside the dialog moves focus.
  const [trigger] = useState(() => document.activeElement as HTMLElement | null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && topDialog() === dialog.current && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  // Keyboard focus stays inside the dialog, and goes back to what opened it afterwards.
  useEffect(() => {
    const el = dialog.current!;
    const keep = () => topDialog() !== el || el.contains(document.activeElement) || el.focus();
    keep();
    document.addEventListener("focusin", keep);
    return () => {
      document.removeEventListener("focusin", keep);
      const top = topDialog();
      // Not when a dialog that replaced this one already put the keyboard in its own input.
      if (top) top.contains(document.activeElement) || top.focus();
      // Only when nothing else took the keyboard as the dialog went: after Discard, a new Editor
      // already has it in its title box.
      else if (!el.isConnected && (!document.activeElement || document.activeElement === document.body)) trigger?.focus();
    };
  }, []);
  return (
    <div className="overlay">
      <TitlebarDrag />
      <div ref={dialog} tabIndex={-1} className={props.alert ? "modal alert" : "modal"} role="dialog" aria-modal="true" aria-label={title}>
        {props.alert && <AppIcon />}
        <h2>{title}</h2>
        {children}
      </div>
    </div>
  );
}

const topDialog = () => document.querySelectorAll<HTMLDivElement>("[role=dialog]").item(document.querySelectorAll("[role=dialog]").length - 1);

export const Confirm = (props: { title: string; text: string; confirmLabel: string; onConfirm: () => void; onClose: () => void }) => (
  <Modal title={props.title} onClose={props.onClose} alert>
    <p>{props.text}</p>
    <div className="actions">
      {/* Cancel has the keyboard: a stray Enter or Space changes nothing. A key still held down
          from opening the dialog doesn't press it either. */}
      <button autoFocus onKeyDown={(e) => e.repeat && e.preventDefault()} onClick={props.onClose}>
        {s.cancel}
      </button>
      <button className="primary" onClick={props.onConfirm}>
        {props.confirmLabel}
      </button>
    </div>
  </Modal>
);

/** Asks the Master Password again (Wallet Kinds, ADR-0004). The input is cleared as it is sent. */
export function PasswordModal(props: {
  title: string;
  text: string;
  submitLabel: string;
  onSubmit: (password: string) => Promise<unknown>;
  onClose: () => void;
  /** status.retry_in_ms: after wrong passwords the core refuses any attempt for a while. */
  retryInMs?: number | null | undefined;
}) {
  const [password, setPassword] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const { busy, error, run } = useAction();
  const wait = props.retryInMs ? seconds(props.retryInMs) : 0;
  async function submit(e: FormEvent) {
    e.preventDefault();
    // During the wait even the right password is refused unchecked: keep what was typed.
    if (wait > 0) return;
    const p = password;
    setPassword("");
    if (await run(() => props.onSubmit(p))) props.onClose();
  }
  // After a wrong password the button that was clicked is disabled again and typing would go
  // nowhere: the password box takes the keyboard back.
  useEffect(() => {
    const at = document.activeElement;
    if (!busy && (!at || at === document.body || (at as HTMLButtonElement).disabled)) input.current?.focus();
  }, [busy]);
  return (
    <Modal title={props.title} onClose={props.onClose}>
      <form onSubmit={submit} className="stack">
        <p>{props.text}</p>
        <Field label={s.masterPassword}>
          <SecretInput ref={input} value={password} onChange={(e) => setPassword(e.target.value)} autoFocus />
        </Field>
        <ErrorLine error={error} />
        {wait > 0 && <p className="hint">{s.tryAgainIn(wait)}</p>}
        <div className="actions">
          <button type="button" onClick={props.onClose}>
            {s.cancel}
          </button>
          <button className="primary" disabled={busy || !password || wait > 0}>
            {busy ? s.checking : props.submitLabel}
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** Choose a Master Password: strength meter, typed twice. `children` go at the top of the same
 *  form (e.g. the current password when changing it). */
export function NewPassword(props: {
  submitLabel: string;
  busyLabel: string;
  onSubmit: (password: string) => Promise<unknown>;
  /** Suggestion and show toggle: only on Create Master Password. */
  allowShow?: boolean;
  /** False while `children` still need input. */
  ready?: boolean;
  children?: ReactNode;
}) {
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const [show, setShow] = useState(false);
  const [assessment, setAssessment] = useState<Assessment | null>(null);
  const { busy, error, run } = useAction();

  useEffect(() => {
    setAssessment(null);
    if (!password) return;
    let live = true;
    const timer = setTimeout(() => assessPassword(password).then((a) => live && setAssessment(a), () => {}), 300);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [password]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const p = password;
    setPassword("");
    setAgain("");
    setShow(false);
    await run(() => props.onSubmit(p));
  }

  // A shown Master Password is ordinary text. It must not reach the general clipboard, which
  // nothing marks private or clears (Notes are copied by the Rust side for that reason).
  const noCopy = (e: { preventDefault: () => void }) => e.preventDefault();
  return (
    <form onSubmit={submit} onCopy={noCopy} onCut={noCopy} className="stack">
      {props.children}
      <Field label={s.newPassword}>
        <SecretInput show={show} value={password} onChange={(e) => setPassword(e.target.value)} autoFocus={!props.children} />
      </Field>
      {props.allowShow && (
        <button
          type="button"
          className="link"
          onClick={() => run(async () => { setPassword(await suggestPassphrase()); setShow(true); })}
        >
          {s.suggestPassword}
        </button>
      )}
      {password && (
        <div className="meter" data-score={assessment?.score ?? 0}>
          <div />
        </div>
      )}
      {assessment && <p className={assessment.acceptable ? "ok" : "warn"}>{assessment.acceptable ? s.strongEnough : s.notStrongEnough}</p>}
      {assessment?.feedback.map((f) => <p key={f} className="hint">{f}</p>)}
      <Field label={s.typeItAgain}>
        <SecretInput show={show} value={again} onChange={(e) => setAgain(e.target.value)} />
      </Field>
      {again && again !== password && <p className="warn">{s.passwordsDontMatch}</p>}
      {props.allowShow && (
        <label className="check">
          <input type="checkbox" checked={show} onChange={(e) => setShow(e.target.checked)} />
          {s.showPassword}
        </label>
      )}
      <ErrorLine error={error} />
      <button className="primary big" disabled={busy || props.ready === false || !assessment?.acceptable || password !== again}>
        {busy ? props.busyLabel : props.submitLabel}
      </button>
    </form>
  );
}
