// One Note: the detail view (Show / Copy of Hidden Fields) and the editor for every Kind.

import { Fragment, useEffect, useId, useRef, useState, type ClipboardEvent as PasteEvent, type FormEvent, type KeyboardEvent as KeyEvent } from "react";
import {
  appendText,
  cancelNativeOperation,
  checkPrivateKey,
  checkSeed,
  copyField,
  copyFieldEnv,
  copyFieldLine,
  copyVisibleField,
  createNote,
  extendReveal,
  getNote,
  hideField,
  isApiError,
  isWalletKind,
  markSeedExplainerSeen,
  seedWordlist,
  setFavorite,
  showAndCopyWalletField,
  showField,
  showWalletField,
  trashNote,
  updateNote,
  type AppStatus,
  type Chain,
  type Kind,
  type NoteView,
} from "./api";
import { Choice, cmdKey, Confirm, ErrorLine, expiry, Field, formatDate, mac, Modal, noAssist, NativeActionModal, SecretInput, seconds, useAction } from "./components";
import { Icon, KindBadge } from "./icons";
import { strings as s } from "./strings";

// Field names as in notes::field, in form order.
const FORM: Record<Kind, string[]> = {
  seed_phrase: ["word_count", "words", "wallet_name", "address", "derivation_path", "passphrase"],
  private_key: ["chain", "address", "key"],
  login: ["website", "username", "password", "purpose"],
  api_key: ["service", "key", "secret", "key_id", "purpose", "url", "expires", "env_var"],
  text: ["body"],
};
const HIDDEN = ["words", "passphrase", "key", "secret", "password", "body"];
const OPTIONAL = ["wallet_name", "passphrase", "address", "derivation_path", "username", "purpose", "key_id", "url", "expires", "env_var", "secret"];
// The fields the Editor folds under one line, in FORM's order. FORM still lists them.
const MORE: Partial<Record<Kind, string[]>> = {
  seed_phrase: ["address", "derivation_path", "passphrase"],
  api_key: ["secret", "key_id", "purpose", "url", "expires", "env_var"],
};
// The Hidden Fields whose typed text the Editor can show (ADR-0006). Never a Wallet Kind.
const SHOWN_WHILE_TYPING: Partial<Record<Kind, string[]>> = { login: ["password"], api_key: ["key", "secret"] };
// From the press. Typing doesn't extend it.
const SHOWN_WHILE_TYPING_MS = 30_000;
// The Hidden Field a Note of the Kind is for. (A Seed Phrase can't be saved without its words.)
const MAIN: Partial<Record<Kind, string>> = { private_key: "key", login: "password", api_key: "key", text: "body" };
// The field a new Note's title is taken from when the title was left blank.
const TITLE_FROM: Partial<Record<Kind, string>> = { login: "website", api_key: "service" };
// Not carried into a duplicate: it is another wallet, with its own address and word count.
const NOT_DUPLICATED = ["address", "word_count"];
// Visible fields without a Copy button: the chain is shown by another name than the one stored,
// and the word count is a number the app worked out.
const NOT_COPIED = ["chain", "word_count"];
// The optional fields older versions of the app and of the Emergency Reader can't read: a Vault
// with a value in one of them opens only in current ones (notes::Kind::schema_2_fields).
const NEWER: Partial<Record<Kind, string[]>> = {
  seed_phrase: ["address", "derivation_path"],
  login: ["purpose"],
  api_key: ["key_id", "purpose", "url", "expires", "env_var", "secret"],
};
// A copied Seed Phrase or Private Key is cleared after this long, whatever Settings says.
const WALLET_COPY_SECONDS = 30;
const TEXT_MODES = [["replace", s.replaceTheText], ["add", s.addToTheEnd]] as const;
const CHAINS = Object.entries(s.chains) as [Chain, string][];
// The chains whose keys the core can check. Any other chain is the owner's own text in the file.
const PRESETS: readonly string[] = ["evm", "solana", "bitcoin"];
const CHAIN_NAMES = new Map<string, Chain>([
  ["ethereum", "evm"], ["eth", "evm"], ["evm", "evm"],
  ["solana", "solana"], ["sol", "solana"],
  ["bitcoin", "bitcoin"], ["btc", "bitcoin"],
]);
/** A stored chain as shown. Only one of the listed keys is looked up, never the raw text: a
 *  chain the owner named "constructor" shows as that word. */
const chainLabel = (chain: string) => (Object.hasOwn(s.chains, chain) ? s.chains[chain as Chain] : chain);
const label = (kind: Kind, field: string) => s.fields[kind][field] ?? field;

/** Lines in a pasted text. A single line break at the very end is not another line. */
const lineCount = (text: string) => text.replace(/(\r\n|\r|\n)$/, "").split(/\r\n|\r|\n/).length;
/** Shaped like a key, not a name: one unbroken run of 40 or more characters with none of the
 *  marks of an address or a sentence. */
const keyLike = (text: string) => /^[^\s./:@]{40,}$/.test(text.trim());
/** 12 or 24 words, all from the Seed Phrase word list. */
const seedLike = (text: string, list: string[]) => {
  const words = text.trim().toLowerCase().split(/\s+/);
  return (words.length === 12 || words.length === 24) && words.every((w) => list.includes(w));
};
/** A date with single digits or slashes, written the way the core stores it: 2027/1/5 becomes
 *  2027-01-05. Anything else stays as typed. So do spaces alone: the core trims them, and a stored
 *  value sent back as it is means "keep". */
const fixExpiry = (value: string) => {
  const m = /^(\d{4})[-/](\d{1,2})[-/](\d{1,2})$/.exec(value.trim());
  const fixed = m && `${m[1]}-${m[2]!.padStart(2, "0")}-${m[3]!.padStart(2, "0")}`;
  return fixed && fixed !== value.trim() ? fixed : value;
};
/** The same rule as is_calendar_date in crates/notes/src/rules.rs. */
const isCalendarDate = (value: string) => {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return false;
  const year = Number(m[1]);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][Number(m[2]) - 1];
  return days !== undefined && Number(m[3]) >= 1 && Number(m[3]) <= days;
};
/** What crates/notes/src/rules.rs would refuse in a visible field, said before Save. Fixed text,
 *  never the value. It never stops a save: the core stays the authority. */
const visibleProblem = (f: string, value: string) => {
  const v = value.trim();
  if (!v) return null;
  if (f === "expires") return isCalendarDate(fixExpiry(v)) ? null : s.expiryProblem;
  if (f === "env_var") return /^[A-Za-z_][A-Za-z0-9_]*$/.test(v) && v.length <= 128 ? null : s.envVarProblem;
  if (f === "purpose") return [...v].length > 200 ? s.purposeProblem : null;
  return null;
};

/** What a duplicate's Editor starts from: a title and visible fields, never a Hidden Field. */
export interface Prefill {
  title: string;
  visible: Record<string, string>;
}

/** An Editor with unsaved changes, as App keeps it while the Sharing Guard shield covers the
 *  window, so that dismissing the shield brings it back. Memory only, gone on any lock. For a
 *  Wallet Kind `hidden` is empty and the words are never here (ADR-0004). */
export interface Draft {
  kind: Kind;
  /** The Note being edited, or null for a new Note. */
  id: string | null;
  title: string;
  /** Visible fields as the Editor holds them: the chain is a listed one or "other". */
  visible: Record<string, string>;
  chainName: string;
  /** What was typed into Hidden Fields. A pasted multi-line key is here as pasted, so its chip
   *  comes back with it. */
  hidden: Record<string, string>;
  clear: Record<string, boolean>;
  /** A Text: what was typed is to be added to the end of its text, not to replace it. */
  adding: boolean;
  wordCount: number;
  /** Wallet Kinds: the Hidden Fields that held something and were not kept. Their names only. */
  again: string[];
}

/** A shown Hidden Field value. It is dropped on any status whose reveal doesn't name this Note
 *  and field (expiry, lock, Sharing Guard, another Note), on window blur and on unmount. */
function useShown(noteId: string, status: AppStatus) {
  const [shown, setShown] = useState<{ field: string; value: string } | null>(null);
  const generation = useRef(0);
  // Whether the last status named a reveal of this Note.
  const revealed = useRef(false);
  // Whether a value came to this page. Set in keep(), not after a render, so a blur right after a
  // Show still ends it. It stays false for a Wallet Kind, whose values never enter the page: a
  // blur there is its own panel opening, and hideField would cancel that panel. Real focus loss
  // cancels native work in Rust.
  const held = useRef(false);
  useEffect(() => {
    const r = status.reveal;
    const mine = r?.note_id === noteId;
    // A reveal of this Note that the core has ended (expiry, lock, Sharing Guard, another Note)
    // makes any answer still on its way stale. A status that merely has no reveal must not: one
    // arrives every second, also between a Show request and its answer, and would cancel it.
    if (revealed.current && !mine) generation.current++;
    revealed.current = mine;
    setShown((v) => (v && r?.note_id === noteId && r.field === v.field ? v : null));
  }, [status.revision, status.reveal, noteId]);
  useEffect(() => {
    const drop = () => {
      generation.current++;
      setShown(null);
      if (held.current) {
        held.current = false;
        hideField().catch(() => {});
      }
    };
    window.addEventListener("blur", drop);
    return () => {
      window.removeEventListener("blur", drop);
      hideField().catch(() => {});
    };
  }, []);
  return {
    shown,
    // A value that arrives after the window lost focus is never shown.
    version: () => ++generation.current,
    keep: (field: string, value: string, version = generation.current) => {
      if (version === generation.current && document.hasFocus()) {
        held.current = true;
        setShown({ field, value });
        return true;
      }
      hideField().catch(() => {});
      return false;
    },
    hide: () => {
      held.current = false;
      generation.current++;
      setShown(null);
      hideField().catch(() => {});
    },
  };
}

/** A shown value. With `lines` (never a Wallet Kind), a value of several lines is drawn line by
 *  line, and each line that holds something has its own Copy button. The lines are numbered as
 *  Rust numbers them: from 0, split at every line feed, empty lines counted too. `lines.name`:
 *  the field's label, for what those buttons are called. */
function Value({ field, value, lines }: {
  field: string;
  value: string;
  lines?: { name: string; copy: (line: number) => void } | undefined;
}) {
  if (field === "words")
    return (
      <ol className="words">
        {value.split(" ").map((word, i) => (
          <li key={i}>
            <span>{i + 1}</span>
            {word}
          </li>
        ))}
      </ol>
    );
  const split = lines ? value.split("\n") : [];
  if (!lines || split.length < 2) return <div className="value">{value}</div>;
  return (
    <div className="value lines">
      {split.map((line, i) => {
        // As Rust takes the line: without the carriage return of a Windows line ending.
        const text = line.endsWith("\r") ? line.slice(0, -1) : line;
        return (
          <div key={i}>
            <span>{text}</span>
            {text && (
              <button className="plain" aria-label={s.copyLine(i + 1, lines.name)} onClick={() => lines.copy(i)}>
                <Icon name="copy" />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function Detail(
  props: {
    id: string;
    status: AppStatus;
    /** The Note no longer exists (it was open before a lock, say). Without this, the error shows. */
    onMissing?: () => void;
  } & (
    | {
        readOnly?: false;
        /** `field`: the Hidden Field the owner wants to replace; the Editor opens with the keyboard there. */
        onEdit: (note: NoteView, field?: string) => void;
        /** A new Note of the same Kind, started from this one's title and visible fields. */
        onDuplicate: (kind: Kind, seed: Prefill) => void;
        onChanged: () => void;
        /** `id`: the Note that went to Trash, so the result can offer to bring it back. */
        onTrashed: (id: string) => void;
      }
    /** A Note in Trash: Show and Copy under the usual rules, nothing that changes it. */
    | { readOnly: true }
  ),
) {
  const { id, status } = props;
  const [note, setNote] = useState<NoteView | null>(null);
  const { shown, keep, hide, version } = useShown(id, status);
  const [modal, setModal] = useState<{ field: string; step: "show" | "confirm-copy" | "copy" | "trash" } | null>(null);
  const { error, run } = useAction();
  // One Show request at a time: the answer to an earlier one would hide what a later one shows.
  // The field it is for, while it is on its way.
  const showing = useRef<string | null>(null);
  // Once a shown value is gone (Hide, the 30 seconds, a lock), its Show button gives up the
  // keyboard: left there, a stray Space or Enter later would show the value again. One button
  // keeps the keyboard: the Show of another field, pressed a moment ago and still waiting for its
  // answer. Pressing it is why the old value went (that one is dropped at once).
  const wasShown = useRef<string | null>(null);
  useEffect(() => {
    const was = wasShown.current;
    wasShown.current = shown?.field ?? null;
    const at = document.activeElement;
    if (!was || shown || !(at instanceof HTMLElement) || !at.classList.contains("toggle")) return;
    const pressed = showing.current;
    if (pressed === null || pressed === was || at.dataset.field !== pressed) at.blur();
  }, [shown]);
  useEffect(
    () =>
      void run(async () => {
        try {
          setNote(await getNote(id));
        } catch (e) {
          if (!props.onMissing || !isApiError(e) || e.code !== "not_found") throw e;
          props.onMissing();
        }
      }),
    [id],
  );
  // Cmd/Ctrl+Shift+C copies the first Hidden Field that has a value, as its Copy button does. Not
  // plain Cmd/Ctrl+C, and not under a dialog or while typing in a box. Two things are not that:
  // a radio or checkbox, and the Home search box, where the keyboard still is after Enter opened
  // this Note from it (Home marks it). The handler is set up once.
  const copyFirst = useRef(() => {});
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!cmdKey(e) || !e.shiftKey || e.repeat || e.key.toLowerCase() !== "c") return;
      const at = document.activeElement as HTMLElement | null;
      const typing = at?.matches("textarea, input:not([type=radio]):not([type=checkbox]):not([data-search])") || at?.isContentEditable;
      if (document.querySelector("[role=dialog]") || typing) return;
      e.preventDefault();
      copyFirst.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  if (!note) return <ErrorLine error={error} />;

  // No Favorite, Edit, Replace, Duplicate or Move to Trash for a Note in Trash, however it came to be shown.
  const live = props.readOnly || note.deleted_at != null ? null : props;
  const wallet = isWalletKind(note.kind);
  const expires = status.reveal?.expires_in_ms;
  const isShown = (field: string) => shown?.field === field && status.reveal?.note_id === id && status.reveal.field === field;
  const show = (field: string) => {
    if (wallet) return setModal({ field, step: "show" });
    if (showing.current !== null) return;
    showing.current = field;
    void run(async () => {
      const requestedAt = version();
      keep(field, await showField(id, field), requestedAt);
    }).finally(() => {
      showing.current = null;
    });
  };
  const copy = (field: string) =>
    wallet
      ? setModal({ field, step: field === "words" ? "confirm-copy" : "copy" })
      : run(() => copyField(id, field));
  const first = note.hidden.find(([, has]) => has)?.[0];
  copyFirst.current = () => {
    if (first) copy(first);
  };
  const duplicate = () =>
    live?.onDuplicate(note.kind, {
      title: s.duplicateTitle(note.title),
      visible: Object.fromEntries(Object.entries(note.visible).filter(([f]) => !NOT_DUPLICATED.includes(f))),
    });
  const toggleFavorite = () =>
    run(async () => {
      await setFavorite(id, !note.favorite);
      setNote({ ...note, favorite: !note.favorite });
      live?.onChanged();
    });

  return (
    <article className="stack">
      <header className="detail-head">
        <KindBadge kind={note.kind} size="large" />
        <div>
          <h1>{note.title}</h1>
          <p className="hint">
            {s.kindNames[note.kind]} · {s.created(formatDate(note.created_at))} ·{" "}
            {note.deleted_at != null ? s.deletedOn(formatDate(note.deleted_at)) : s.updated(formatDate(note.updated_at))}
          </p>
        </div>
        {live && (
          <>
            <button className="plain star" aria-pressed={note.favorite} aria-label={s.favorite} onClick={toggleFavorite}>
              <Icon name="star" />
            </button>
            <button onClick={() => live.onEdit(note)}>
              <Icon name="pencil" />
              {s.edit}
            </button>
          </>
        )}
      </header>
      <dl className="card fields rows">
        {FORM[note.kind]
          .filter((f) => !HIDDEN.includes(f) && note.visible[f])
          .map((f) => {
            const name = label(note.kind, f);
            const value = note.visible[f] ?? "";
            const due = f === "expires" ? expiry(value) : null;
            return (
              <div key={f}>
                <dt>{name}</dt>
                {NOT_COPIED.includes(f) ? (
                  <dd>{f === "chain" ? chainLabel(value) : value}</dd>
                ) : (
                  // Rust writes the clipboard and clears it, as for a Hidden Field. No Master
                  // Password, for a Wallet Kind either: this part of the Note is in plain sight.
                  <dd className="copyable">
                    <span>
                      {value}
                      {due && " · "}
                      {due && <span className={due.past ? "expiry past" : "expiry"}>{due.text}</span>}
                    </span>
                    <button className="small" aria-label={s.fieldAction(s.copy, name)} onClick={() => run(() => copyVisibleField(id, f))}>
                      <Icon name="copy" />
                      {s.copy}
                    </button>
                  </dd>
                )}
              </div>
            );
          })}
        {note.hidden.map(([f, has]) => {
          const name = label(note.kind, f);
          const on = isShown(f);
          // An API Key with a variable name: its key can also be copied as NAME=value.
          const envVar = note.kind === "api_key" && f === "key" ? note.visible.env_var : undefined;
          return (
            <div key={f}>
              <dt>{name}</dt>
              <dd className={has || live ? "secret" : undefined}>
                {/* The first line is the same shown or hidden, so no button moves; the value goes under it. */}
                {has ? <span className="dots">{!on && "••••••••••••"}</span> : <span className="hint">{s.empty}</span>}
                {(has || live) && (
                  <div className="actions">
                    {has && (
                      <>
                        {/* data-field: the field's name, for the rule above about which button keeps the keyboard. */}
                        <button className="small toggle" data-field={f} aria-label={s.fieldAction(on ? s.hide : s.show, name)} onClick={on ? hide : () => show(f)}>
                          <Icon name={on ? "eyeSlash" : "eye"} />
                          {on ? s.hide : s.show}
                        </button>
                        <button
                          className="small"
                          aria-label={s.fieldAction(s.copy, name)}
                          aria-keyshortcuts={f !== first ? undefined : mac ? "Meta+Shift+C" : "Control+Shift+C"}
                          onClick={() => copy(f)}
                        >
                          <Icon name="copy" />
                          {s.copy}
                          {f === first && <kbd aria-hidden="true">{s.shiftShortcut(mac, "C")}</kbd>}
                        </button>
                      </>
                    )}
                    {live && (
                      <button className="small" aria-label={s.fieldAction(has ? s.replace : s.add, name)} onClick={() => live.onEdit(note, f)}>
                        <Icon name="pencil" />
                        {has ? s.replace : s.add}
                      </button>
                    )}
                  </div>
                )}
                {/* Under Copy, on a line of its own: the name can be long. The line is built and
                    copied in Rust, and a refusal (a key that can't be written that way) shows as
                    it comes. */}
                {has && envVar && (
                  <div className="actions env">
                    <button className="small" onClick={() => run(() => copyFieldEnv(id))}>
                      <Icon name="copy" />
                      <span>{s.copyAsEnv(envVar)}</span>
                    </button>
                  </div>
                )}
                {on && (
                  <div className="shown">
                    <div className="actions">
                      {expires != null && <span className="hint">{s.hidesIn(seconds(expires))}</span>}
                      {/* Never a Wallet Kind: those are shown only in their own prompt (ADR-0004). */}
                      {/* A key held down must not keep the value on screen: repeats are ignored, and
                          once pressed the button gives up the keyboard. More time is asked of the
                          core; the value itself doesn't come again. */}
                      {!wallet && (
                        <button
                          className="small"
                          aria-label={s.resetTimerFor(name)}
                          disabled={status.reveal?.can_extend === false}
                          onKeyDown={(e) => e.repeat && e.preventDefault()}
                          onClick={(e) => {
                            e.currentTarget.blur();
                            run(extendReveal);
                          }}
                        >
                          {s.resetTimer}
                        </button>
                      )}
                    </div>
                    {/* A line is copied in Rust, by its number: the shown text is only what the
                        owner picks from. */}
                    <Value field={f} value={shown!.value} lines={wallet ? undefined : { name, copy: (line) => run(() => copyFieldLine(id, f, line)) }} />
                  </div>
                )}
              </dd>
            </div>
          );
        })}
      </dl>
      <ErrorLine error={error} />
      {live && (
        <div className="actions">
          <button onClick={duplicate}>
            <Icon name="duplicate" />
            {s.duplicate}
          </button>
          <button
            className="danger"
            // A Wallet Kind leaves only with the Master Password, like every other change to one.
            onClick={() => (wallet ? setModal({ field: "", step: "trash" }) : run(async () => { await trashNote(id); live.onTrashed(id); }))}
          >
            <Icon name="trash" />
            {s.moveToTrash}
          </button>
        </div>
      )}
      {modal?.step === "show" && (
        <WalletShow id={id} title={note.title} field={modal.field} name={label(note.kind, modal.field)} status={status} onClose={() => setModal(null)} />
      )}
      {modal?.step === "confirm-copy" && (
        <Confirm
          title={s.noteAction(s.copyTitle(label(note.kind, modal.field)), note.title)}
          text={s.copySeedText}
          confirmLabel={s.copyAnyway}
          onConfirm={() => setModal({ ...modal, step: "copy" })}
          onClose={() => setModal(null)}
        />
      )}
      {modal?.step === "copy" && (
        <NativeActionModal
          title={s.noteAction(s.copyTitle(label(note.kind, modal.field)), note.title)}
          text={s.copyText(WALLET_COPY_SECONDS)}
          onSubmit={() => copyField(id, modal.field)}
          onClose={() => setModal(null)}
          retryInMs={status.retry_in_ms}
        />
      )}
      {modal?.step === "trash" && live && (
        <NativeActionModal
          title={s.noteAction(s.moveToTrash, note.title)}
          text={s.walletTrashText}
          onSubmit={async () => {
            await trashNote(id);
            live.onTrashed(id);
          }}
          onClose={() => setModal(null)}
          retryInMs={status.retry_in_ms}
        />
      )}
    </article>
  );
}

/** Wallet authentication and readout stay together in native controls. */
function WalletShow(props: { id: string; title: string; field: string; name: string; status: AppStatus; onClose: () => void }) {
  const { busy, error, run } = useAction();
  const [confirmCopy, setConfirmCopy] = useState(false);
  const pending = useRef(false);
  const wait = props.status.retry_in_ms ? seconds(props.status.retry_in_ms) : 0;
  useEffect(() => () => {
    if (pending.current) void cancelNativeOperation().catch(() => {});
  }, []);
  const close = () => {
    if (pending.current) void cancelNativeOperation().catch(() => {});
    props.onClose();
  };
  const show = (mode: "held" | "accessible" | "copy") => {
    if (busy || wait > 0) return;
    pending.current = true;
    run(() => mode === "copy" ? showAndCopyWalletField(props.id, props.field) : showWalletField(props.id, props.field, mode))
      .then((ok) => {
        pending.current = false;
        if (ok) props.onClose();
      });
  };
  return (
    <>
      <Modal title={s.noteAction(s.showWalletTitle(props.name), props.title)} onClose={close}>
        <p className="hint">{s.holdHint}</p>
        <ErrorLine error={error} />
        {wait > 0 && <p className="hint">{s.tryAgainIn(wait)}</p>}
        <div className="actions">
          <button className="primary" disabled={busy || wait > 0} onClick={() => show("held")}>{s.holdToShow}</button>
          <button disabled={busy || wait > 0} onClick={() => show("accessible")}>{s.showFor20}</button>
          <button disabled={busy || wait > 0} onClick={() => props.field === "words" ? setConfirmCopy(true) : show("copy")}>{s.continueShowFor20AndCopy}</button>
          <button onClick={close}>{s.close}</button>
        </div>
      </Modal>
      {confirmCopy && <Confirm
        title={s.noteAction(s.fieldAction(s.showFor20AndCopy, props.name), props.title)}
        text={s.copySeedText}
        confirmLabel={s.copyAnyway}
        onConfirm={() => { setConfirmCopy(false); show("copy"); }}
        onClose={() => setConfirmCopy(false)}
      />}
    </>
  );
}

export function Editor(props: {
  kind: Kind;
  note?: NoteView;
  status: AppStatus;
  onSaved: (id: string) => void;
  /** A new Note was saved with "Save and add another": open a fresh Editor of the same Kind. */
  onSavedAndNew?: (() => void) | undefined;
  onCancel: () => void;
  /** Called on any edit, so leaving can ask first. */
  onDirty: () => void;
  /** The field that takes the keyboard instead of the title (Replace on a Hidden Field). */
  focusField?: string | undefined;
  /** A duplicate: what the title and visible fields start as. Still a new Note, not an edit. */
  seed?: Prefill | undefined;
  /** This Editor follows "Save and add another": it says that the Note before it was saved. */
  savedBefore?: boolean | undefined;
  /** What this Editor holds now, told on every change, for App to keep (see Draft). */
  onDraft: (draft: Draft) => void;
  /** The Sharing Guard shield covered this Editor: the draft it comes back with. */
  draft?: Draft | undefined;
}) {
  const { kind, note, seed, draft } = props;
  const wallet = isWalletKind(kind);
  const [title, setTitle] = useState(draft?.title ?? note?.title ?? seed?.title ?? "");
  // The chain as stored: a listed one, or any text. A new Note starts as EVM. A Note stored
  // without a chain is shown as Other, and saved as that.
  const from = note ?? seed;
  const stored = from ? (from.visible.chain ?? "") : "evm";
  const [visible, setVisible] = useState<Record<string, string>>(draft?.visible ?? { ...from?.visible, chain: PRESETS.includes(stored) ? stored : "other" });
  // The name of a chain that isn't listed. Left blank, the chain is saved as "other".
  const [chainName, setChainName] = useState(draft?.chainName ?? (PRESETS.includes(stored) || stored === "other" ? "" : stored));
  // Hidden Fields are replace-only: they start blank and blank means "keep".
  const [hidden, setHidden] = useState<Record<string, string>>(draft?.hidden ?? {});
  // Hidden Fields the owner ticked to remove: saved as empty, which clears them.
  const [clear, setClear] = useState<Record<string, boolean>>(draft?.clear ?? {});
  const [words, setWords] = useState<string[]>(() => Array(draft?.wordCount ?? Number(note?.visible.word_count ?? 12)).fill(""));
  // The Seed Phrase word list: for the word boxes, and to notice a phrase typed into a visible field.
  const [list, setList] = useState<string[]>([]);
  const [warning, setWarning] = useState<string | null>(null);
  const [addressIsKey, setAddressIsKey] = useState(false);
  const [pasteOverflow, setPasteOverflow] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  // Visible boxes that have had their say: the keyboard left them, or a save was tried.
  const [checked, setChecked] = useState<string[]>([]);
  // What a Move button last did: the box it emptied and where the value went.
  const [moved, setMoved] = useState<{ from: string; to: string } | null>(null);
  // The Hidden Field whose typed text is shown under its box. Not in Draft, on purpose.
  const [peek, setPeek] = useState<string | null>(null);
  const [step, setStep] = useState<"explainer" | "password" | null>(null);
  // Save and add another: what the save under way does once it has worked.
  const another = useRef(false);
  // The chip that stands in for the key box while it holds a pasted multi-line key. After Clear
  // the box comes back with the keyboard in it.
  const chips = useRef<Record<string, HTMLDivElement | null>>({});
  // The Title and visible boxes: a Move button puts the keyboard back in the box it emptied.
  const visibleBoxes = useRef<Record<string, HTMLInputElement | null>>({});
  const pastedField = useRef<string | null>(null);
  const [clearedField, setClearedField] = useState<string | null>(null);
  // "Saved" is filled in after the first render: a status line is read out when its text changes,
  // not when it arrives with the page.
  const [announced, setAnnounced] = useState(false);
  const keyLabel = useId();
  const { busy, error, run } = useAction();

  const filled = (f: string) => !!note?.hidden.some(([name, has]) => name === f && has);
  // A Text that has text in it can be added to instead of replaced: what is typed then goes after
  // what is there. Rust joins the two, so the text that is there never comes to this Editor.
  // That is how it starts, because nothing can be lost that way, unless Replace on the text is
  // what opened the Editor.
  const canAdd = kind === "text" && filled("body");
  const [adding, setAdding] = useState(draft ? draft.adding : canAdd && props.focusField !== "body");
  const add = canAdd && adding;
  const newer = NEWER[kind];
  const phrase = words.every((w) => w.trim()) ? words.map((w) => w.trim()).join(" ") : "";
  // Always one of the four the core knows: it refuses any other name.
  const chain = visible.chain as Chain;
  // A pasted multi-line key (PEM, SSH, PGP) is kept as pasted. No chain has a format for it.
  const multiline = lineCount(hidden.key ?? "") > 1;
  const privateKey = kind === "private_key" && !multiline ? (hidden.key ?? "") : "";
  // Only the listed chains have a key format an address can be compared with.
  const address = kind === "private_key" && chain !== "other" ? (visible.address ?? "").trim() : "";
  const wordsOk = kind !== "seed_phrase" || phrase !== "" || (!!note && words.every((w) => !w.trim()));
  // Whether a Service or Website can name a new Note. Not a value shaped like a key or a Seed
  // Phrase, and not before the word list that tells has arrived.
  const canTitle = (v: string) => !note && !!v && list.length > 0 && !keyLike(v) && !seedLike(v, list);
  const titleSource = () => {
    const f = TITLE_FROM[kind];
    const v = f ? (visible[f] ?? "").trim() : "";
    return canTitle(v) ? v : "";
  };
  // The title a save sends: a blank Title takes its name from the Service or Website.
  const finalTitle = title.trim() || titleSource();
  const titleField = note ? undefined : TITLE_FROM[kind];
  const missing = !finalTitle
    ? titleField && !(visible[titleField] ?? "").trim()
      ? s.addTitleOr(label(kind, titleField))
      : s.addTitle
    : wordsOk
      ? null
      : s.fillWords(words.length, !!note);
  const canSave = !busy && !missing && !pasteOverflow;
  const main = note ? undefined : MAIN[kind];
  // Whether a Hidden Field holds something typed.
  const typed = (f: string) => (f === "words" ? words.some(Boolean) : !!hidden[f]);
  // Hidden Fields of a Wallet Kind that the draft came back without and that are still blank.
  const again = (draft?.again ?? []).filter((f) => !typed(f) && !clear[f]);
  const more = MORE[kind] ?? [];
  // The fold can't close over something Save will write or remove, or the box that has the keyboard.
  const mustOpen = more.some((f) => visible[f]?.trim() || typed(f) || filled(f) || clear[f] || again.includes(f) || f === props.focusField || f === clearedField);
  // Only text typed here, on one line, in a box not ticked for removal. Never a stored value.
  const peekable =(f: string) => !!SHOWN_WHILE_TYPING[kind]?.includes(f) && !!hidden[f] && !clear[f] && lineCount(hidden[f]) < 2;

  useEffect(() => {
    // A duplicate, or a draft that came back, holds something unsaved from the start, so leaving
    // it asks first.
    if (seed || draft) props.onDirty();
    setAnnounced(true);
    seedWordlist().then(setList, () => {});
  }, []);

  // After the effect above, so a draft that came back is told upward again at once. What a
  // Wallet Kind's Hidden Fields hold never leaves this Editor (ADR-0004): only their names go.
  useEffect(() => {
    props.onDraft({
      kind,
      id: note?.id ?? null,
      title,
      visible,
      chainName,
      hidden: wallet ? {} : hidden,
      clear,
      adding,
      wordCount: words.length,
      again: wallet ? FORM[kind].filter((f) => HIDDEN.includes(f) && (typed(f) || again.includes(f))) : [],
    });
  }, [title, visible, chainName, hidden, clear, adding, words]);

  // The box a multi-line key was pasted into is gone, and the keyboard with it: the chip takes
  // it, so Tab and the save keys carry on from there.
  useEffect(() => {
    const field = pastedField.current;
    if (field) chips.current[field]?.focus();
    pastedField.current = null;
  }, [hidden.key, hidden.secret]);

  // Once open it stays open, so deleting the last character can't close it under the keyboard.
  useEffect(() => {
    if (mustOpen) setMoreOpen(true);
  }, [mustOpen]);

  // Typed text under its box goes on window blur and 30 seconds after the press. Its button then
  // gives up the keyboard: left there, a stray Space later would show the text again.
  useEffect(() => {
    if (!peek) return;
    const end = () => setPeek(null);
    const timer = setTimeout(end, SHOWN_WHILE_TYPING_MS);
    window.addEventListener("blur", end);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("blur", end);
      const at = document.activeElement;
      if (at instanceof HTMLElement && at.dataset.peek === peek) at.blur();
    };
  }, [peek]);
  useEffect(() => {
    if (peek && !peekable(peek)) setPeek(null);
  }, [peek, hidden, clear]);

  // The address is always visible. No hint from the key check means it is shaped like a key.
  useEffect(() => {
    if (!address) return setAddressIsKey(false);
    let live = true;
    const timer = setTimeout(() => checkPrivateKey(chain, address).then((hint) => live && setAddressIsKey(hint === null), () => {}), 300);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [address, chain]);

  // Warnings only; saving is never blocked by them.
  useEffect(() => {
    if (!phrase && !privateKey) return setWarning(null);
    let live = true;
    const timer = setTimeout(async () => {
      try {
        let text: string | null;
        if (phrase) {
          const check = await checkSeed(phrase);
          text = check.unknown_words.length
            ? s.unknownWords(check.unknown_words.map((i) => i + 1))
            : check.checksum_ok
              ? null
              : s.checksumFailed;
        } else text = await checkPrivateKey(chain, privateKey);
        if (live) setWarning(text);
      } catch {
        // A failed check just means no warning.
      }
    }, 300);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [phrase, privateKey, chain]);

  async function save() {
    if (note && add) {
      // Only what was typed is sent. The title goes first: saved a second time after a failure
      // it changes nothing, where text added a second time would be there twice.
      if (finalTitle !== note.title) await updateNote(note.id, { title: finalTitle, visible: {}, hidden: {} });
      if (hidden.body) await appendText(note.id, hidden.body);
      return props.onSaved(note.id);
    }
    const input = { title: finalTitle, visible: {} as Record<string, string>, hidden: {} as Record<string, string> };
    for (const f of FORM[kind]) {
      if (f === "words") {
        if (phrase) input.hidden.words = phrase;
      } else if (HIDDEN.includes(f)) {
        // Blank keeps what is stored; an empty value is how a Hidden Field is cleared.
        if (clear[f]) input.hidden[f] = "";
        else if (hidden[f]) input.hidden[f] = hidden[f];
      } else if (f === "chain") input.visible.chain = chain === "other" ? chainName.trim() || "other" : chain;
      // Here as well as on blur: Cmd/Ctrl+S and Enter never leave the box.
      else if (f === "expires") input.visible[f] = fixExpiry(visible[f] ?? "");
      else if (f !== "word_count") input.visible[f] = visible[f] ?? "";
    }
    if (note) await updateNote(note.id, input);
    const id = note ? note.id : await createNote({ kind, ...input });
    // The explainer counts as seen once a Seed Phrase is actually saved.
    if (kind === "seed_phrase" && !props.status.settings.seed_explainer_seen) markSeedExplainerSeen().catch(() => {});
    if (another.current && props.onSavedAndNew) props.onSavedAndNew();
    else props.onSaved(id);
  }

  // Wallet Kinds ask for the Master Password on every save, "Save and add another" (`again`) too.
  function begin(again: boolean) {
    setChecked((c) => c.concat(FORM[kind].filter((f) => visibleProblem(f, visible[f] ?? ""))));
    another.current = again;
    if (kind === "seed_phrase" && !props.status.settings.seed_explainer_seen) setStep("explainer");
    else if (isWalletKind(kind)) setStep("password");
    else run(() => save());
  }
  function submit(e: FormEvent) {
    e.preventDefault();
    begin(false);
  }
  // In a new Note whose main Hidden Field is still blank, Enter in a box goes on to the next box
  // instead of saving too early; after that it saves as usual. It never goes from a password box
  // to one that shows what is typed, so a key can't be carried into plain sight.
  const enterGoesOn = (e: KeyEvent<HTMLFormElement>) => {
    if (e.key !== "Enter" || e.metaKey || e.ctrlKey || e.defaultPrevented || e.nativeEvent.isComposing || e.keyCode === 229) return;
    const at = e.target;
    if (!(at instanceof HTMLInputElement) || !main || hidden[main]) return;
    e.preventDefault();
    if (e.repeat) return;
    const all = [...e.currentTarget.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input, textarea")];
    const next = all.slice(all.indexOf(at) + 1).find((b) => !b.disabled && b.offsetParent !== null && b.type !== "radio" && b.type !== "checkbox");
    if (next && (at.type !== "password" || next.type === "password")) next.focus();
  };
  // Cmd/Ctrl+S and Cmd/Ctrl+Enter save, as the Save button does (in a Text body Enter is a new
  // line). The keys do nothing else here, also while Save is off: never the browser's own Save.
  // Heard on the window for as long as the Editor is there, not on the form: the keyboard is not
  // always inside it (after a click on blank space, or on a checkbox or radio in WebKit). The
  // handler is set up once and reads what the Editor holds now through the refs.
  const saveNow = useRef(() => {});
  saveNow.current = () => {
    if (canSave) begin(false);
  };
  const cancelNow = useRef(() => {});
  cancelNow.current = props.onCancel;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Escape leaves the Editor: not a held key, an input method's, or one meant for a dialog or
      // the search box.
      if (e.key === "Escape") {
        if (e.repeat || e.isComposing || e.keyCode === 229) return;
        if (document.querySelector("[role=dialog]") || document.activeElement?.matches("[data-search]")) return;
        return cancelNow.current();
      }
      if (!cmdKey(e) || e.shiftKey || (e.key.toLowerCase() !== "s" && e.key !== "Enter")) return;
      if (document.querySelector("[role=dialog]")) return;
      e.preventDefault();
      if (!e.repeat) saveNow.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  // A pasted PEM, SSH or PGP key: a one-line box would drop its line breaks, so the text is kept
  // exactly as pasted and a chip stands in for the box.
  const pasteKey = (field: string, e: PasteEvent<HTMLInputElement>) => {
    const text = e.clipboardData.getData("text");
    if (!text.trim() || lineCount(text) < 2) return;
    e.preventDefault();
    pastedField.current = field;
    setHidden((h) => ({ ...h, [field]: text }));
    props.onDirty();
  };
  // A new API Key or Login with no title yet takes its Service or Website as the title, in the
  // title box, where it can still be changed. Not a value that looks like a key or a Seed Phrase.
  const titleFrom = (value: string) => {
    const v = value.trim();
    if (!title.trim() && canTitle(v)) setTitle(v);
  };
  // A paste-first slip puts a key or a Seed Phrase into a field that is always visible and
  // searchable. The line names the field, never what is in it. `to`: the Hidden Field a button can
  // move it to, where that is its obvious place and nothing there would be overwritten.
  const slip = (f: string, value: string): { text: string; to: string | null } | null => {
    const name = f === "title" ? s.title : label(kind, f);
    if (seedLike(value, list)) {
      // Never into a field that isn't a Wallet Kind's.
      if (kind !== "seed_phrase") return { text: s.looksLikeSeedManual(name), to: null };
      const free = (f === "title" || f === "wallet_name") && !filled("words") && words.every((w) => !w.trim());
      return { text: s.looksLikeSeed(name), to: free ? "words" : null };
    }
    if (!(f === "address" ? address && addressIsKey : keyLike(value))) return null;
    const target = MAIN[kind];
    if (!target) return { text: s.looksLikeKey(name), to: null };
    const free = ["title", "service", "username", "address"].includes(f) && !hidden[target] && !filled(target) && !clear[target];
    if (!free) return { text: s.looksLikeKeyManual(name, label(kind, target)), to: null };
    return { text: f === "address" ? s.looksLikePrivateKey(name) : s.looksLikeKey(name), to: target };
  };
  const move = (f: string, value: string, to: string) => {
    if (to === "words") {
      // Not through the grid's own paste: this sets the word count too, and is never too long.
      setWords(value.trim().toLowerCase().split(/\s+/));
      setPasteOverflow(false);
    } else setHidden({ ...hidden, [to]: value.trim() });
    if (f === "title") setTitle("");
    else setVisible({ ...visible, [f]: "" });
    setMoved({ from: f, to });
    props.onDirty();
    visibleBoxes.current[f]?.focus();
  };
  // Under a visible box, never inside its Field, which is a label: the Move button and what it did.
  // The status line is there before its text, so that the text is read out when it comes.
  const moveLine = (f: string, value: string, found: { to: string | null } | null) => {
    const to = found?.to;
    return (
      <>
        {to && (
          <button type="button" className="small" onClick={() => move(f, value, to)}>
            {to === "words" ? s.moveToWords : s.moveToBox(label(kind, to))}
          </button>
        )}
        {(to || moved?.from === f) && (
          <p className="ok" role="status">
            {moved?.from === f && (moved.to === "words" ? s.movedToWords(!finalTitle) : s.movedToBox(label(kind, moved.to), !finalTitle))}
          </p>
        )}
      </>
    );
  };
  const titleSlip = slip("title", title);

  const fieldBox = (f: string) => {
    const name = OPTIONAL.includes(f) ? `${label(kind, f)} ${s.optional}` : label(kind, f);
    if (f === "word_count")
      return (
        <div key={f} className="choice-row">
          <Choice
            label={name}
            options={[[12, s.words(12)], [24, s.words(24)]]}
            value={words.length}
            onChange={(n) => setWords((ws) => Array.from({ length: n }, (_, i) => ws[i] ?? ""))}
          />
          <button
            type="button"
            onClick={() => {
              setWords((ws) => ws.map(() => ""));
              setPasteOverflow(false);
              props.onDirty();
            }}
          >
            {s.clearAllWords}
          </button>
        </div>
      );
    if (f === "words") return <SeedGrid key={f} list={list} words={words} setWords={setWords} editing={!!note} autoFocus={props.focusField === f} onDirty={props.onDirty} onOverflow={() => setPasteOverflow(true)} onAccepted={() => setPasteOverflow(false)} />;
    if (f === "chain")
      return (
        <Fragment key={f}>
          <Choice label={name} options={CHAINS} value={chain} onChange={(c) => setVisible({ ...visible, chain: c })} />
          {chain === "other" && (
            <Field label={s.chainName} hint={s.chainNameHint}>
              <input
                value={chainName}
                maxLength={30}
                onChange={(e) => setChainName(e.target.value)}
                // An obvious name for a listed chain chooses that chain.
                onBlur={() => {
                  const listed = CHAIN_NAMES.get(chainName.trim().toLowerCase());
                  if (!listed) return;
                  setVisible({ ...visible, chain: listed });
                  setChainName("");
                }}
                {...noAssist}
              />
            </Field>
          )}
        </Fragment>
      );
    if (!HIDDEN.includes(f)) {
      const shownValue = visible[f] ?? "";
      const found = slip(f, shownValue);
      const problem = checked.includes(f) ? visibleProblem(f, shownValue) : null;
      return (
        // A plain box, not only the Field: moveLine goes beside the label, and the box stays the
        // same one when a button comes, so the input keeps the keyboard.
        <div key={f} className="field">
          <Field
            label={name}
            check={
              <>
                {found && <span className="warn">{found.text}</span>}
                {problem && <span className="warn">{problem}</span>}
              </>
            }
            hint={s.hints[kind][f]}
          >
            <input
              ref={(el) => {
                visibleBoxes.current[f] = el;
              }}
              value={shownValue}
              placeholder={f === "expires" ? s.expiryPlaceholder : undefined}
              onChange={(e) => {
                setVisible({ ...visible, [f]: e.target.value });
                if (moved?.from === f) setMoved(null);
              }}
              onBlur={() => {
                if (f === TITLE_FROM[kind]) titleFrom(shownValue);
                if (f === "expires" && fixExpiry(shownValue) !== shownValue) setVisible({ ...visible, [f]: fixExpiry(shownValue) });
                setChecked((c) => (c.includes(f) ? c : [...c, f]));
              }}
              {...noAssist}
            />
          </Field>
          {moveLine(f, shownValue, found)}
        </div>
      );
    }
    const value = hidden[f] ?? "";
    const had = filled(f);
    // The text of a Text, in Add mode: what is typed joins what is there, so there is
    // nothing to keep by leaving the box blank and nothing to remove.
    const adds = f === "body" && add;
    const edgeSpace = /^\s|\s$/.test(value);
    const inputProps = {
      value,
      // Only a field that has a value has one to keep.
      placeholder: adds ? s.typeToAdd : had && !clear[f] ? s.leaveBlankToKeep : "",
      disabled: !!clear[f] && !adds,
      onChange: (e: { target: { value: string } }) => setHidden({ ...hidden, [f]: e.target.value }),
      autoFocus: f === props.focusField || f === clearedField,
    };
    const peeking = peek === f && peekable(f);
    return (
      // Field is a label, and so is the checkbox: they sit one under the other in a plain box.
      <div key={f} className="field">
        {f === "body" && canAdd && (
          <Choice
            label={s.textMode}
            options={TEXT_MODES}
            value={add ? "add" : "replace"}
            onChange={(mode) => {
              setAdding(mode === "add");
              if (mode === "add") setClear({ ...clear, body: false });
            }}
          />
        )}
        {(f === "key" || f === "secret") && lineCount(value) > 1 ? (
          // A group, not a Field: a click on a label's text would press the button in it.
          <div className="field" role="group" aria-labelledby={`${keyLabel}-${f}`}>
            <span className="label" id={`${keyLabel}-${f}`}>
              {name}
            </span>
            <div className="chip" ref={(el) => { chips.current[f] = el; }} tabIndex={-1}>
              <span>{s.multilineKey(lineCount(value), [...value].length)}</span>
              <button
                type="button"
                className="small"
                aria-label={s.fieldAction(s.clear, name)}
                onClick={() => {
                  setHidden({ ...hidden, [f]: "" });
                  setClearedField(f);
                  props.onDirty();
                }}
              >
                {s.clear}
              </button>
            </div>
          </div>
        ) : (
          <Field
            label={name}
            hint={s.hints[kind][f]}
            // The length and the ends are all that is said about what was typed.
            check={
              (value || (adds && had)) && (
                <>
                  {/* The core trims a one-line key or secret, and keeps every other value as typed. */}
                  {value && (
                    <span className={edgeSpace ? "hint warn" : "hint"}>
                      {edgeSpace && (f === "key" || f === "secret") ? s.editorEdgeSpace([...value].length) : s.typedCount([...value].length, edgeSpace, f === "body")}
                    </span>
                  )}
                  {adds && had && <span className="hint">{s.addsText}</span>}
                  {value && f === "body" && had && !adds && <span className="warn">{s.replacesText}</span>}
                </>
              )
            }
          >
            {f === "body" ? (
              <textarea {...inputProps} {...noAssist} rows={10} />
            ) : (
              <SecretInput {...inputProps} onPaste={f === "key" || f === "secret" ? (e) => pasteKey(f, e) : undefined} />
            )}
          </Field>
        )}
        {/* The box stays a password box. What was typed is drawn apart from it, outside the label
            (its text would become the box's name), on a line that can't be selected and isn't
            read out by itself. A key held down is ignored. */}
        {peekable(f) && (
          <button
            type="button"
            className="small"
            data-peek={f}
            aria-label={s.fieldAction(peeking ? s.hide : s.show, label(kind, f))}
            onKeyDown={(e) => e.repeat && e.preventDefault()}
            onClick={() => setPeek(!peeking && document.hasFocus() ? f : null)}
          >
            <Icon name={peeking ? "eyeSlash" : "eye"} />
            {peeking ? s.hide : s.show}
          </button>
        )}
        {peeking && <Value field={f} value={value} />}
        {had && !adds && (
          <label className="check">
            <input
              type="checkbox"
              checked={!!clear[f]}
              onChange={(e) => {
                setClear({ ...clear, [f]: e.target.checked });
                // The box is off while this is ticked, so what was typed in it goes too.
                if (e.target.checked) setHidden({ ...hidden, [f]: "" });
              }}
            />
            {s.clearField(label(kind, f))}
          </label>
        )}
      </div>
    );
  };

  return (
    <>
      <form onSubmit={submit} onChange={props.onDirty} onKeyDown={enterGoesOn} className="stack">
        <header className="detail-head">
          <KindBadge kind={kind} size="large" />
          <div>
            <h1>{note ? s.editTitle(s.kindNames[kind]) : s.newTitle(s.kindNames[kind])}</h1>
            {props.savedBefore && (
              <p className="ok" role="status">
                {announced && s.savedAnother(s.kindNames[kind])}
              </p>
            )}
          </div>
        </header>
        <p className="hint">
          {s.draftLossHint(props.status.settings.idle_minutes, !!props.status.settings.count_activity_anywhere, props.status.settings.lock_on_app_switch, !wallet)}
          {wallet && ` ${s.draftLossWallet(FORM[kind].filter((f) => HIDDEN.includes(f)).map((f) => label(kind, f)))}`}
        </p>
        {/* Filled in after the first render, like "Saved" above, so that it is read out. */}
        {again.length > 0 && (
          <p className="warn" role="status">
            {announced && s.enterAgain(again.map((f) => label(kind, f)))}
          </p>
        )}
        <div className="card stack">
          <div className="field">
            <Field label={s.title} hint={note ? undefined : s.titleHint[kind]} check={titleSlip && <span className="warn">{titleSlip.text}</span>}>
              {/* Not when a draft comes back from under the Sharing Guard shield: the owner may have
                  been typing in a Hidden Field, and would carry on in this box, in plain sight. */}
              <input
                ref={(el) => {
                  visibleBoxes.current.title = el;
                }}
                value={title}
                onChange={(e) => {
                  setTitle(e.target.value);
                  if (moved?.from === "title") setMoved(null);
                }}
                autoFocus={!props.focusField && !draft}
                {...noAssist}
              />
            </Field>
            {moveLine("title", title, titleSlip)}
          </div>
          {FORM[kind].filter((f) => !more.includes(f)).map(fieldBox)}
          {more.length > 0 && (
            <details className="more-details" open={mustOpen || moreOpen}>
              {/* The state decides, not the browser, so the fold can't close over what mustOpen names. */}
              <summary
                onClick={(e) => {
                  e.preventDefault();
                  if (!mustOpen) setMoreOpen(!moreOpen);
                }}
              >
                {s.moreDetails(more.map((f) => label(kind, f)).join(", "))}
              </summary>
              <div className="stack">{more.map(fieldBox)}</div>
            </details>
          )}
          {/* What an older app can't open, once something is in it, and what is in plain sight:
              said once, by label. */}
          {newer?.some((f) => visible[f]?.trim() || hidden[f] || (filled(f) && !clear[f])) && <p className="hint">{s.needsCurrentApp(newer.map((f) => label(kind, f)))}</p>}
          <p className="hint">{s.notHidden(FORM[kind].filter((f) => HIDDEN.includes(f)).map((f) => label(kind, f)))}</p>
        </div>
        {/* Warnings and errors stay beside Save, whatever box the form is scrolled to. */}
        <div className="editor-footer">
          {warning && <p className="warn">{warning}</p>}
          {pasteOverflow && <p className="warn">{s.seedPasteTooMany}</p>}
          <ErrorLine error={error} />
          {missing && <p className="hint">{missing}</p>}
          {!missing && main && !hidden[main] && <p className="hint">{s.mainBlank(label(kind, main))}</p>}
          <div className="actions end">
            <button type="button" onClick={props.onCancel}>
              {s.cancel}
            </button>
            {!note && props.onSavedAndNew && (
              <button type="button" disabled={!canSave} onClick={() => begin(true)}>
                {s.saveAndAddAnother}
              </button>
            )}
            <button className="primary" disabled={!canSave} aria-keyshortcuts={mac ? "Meta+S" : "Control+S"}>
              {busy ? s.saving : s.save}
              <kbd aria-hidden="true">{s.shortcut(mac, "S")}</kbd>
            </button>
          </div>
        </div>
      </form>
      {step === "explainer" && (
        <Modal title={s.explainerTitle} onClose={() => setStep(null)}>
          <p>{s.explainerText}</p>
          <div className="actions">
            <button onClick={() => setStep(null)}>{s.goBack}</button>
            <button className="primary" onClick={() => setStep("password")}>
              {s.continue}
            </button>
          </div>
        </Modal>
      )}
      {step === "password" && (
        <NativeActionModal
          title={s.noteAction(note ? s.edit : s.add, note?.title ?? finalTitle)}
          text={s.savePasswordText}
          onSubmit={save}
          onClose={() => setStep(null)}
          retryInMs={props.status.retry_in_ms}
        />
      )}
    </>
  );
}

/** One password-type box per word (OS secure input). Nothing that is typed is echoed anywhere:
 *  there is no list of matching words (it would put the word on screen, and in front of anything
 *  that reads the page for accessibility). Enter still finishes a word once its start fits only
 *  one; when more fit it says so, never how many or which. A word that isn't in the list is named
 *  by its number alone, once the keyboard has left its box. */
function SeedGrid(props: {
  /** The Seed Phrase word list (empty until it has loaded). */
  list: string[];
  words: string[];
  setWords: (update: (words: string[]) => string[]) => void;
  editing: boolean;
  /** The first word box takes the keyboard (Replace on the words). */
  autoFocus: boolean;
  onDirty: () => void;
  onOverflow: () => void;
  onAccepted: () => void;
}) {
  const { list } = props;
  const [focus, setFocus] = useState<number | null>(null);
  // Enter was pressed and more than one word still fits.
  const [moreFits, setMoreFits] = useState(false);
  const boxes = useRef<(HTMLInputElement | null)[]>([]);
  const listId = useId();

  // The words the typed start could still become. Used for Enter only, never drawn.
  const prefix = focus === null ? "" : (props.words[focus] ?? "").trim().toLowerCase();
  const matches = prefix && !list.includes(prefix) ? list.filter((w) => w.startsWith(prefix)) : [];
  // The positions of words not in the list. Worked out on each render, so pasted words count, and
  // a box still being typed in never does.
  const bad = list.length ? props.words.flatMap((w, i) => (i !== focus && w.trim() && !list.includes(w.trim().toLowerCase()) ? [i + 1] : [])) : [];
  // Typing or pasting several words fills the following boxes and moves on; so does a trailing
  // space. A pasted 24-word phrase switches a 12-word grid to 24.
  const set = (i: number, value: string): void => {
    setMoreFits(false);
    let parts = value.trim().split(/\s+/);
    // A phrase pasted as a list ("1. word 2. word", "word, word"): the numbering and the mark
    // after a word go. One word being typed is never rewritten, and a word that merely isn't in
    // the word list stays as it is.
    if (parts.length > 1) {
      parts = parts.map((p) => p.replace(/^\d+[.)]/, "").replace(/[,;.]$/, "")).filter(Boolean);
      if (!parts.length) return set(i, "");
    }
    const end = i + parts.length;
    if (end > 24) {
      props.onOverflow();
      return;
    }
    props.onAccepted();
    props.onDirty();
    props.setWords((ws) =>
      Array.from({ length: end > ws.length ? 24 : ws.length }, (_, j) => (j >= i && j < end ? (parts[j - i] ?? "") : (ws[j] ?? ""))),
    );
    if (parts.length > 1 || /\s$/.test(value)) boxes.current[end]?.focus();
  };
  const pick = (word: string) => {
    if (focus === null) return;
    set(focus, word);
    boxes.current[focus + 1]?.focus();
  };

  return (
    // A group, not a fieldset: see Choice.
    <div className="seed" role="group" aria-labelledby={`${listId}-label`}>
      <span className="label" id={`${listId}-label`}>
        {label("seed_phrase", "words")}
      </span>
      {props.editing && <p className="hint">{s.leaveBlankToKeepWords}</p>}
      <ol className="words">
        {props.words.map((word, i) => (
          <li key={i}>
            <span>{i + 1}</span>
            <SecretInput
              ref={(el) => {
                boxes.current[i] = el;
              }}
              autoFocus={props.autoFocus && i === 0}
              aria-label={s.wordN(i + 1)}
              aria-invalid={bad.includes(i + 1) || undefined}
              value={word}
              onChange={(e) => set(i, e.target.value)}
              // Password inputs remove pasted line breaks before onChange. Read the original
              // clipboard text so a phrase copied as one word or numbered word per line works.
              onPaste={(e) => {
                const text = e.clipboardData.getData("text");
                // A single word still uses the input's usual insertion and selection rules.
                if (!/[\r\n]/.test(text) && text.trim().split(/\s+/).length < 2) return;
                e.preventDefault();
                set(i, text);
              }}
              onFocus={() => setFocus(i)}
              onBlur={() => {
                setFocus(null);
                setMoreFits(false);
              }}
              onKeyDown={(e) => {
                if (e.key !== "Enter" || cmdKey(e)) return; // (with Cmd/Ctrl it saves: see the Editor)
                e.preventDefault();
                // One possible word: finish it. A whole word, or one that isn't in the list: go on
                // to the next box. Several still possible: stay, say so, and keep typing.
                if (matches.length === 1) pick(matches[0]!);
                else if (!matches.length) boxes.current[i + 1]?.focus();
                else setMoreFits(true);
              }}
            />
          </li>
        ))}
      </ol>
      {/* The same words as the check of the whole phrase, which takes over beside Save once every
          box is filled. "You can still save" stays true: the list is English, and unusual phrases
          are allowed. */}
      {moreFits ? (
        <p className="warn">{s.moreWordsFit}</p>
      ) : (
        bad.length > 0 && props.words.some((w) => !w.trim()) && <p className="warn">{s.unknownWords(bad)}</p>
      )}
      {/* This one is always the same words, whatever has been typed. */}
      <p className="hint">{s.enterToFinish}</p>
    </div>
  );
}
