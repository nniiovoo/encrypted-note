// One Note: the detail view (Show / Copy of Hidden Fields) and the editor for every Kind.

import { Fragment, useEffect, useId, useRef, useState, type ClipboardEvent as PasteEvent, type FormEvent, type KeyboardEvent as KeyEvent } from "react";
import {
  checkPrivateKey,
  checkSeed,
  copyField,
  createNote,
  getNote,
  hideField,
  isApiError,
  isWalletKind,
  markSeedExplainerSeen,
  seedWordlist,
  setFavorite,
  showField,
  showWalletField,
  trashNote,
  updateNote,
  type AppStatus,
  type Chain,
  type Kind,
  type NoteView,
} from "./api";
import { Choice, cmdKey, Confirm, ErrorLine, Field, formatDate, mac, Modal, noAssist, PasswordModal, SecretInput, seconds, useAction } from "./components";
import { Icon, KindBadge } from "./icons";
import { strings as s } from "./strings";

// Field names as in notes::field, in form order.
const FORM: Record<Kind, string[]> = {
  seed_phrase: ["word_count", "words", "wallet_name", "address", "derivation_path", "passphrase"],
  private_key: ["chain", "address", "key"],
  login: ["website", "username", "purpose", "password"],
  api_key: ["service", "key_id", "purpose", "url", "expires", "env_var", "key", "secret"],
  text: ["body"],
};
const HIDDEN = ["words", "passphrase", "key", "secret", "password", "body"];
const OPTIONAL = ["wallet_name", "passphrase", "address", "derivation_path", "username", "purpose", "key_id", "url", "expires", "env_var", "secret"];
// The Hidden Field a Note of the Kind is for. (A Seed Phrase can't be saved without its words.)
const MAIN: Partial<Record<Kind, string>> = { private_key: "key", login: "password", api_key: "key", text: "body" };
// The field a new Note's title is taken from when the title was left blank.
const TITLE_FROM: Partial<Record<Kind, string>> = { login: "website", api_key: "service" };
// Not carried into a duplicate: it is another wallet, with its own address and word count.
const NOT_DUPLICATED = ["address", "word_count"];
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
      hideField().catch(() => {});
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
        setShown({ field, value });
        return true;
      }
      hideField().catch(() => {});
      return false;
    },
    hide: () => {
      generation.current++;
      setShown(null);
      hideField().catch(() => {});
    },
  };
}

const Value = ({ field, value }: { field: string; value: string }) =>
  field === "words" ? (
    <ol className="words">
      {value.split(" ").map((word, i) => (
        <li key={i}>
          <span>{i + 1}</span>
          {word}
        </li>
      ))}
    </ol>
  ) : (
    <div className="value">{value}</div>
  );

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
        onTrashed: () => void;
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
  // Once a shown value is gone (Hide, the 30 seconds, a lock), its Show button gives up the
  // keyboard: left there, a stray Space or Enter later would show the value again.
  const wasShown = useRef(false);
  useEffect(() => {
    const at = document.activeElement;
    if (wasShown.current && !shown && at instanceof HTMLElement && at.classList.contains("toggle")) at.blur();
    wasShown.current = !!shown;
  }, [shown]);
  // One Show request at a time: the answer to an earlier one would hide what a later one shows.
  const showing = useRef(false);
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
  // plain Cmd/Ctrl+C, and not under a dialog or while typing in a box. The handler is set up once.
  const copyFirst = useRef(() => {});
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!cmdKey(e) || !e.shiftKey || e.repeat || e.key.toLowerCase() !== "c") return;
      const at = document.activeElement as HTMLElement | null;
      if (document.querySelector("[role=dialog]") || at?.matches("input, textarea") || at?.isContentEditable) return;
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
  // Also "Reset to 30 s" on a value already shown: the same request, under the same protections.
  const show = (field: string) => {
    if (wallet) return setModal({ field, step: "show" });
    if (showing.current) return;
    showing.current = true;
    void run(async () => {
      const requestedAt = version();
      keep(field, await showField(id, field), requestedAt);
    }).finally(() => {
      showing.current = false;
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
          .map((f) => (
            <div key={f}>
              <dt>{label(note.kind, f)}</dt>
              <dd>{f === "chain" ? chainLabel(note.visible[f] ?? "") : note.visible[f]}</dd>
            </div>
          ))}
        {note.hidden.map(([f, has]) => {
          const name = label(note.kind, f);
          const on = isShown(f);
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
                        <button className="small toggle" aria-label={s.fieldAction(on ? s.hide : s.show, name)} onClick={on ? hide : () => show(f)}>
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
                {on && (
                  <div className="shown">
                    <div className="actions">
                      {expires != null && <span className="hint">{s.hidesIn(seconds(expires))}</span>}
                      {/* Never a Wallet Kind: those are shown only in their own prompt (ADR-0004). */}
                      {!wallet && (
                        <button className="small" aria-label={s.resetTimerFor(name)} onClick={() => show(f)}>
                          {s.resetTimer}
                        </button>
                      )}
                    </div>
                    <Value field={f} value={shown!.value} />
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
            onClick={() => (wallet ? setModal({ field: "", step: "trash" }) : run(async () => { await trashNote(id); live.onTrashed(); }))}
          >
            <Icon name="trash" />
            {s.moveToTrash}
          </button>
        </div>
      )}
      {modal?.step === "show" && (
        <WalletShow id={id} field={modal.field} name={label(note.kind, modal.field)} status={status} onClose={() => setModal(null)} />
      )}
      {modal?.step === "confirm-copy" && (
        <Confirm
          title={s.copySeedTitle}
          text={s.copySeedText}
          confirmLabel={s.copyAnyway}
          onConfirm={() => setModal({ ...modal, step: "copy" })}
          onClose={() => setModal(null)}
        />
      )}
      {modal?.step === "copy" && (
        <PasswordModal
          title={s.copyTitle(label(note.kind, modal.field))}
          text={s.copyText}
          submitLabel={s.copy}
          onSubmit={(password) => copyField(id, modal.field, password)}
          onClose={() => setModal(null)}
          retryInMs={status.retry_in_ms}
        />
      )}
      {modal?.step === "trash" && live && (
        <PasswordModal
          title={s.moveToTrash}
          text={s.walletTrashText}
          submitLabel={s.moveToTrash}
          onSubmit={async (password) => {
            await trashNote(id, password);
            live.onTrashed();
          }}
          onClose={() => setModal(null)}
          retryInMs={status.retry_in_ms}
        />
      )}
    </article>
  );
}

/** Wallet Kinds (ADR-0004): Master Password each time, visible only while held, or for 20 s. */
function WalletShow(props: { id: string; field: string; name: string; status: AppStatus; onClose: () => void }) {
  const { id, field } = props;
  const [password, setPassword] = useState("");
  const { shown, keep, hide, version } = useShown(id, props.status);
  const holding = useRef(false);
  // The right password, but the button was let go before the check finished: said once idle.
  const [letGo, setLetGo] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const { busy, error, run } = useAction();
  const expires = props.status.reveal?.expires_in_ms;
  const isShown = shown && props.status.reveal?.note_id === id && props.status.reveal.field === field;
  // After three wrong passwords the core refuses every attempt for a while, the right one too.
  const wait = props.status.retry_in_ms ? seconds(props.status.retry_in_ms) : 0;

  const show = (mode: "held" | "accessible") => {
    if (wait > 0) return; // it would be thrown away unchecked: what was typed stays in the box
    const p = password;
    const requestedAt = version();
    setPassword("");
    setLetGo(false);
    run(async () => {
      const value = await showWalletField(id, field, p, mode);
      if (mode === "accessible" || holding.current) keep(field, value, requestedAt);
      else {
        // let go before the password check finished
        setLetGo(true);
        await hideField();
      }
    }).then((ok) => {
      if (!ok) holding.current = false; // the button may be disabled before it's let go
    });
  };
  const hold = () => {
    if (!password || holding.current || wait > 0) return;
    holding.current = true;
    show("held");
  };
  const release = () => {
    if (!holding.current) return;
    holding.current = false;
    hide();
  };
  useEffect(() => {
    // A release the button never hears of must still hide the value. On a Mac the key-up for
    // Space is not delivered while Cmd is down; any other key means the hold is over; and the
    // page can be hidden without the window losing focus.
    const onKey = (e: KeyboardEvent) => {
      if (holding.current && (e.metaKey || e.key !== " ")) release();
    };
    const onHidden = () => {
      if (document.hidden) release();
    };
    window.addEventListener("blur", release);
    window.addEventListener("keydown", onKey, true);
    document.addEventListener("visibilitychange", onHidden);
    return () => {
      window.removeEventListener("blur", release);
      window.removeEventListener("keydown", onKey, true);
      document.removeEventListener("visibilitychange", onHidden);
    };
  }, []);
  // When a reveal ends or a check fails, the button that had the keyboard is disabled again and
  // typing would go nowhere: the password box takes it back. The core can also end a hold itself,
  // and a disabled button never hears that it was let go.
  useEffect(() => {
    if (!busy && !shown) holding.current = false;
    const at = document.activeElement;
    if (!at || at === document.body || (at as HTMLButtonElement).disabled) input.current?.focus();
  }, [busy, !!shown]);

  return (
    <Modal title={s.showWalletTitle(props.name)} onClose={props.onClose}>
      {isShown ? <Value field={field} value={shown.value} /> : <p className="hint">{s.holdHint}</p>}
      {isShown && expires != null && <p className="hint">{s.hidesIn(seconds(expires))}</p>}
      {/* Enter shows for 20 seconds. */}
      <form
        className="stack"
        onSubmit={(e) => {
          e.preventDefault();
          show("accessible");
        }}
      >
        <Field label={s.masterPassword}>
          <SecretInput
            ref={input}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            // Space may still be held down from the button when the keyboard comes back here.
            onKeyDown={(e) => e.key === " " && e.repeat && e.preventDefault()}
            autoFocus
          />
        </Field>
        <ErrorLine error={error} />
        {wait > 0 && <p className="hint">{s.tryAgainIn(wait)}</p>}
        {letGo && !busy && !shown && (
          <p className="hint" role="status">
            {s.keepHolding}
          </p>
        )}
        <div className="actions">
          <button
            type="button"
            className="primary big"
            disabled={!busy && !shown && (!password || wait > 0)}
            onPointerDown={(e) => {
              e.currentTarget.setPointerCapture(e.pointerId);
              hold();
            }}
            onPointerUp={release}
            onPointerCancel={release}
            onKeyDown={(e) => {
              if (e.key !== " ") return;
              e.preventDefault();
              if (!e.repeat) hold();
            }}
            onKeyUp={(e) => e.key === " " && release()}
            onBlur={release}
          >
            {busy ? s.checking : s.holdToShow}
          </button>
          <button disabled={busy || !password || wait > 0}>{s.showFor20}</button>
          <button type="button" onClick={props.onClose}>
            {s.close}
          </button>
        </div>
      </form>
    </Modal>
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
  const [step, setStep] = useState<"explainer" | "password" | null>(null);
  // Save and add another: what the save under way does once it has worked.
  const another = useRef(false);
  // The chip that stands in for the key box while it holds a pasted multi-line key. After Clear
  // the box comes back with the keyboard in it.
  const chips = useRef<Record<string, HTMLDivElement | null>>({});
  const pastedField = useRef<string | null>(null);
  const [clearedField, setClearedField] = useState<string | null>(null);
  // "Saved" is filled in after the first render: a status line is read out when its text changes,
  // not when it arrives with the page.
  const [announced, setAnnounced] = useState(false);
  const keyLabel = useId();
  const { busy, error, run } = useAction();

  const filled = (f: string) => !!note?.hidden.some(([name, has]) => name === f && has);
  const phrase = words.every((w) => w.trim()) ? words.map((w) => w.trim()).join(" ") : "";
  // Always one of the four the core knows: it refuses any other name.
  const chain = visible.chain as Chain;
  // A pasted multi-line key (PEM, SSH, PGP) is kept as pasted. No chain has a format for it.
  const multiline = lineCount(hidden.key ?? "") > 1;
  const privateKey = kind === "private_key" && !multiline ? (hidden.key ?? "") : "";
  // Only the listed chains have a key format an address can be compared with.
  const address = kind === "private_key" && chain !== "other" ? (visible.address ?? "").trim() : "";
  const wordsOk = kind !== "seed_phrase" || phrase !== "" || (!!note && words.every((w) => !w.trim()));
  const missing = !title.trim() ? s.addTitle : wordsOk ? null : s.fillWords(words.length, !!note);
  const canSave = !busy && !missing && !pasteOverflow;
  const main = note ? undefined : MAIN[kind];
  // Whether a Hidden Field holds something typed.
  const typed = (f: string) => (f === "words" ? words.some(Boolean) : !!hidden[f]);
  // Hidden Fields of a Wallet Kind that the draft came back without and that are still blank.
  const again = (draft?.again ?? []).filter((f) => !typed(f) && !clear[f]);

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
      wordCount: words.length,
      again: wallet ? FORM[kind].filter((f) => HIDDEN.includes(f) && (typed(f) || again.includes(f))) : [],
    });
  }, [title, visible, chainName, hidden, clear, words]);

  // The box a multi-line key was pasted into is gone, and the keyboard with it: the chip takes
  // it, so Tab and the save keys carry on from there.
  useEffect(() => {
    const field = pastedField.current;
    if (field) chips.current[field]?.focus();
    pastedField.current = null;
  }, [hidden.key, hidden.secret]);

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

  async function save(password?: string) {
    const input = { title, visible: {} as Record<string, string>, hidden: {} as Record<string, string> };
    for (const f of FORM[kind]) {
      if (f === "words") {
        if (phrase) input.hidden.words = phrase;
      } else if (HIDDEN.includes(f)) {
        // Blank keeps what is stored; an empty value is how a Hidden Field is cleared.
        if (clear[f]) input.hidden[f] = "";
        else if (hidden[f]) input.hidden[f] = hidden[f];
      } else if (f === "chain") input.visible.chain = chain === "other" ? chainName.trim() || "other" : chain;
      else if (f !== "word_count") input.visible[f] = visible[f] ?? "";
    }
    if (note) await updateNote(note.id, input, password);
    const id = note ? note.id : await createNote({ kind, ...input }, password);
    // The explainer counts as seen once a Seed Phrase is actually saved.
    if (kind === "seed_phrase" && !props.status.settings.seed_explainer_seen) markSeedExplainerSeen().catch(() => {});
    if (another.current && props.onSavedAndNew) props.onSavedAndNew();
    else props.onSaved(id);
  }

  // Wallet Kinds ask for the Master Password on every save, "Save and add another" (`again`) too.
  function begin(again: boolean) {
    another.current = again;
    if (kind === "seed_phrase" && !props.status.settings.seed_explainer_seen) setStep("explainer");
    else if (isWalletKind(kind)) setStep("password");
    else run(() => save());
  }
  function submit(e: FormEvent) {
    e.preventDefault();
    begin(false);
  }
  // Cmd/Ctrl+S and Cmd/Ctrl+Enter save, as the Save button does (in a Text body Enter is a new
  // line). The keys do nothing else here, also while Save is off: never the browser's own Save.
  const onKey = (e: KeyEvent<HTMLFormElement>) => {
    if (!cmdKey(e) || e.shiftKey || (e.key.toLowerCase() !== "s" && e.key !== "Enter")) return;
    if (document.querySelector("[role=dialog]")) return;
    e.preventDefault();
    if (canSave && !e.repeat) e.currentTarget.requestSubmit();
  };
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
    if (!note && !title.trim() && v && !keyLike(v) && !seedLike(v, list)) setTitle(v);
  };
  // A paste-first slip puts a key or a Seed Phrase into a field that is always visible and
  // searchable. The line names the field, never what is in it.
  const slip = (f: string, value: string) => {
    const name = f === "title" ? s.title : label(kind, f);
    const text =
      (f === "title" || f === "wallet_name") && seedLike(value, list)
        ? s.looksLikeSeed(name)
        : (f === "title" || f === "service" || f === "username") && keyLike(value)
          ? s.looksLikeKey(name)
          : f === "address" && address && addressIsKey
            ? s.looksLikePrivateKey(name)
            : null;
    return text && <span className="warn">{text}</span>;
  };

  return (
    <>
      <form onSubmit={submit} onChange={props.onDirty} onKeyDown={onKey} className="stack">
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
          {s.draftLossHint}
          {wallet && ` ${s.draftLossWallet(FORM[kind].filter((f) => HIDDEN.includes(f)).map((f) => label(kind, f)))}`}
        </p>
        {/* Filled in after the first render, like "Saved" above, so that it is read out. */}
        {again.length > 0 && (
          <p className="warn" role="status">
            {announced && s.enterAgain(again.map((f) => label(kind, f)))}
          </p>
        )}
        <div className="card stack">
          <Field label={s.title} check={slip("title", title)}>
            <input value={title} onChange={(e) => setTitle(e.target.value)} autoFocus={!props.focusField} {...noAssist} />
          </Field>
          {FORM[kind].map((f) => {
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
            if (!HIDDEN.includes(f))
              return (
                <Field key={f} label={name} check={slip(f, visible[f] ?? "")} hint={f === "expires" ? s.expiryDateHint : f === "env_var" ? s.variableNameHint : undefined}>
                  <input
                    value={visible[f] ?? ""}
                    onChange={(e) => setVisible({ ...visible, [f]: e.target.value })}
                    onBlur={f === TITLE_FROM[kind] ? () => titleFrom(visible[f] ?? "") : undefined}
                    {...noAssist}
                  />
                </Field>
              );
            const value = hidden[f] ?? "";
            const had = filled(f);
            const edgeSpace = /^\s|\s$/.test(value);
            const inputProps = {
              value,
              // Only a field that has a value has one to keep.
              placeholder: had && !clear[f] ? s.leaveBlankToKeep : "",
              disabled: !!clear[f],
              onChange: (e: { target: { value: string } }) => setHidden({ ...hidden, [f]: e.target.value }),
              autoFocus: f === props.focusField || f === clearedField,
            };
            return (
              // Field is a label, and so is the checkbox: they sit one under the other in a plain box.
              <div key={f} className="field">
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
                    hint={f === "passphrase" ? s.passphraseTip : undefined}
                    // The length and the ends are all that is said about what was typed.
                    check={
                      value && (
                        <>
                          <span className={edgeSpace ? "hint warn" : "hint"}>{s.typedCount([...value].length, edgeSpace, f === "body")}</span>
                          {f === "body" && had && <span className="warn">{s.replacesText}</span>}
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
                {had && (
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
          })}
        </div>
        {warning && <p className="warn">{warning}</p>}
        {pasteOverflow && <p className="warn">{s.seedPasteTooMany}</p>}
        <ErrorLine error={error} />
        {missing && <p className="hint">{missing}</p>}
        {!missing && main && !hidden[main] && <p className="warn">{s.mainBlank(label(kind, main))}</p>}
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
      </form>
      {step === "explainer" && (
        <Modal title={s.explainerTitle} onClose={() => setStep(null)}>
          <p>{s.explainerText}</p>
          <div className="actions">
            <button onClick={() => setStep("password")}>{s.saveAnyway}</button>
            <button className="primary" onClick={() => setStep("password")}>
              {s.haveCopy}
            </button>
          </div>
        </Modal>
      )}
      {step === "password" && (
        <PasswordModal
          title={s.savePasswordTitle}
          text={s.savePasswordText}
          submitLabel={s.save}
          onSubmit={save}
          onClose={() => setStep(null)}
          retryInMs={props.status.retry_in_ms}
        />
      )}
    </>
  );
}

/** One password-type box per word (OS secure input), with in-page BIP39 suggestions. */
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
  const [active, setActive] = useState(0);
  const boxes = useRef<(HTMLInputElement | null)[]>([]);
  const listId = useId();

  // Suggestions are on screen, so they must not give the word away: none for a finished word,
  // and none once the typed start fits only one word (Enter still completes it).
  const prefix = focus === null ? "" : (props.words[focus] ?? "").trim().toLowerCase();
  const matches = prefix && !list.includes(prefix) ? list.filter((w) => w.startsWith(prefix)) : [];
  const suggestions = matches.length > 1 ? matches.slice(0, 6) : [];
  // Typing or pasting several words fills the following boxes and moves on; so does a trailing
  // space. A pasted 24-word phrase switches a 12-word grid to 24.
  const set = (i: number, value: string): void => {
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
    setActive(0);
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
              aria-autocomplete="list"
              aria-controls={listId}
              aria-activedescendant={focus === i && suggestions.length ? `${listId}-${active}` : undefined}
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
              onFocus={() => {
                setFocus(i);
                setActive(0);
              }}
              onBlur={() => setFocus(null)}
              onKeyDown={(e) => {
                if ((e.key === "ArrowDown" || e.key === "ArrowUp") && suggestions.length) {
                  e.preventDefault();
                  const step = e.key === "ArrowDown" ? 1 : suggestions.length - 1;
                  setActive((a) => (a + step) % suggestions.length);
                } else if (e.key === "Enter" && !cmdKey(e)) {
                  // (With Cmd/Ctrl it saves: see the Editor.)
                  e.preventDefault();
                  const next = suggestions[active] ?? matches[0];
                  if (next) pick(next);
                  else boxes.current[i + 1]?.focus();
                }
              }}
            />
          </li>
        ))}
      </ol>
      {matches.length === 1 ? (
        <p className="suggestions hint">{s.enterToFinish}</p>
      ) : (
        <div className="suggestions" role="listbox" id={listId} aria-label={s.suggestions}>
          {suggestions.map((w, k) => (
            <button
              type="button"
              role="option"
              id={`${listId}-${k}`}
              aria-selected={k === active}
              tabIndex={-1}
              key={w}
              onPointerDown={(e) => e.preventDefault()}
              onClick={() => pick(w)}
            >
              {w}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
