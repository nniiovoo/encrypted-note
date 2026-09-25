// One Note: the detail view (Show / Copy of Hidden Fields) and the editor for every Kind.

import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import {
  checkPrivateKey,
  checkSeed,
  copyField,
  createNote,
  getNote,
  hideField,
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
import { Choice, Confirm, ErrorLine, Field, formatDate, Modal, noAssist, PasswordModal, SecretInput, seconds, useAction } from "./components";
import { Icon, KindBadge } from "./icons";
import { strings as s } from "./strings";

// Field names as in notes::field, in form order.
const FORM: Record<Kind, string[]> = {
  seed_phrase: ["word_count", "words", "wallet_name", "passphrase"],
  private_key: ["chain", "address", "key"],
  login: ["website", "username", "password"],
  api_key: ["service", "key"],
  text: ["body"],
};
const HIDDEN = ["words", "passphrase", "key", "password", "body"];
const OPTIONAL = ["wallet_name", "passphrase"];
const CHAINS = Object.entries(s.chains) as [Chain, string][];
const label = (kind: Kind, field: string) => s.fields[kind][field] ?? field;

/** A shown Hidden Field value. It is dropped on any status whose reveal doesn't name this Note
 *  and field (expiry, lock, Sharing Guard, another Note), on window blur and on unmount. */
function useShown(noteId: string, status: AppStatus) {
  const [shown, setShown] = useState<{ field: string; value: string; at: number } | null>(null);
  useEffect(() => {
    const r = status.reveal;
    // A status fetched just before the value arrived can't name it yet: allow 1.5 s for that.
    setShown((v) => (v && (Date.now() - v.at < 1500 || (r?.note_id === noteId && r.field === v.field)) ? v : null));
  }, [status, noteId]);
  useEffect(() => {
    const drop = () => setShown(null);
    window.addEventListener("blur", drop);
    return () => {
      window.removeEventListener("blur", drop);
      hideField().catch(() => {});
    };
  }, []);
  return {
    shown,
    // A value that arrives after the window lost focus is never shown.
    keep: (field: string, value: string) =>
      document.hasFocus() ? setShown({ field, value, at: Date.now() }) : hideField().catch(() => {}),
    hide: () => {
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

export function Detail(props: {
  id: string;
  status: AppStatus;
  onEdit: (note: NoteView) => void;
  onChanged: () => void;
  onTrashed: () => void;
}) {
  const { id, status } = props;
  const [note, setNote] = useState<NoteView | null>(null);
  const { shown, keep, hide } = useShown(id, status);
  const [modal, setModal] = useState<{ field: string; step: "show" | "confirm-copy" | "copy" } | null>(null);
  const { error, run } = useAction();
  useEffect(() => void run(async () => setNote(await getNote(id))), [id]);
  if (!note) return <ErrorLine error={error} />;

  const wallet = isWalletKind(note.kind);
  const expires = status.reveal?.expires_in_ms;
  const show = (field: string) =>
    wallet ? setModal({ field, step: "show" }) : run(async () => keep(field, await showField(id, field)));
  const copy = (field: string) =>
    wallet
      ? setModal({ field, step: field === "words" ? "confirm-copy" : "copy" })
      : run(() => copyField(id, field));
  const toggleFavorite = () =>
    run(async () => {
      await setFavorite(id, !note.favorite);
      setNote({ ...note, favorite: !note.favorite });
      props.onChanged();
    });

  return (
    <article className="stack">
      <header className="detail-head">
        <KindBadge kind={note.kind} size="large" />
        <div>
          <h1>{note.title}</h1>
          <p className="hint">
            {s.kindNames[note.kind]} · {s.updated(formatDate(note.updated_at))}
          </p>
        </div>
        <button className="plain star" aria-pressed={note.favorite} aria-label={s.favorite} onClick={toggleFavorite}>
          <Icon name="star" />
        </button>
        <button onClick={() => props.onEdit(note)}>
          <Icon name="pencil" />
          {s.edit}
        </button>
      </header>
      <dl className="card fields rows">
        {FORM[note.kind]
          .filter((f) => !HIDDEN.includes(f) && note.visible[f])
          .map((f) => (
            <div key={f}>
              <dt>{label(note.kind, f)}</dt>
              <dd>{f === "chain" ? s.chains[note.visible[f] as Chain] ?? note.visible[f] : note.visible[f]}</dd>
            </div>
          ))}
        {note.hidden.map(([f, has]) => (
          <div key={f}>
            <dt>{label(note.kind, f)}</dt>
            <dd className={has ? "secret" : undefined}>
              {!has ? (
                <span className="hint">{s.empty}</span>
              ) : (
                <>
                  {shown?.field === f ? <Value field={f} value={shown.value} /> : <span className="dots">••••••••••••</span>}
                  <div className="actions">
                    {shown?.field === f && expires != null && <span className="hint">{s.hidesIn(seconds(expires))}</span>}
                    {shown?.field === f ? (
                      <button className="small" onClick={hide}>
                        <Icon name="eyeSlash" />
                        {s.hide}
                      </button>
                    ) : (
                      <button className="small" onClick={() => show(f)}>
                        <Icon name="eye" />
                        {s.show}
                      </button>
                    )}
                    <button className="small" onClick={() => copy(f)}>
                      <Icon name="copy" />
                      {s.copy}
                    </button>
                  </div>
                </>
              )}
            </dd>
          </div>
        ))}
      </dl>
      <ErrorLine error={error} />
      <div className="actions">
        <button className="danger" onClick={() => run(async () => { await trashNote(id); props.onTrashed(); })}>
          <Icon name="trash" />
          {s.moveToTrash}
        </button>
      </div>
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
        />
      )}
    </article>
  );
}

/** Wallet Kinds (ADR-0004): Master Password each time, visible only while held, or for 20 s. */
function WalletShow(props: { id: string; field: string; name: string; status: AppStatus; onClose: () => void }) {
  const { id, field } = props;
  const [password, setPassword] = useState("");
  const { shown, keep, hide } = useShown(id, props.status);
  const holding = useRef(false);
  const { busy, error, run } = useAction();
  const expires = props.status.reveal?.expires_in_ms;

  const show = (mode: "held" | "accessible") => {
    const p = password;
    setPassword("");
    run(async () => {
      const value = await showWalletField(id, field, p, mode);
      if (mode === "accessible" || holding.current) keep(field, value);
      else await hideField(); // let go before the password check finished
    }).then((ok) => {
      if (!ok) holding.current = false; // the button may be disabled before it's let go
    });
  };
  const hold = () => {
    if (!password || holding.current) return;
    holding.current = true;
    show("held");
  };
  const release = () => {
    if (!holding.current) return;
    holding.current = false;
    hide();
  };
  useEffect(() => {
    window.addEventListener("blur", release);
    return () => window.removeEventListener("blur", release);
  }, []);

  return (
    <Modal title={s.showWalletTitle(props.name)} onClose={props.onClose}>
      {shown ? <Value field={field} value={shown.value} /> : <p className="hint">{s.holdHint}</p>}
      {shown && expires != null && <p className="hint">{s.hidesIn(seconds(expires))}</p>}
      {/* Enter shows for 20 seconds. */}
      <form
        className="stack"
        onSubmit={(e) => {
          e.preventDefault();
          show("accessible");
        }}
      >
        <Field label={s.masterPassword}>
          <SecretInput value={password} onChange={(e) => setPassword(e.target.value)} autoFocus />
        </Field>
        <ErrorLine error={error} />
        <div className="actions">
          <button
            type="button"
            className="primary big"
            disabled={!password && !busy && !shown}
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
          <button disabled={busy || !password}>{s.showFor20}</button>
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
  onCancel: () => void;
  /** Called on any edit, so leaving can ask first. */
  onDirty: () => void;
}) {
  const { kind, note } = props;
  const [title, setTitle] = useState(note?.title ?? "");
  const [visible, setVisible] = useState<Record<string, string>>(note?.visible ?? { chain: "evm" });
  // Hidden Fields are replace-only: they start blank and blank means "keep".
  const [hidden, setHidden] = useState<Record<string, string>>({});
  const [words, setWords] = useState<string[]>(() => Array(Number(note?.visible.word_count ?? 12)).fill(""));
  const [warning, setWarning] = useState<string | null>(null);
  const [step, setStep] = useState<"explainer" | "password" | null>(null);
  const { busy, error, run } = useAction();

  const phrase = words.every((w) => w.trim()) ? words.map((w) => w.trim()).join(" ") : "";
  const privateKey = kind === "private_key" ? (hidden.key ?? "") : "";
  const chain = (visible.chain ?? "evm") as Chain;
  const wordsOk = kind !== "seed_phrase" || phrase !== "" || (!!note && words.every((w) => !w.trim()));
  const missing = !title.trim() ? s.addTitle : wordsOk ? null : s.fillWords(words.length, !!note);

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
        if (hidden[f]) input.hidden[f] = hidden[f];
      } else if (f !== "word_count") input.visible[f] = visible[f] ?? "";
    }
    if (note) await updateNote(note.id, input, password);
    const id = note ? note.id : await createNote({ kind, ...input }, password);
    // The explainer counts as seen once a Seed Phrase is actually saved.
    if (kind === "seed_phrase" && !props.status.settings.seed_explainer_seen) markSeedExplainerSeen().catch(() => {});
    props.onSaved(id);
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    if (kind === "seed_phrase" && !props.status.settings.seed_explainer_seen) setStep("explainer");
    else if (isWalletKind(kind)) setStep("password");
    else run(() => save());
  }

  return (
    <>
      <form onSubmit={submit} onChange={props.onDirty} className="stack">
        <header className="detail-head">
          <KindBadge kind={kind} size="large" />
          <div>
            <h1>{note ? s.editTitle(s.kindNames[kind]) : s.newTitle(s.kindNames[kind])}</h1>
          </div>
        </header>
        <div className="card stack">
          <Field label={s.title}>
            <input value={title} onChange={(e) => setTitle(e.target.value)} autoFocus {...noAssist} />
          </Field>
          {FORM[kind].map((f) => {
            const name = OPTIONAL.includes(f) ? `${label(kind, f)} ${s.optional}` : label(kind, f);
            if (f === "word_count")
              return (
                <Choice
                  key={f}
                  label={name}
                  options={[[12, s.words(12)], [24, s.words(24)]]}
                  value={words.length}
                  onChange={(n) => setWords((ws) => Array.from({ length: n }, (_, i) => ws[i] ?? ""))}
                />
              );
            if (f === "words") return <SeedGrid key={f} words={words} setWords={setWords} editing={!!note} />;
            if (f === "chain")
              return <Choice key={f} label={name} options={CHAINS} value={chain} onChange={(c) => setVisible({ ...visible, chain: c })} />;
            if (!HIDDEN.includes(f))
              return (
                <Field key={f} label={name}>
                  <input
                    value={visible[f] ?? ""}
                    onChange={(e) => setVisible({ ...visible, [f]: e.target.value })}
                    {...noAssist}
                  />
                </Field>
              );
            const inputProps = {
              value: hidden[f] ?? "",
              placeholder: note ? s.leaveBlankToKeep : "",
              onChange: (e: { target: { value: string } }) => setHidden({ ...hidden, [f]: e.target.value }),
            };
            return (
              <Field key={f} label={name} hint={f === "passphrase" ? s.passphraseTip : undefined}>
                {f === "body" ? <textarea {...inputProps} {...noAssist} rows={10} /> : <SecretInput {...inputProps} />}
              </Field>
            );
          })}
        </div>
        {warning && <p className="warn">{warning}</p>}
        <ErrorLine error={error} />
        {missing && <p className="hint">{missing}</p>}
        <div className="actions end">
          <button type="button" onClick={props.onCancel}>
            {s.cancel}
          </button>
          <button className="primary" disabled={busy || !!missing}>
            {busy ? s.saving : s.save}
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
        />
      )}
    </>
  );
}

/** One password-type box per word (OS secure input), with in-page BIP39 suggestions. */
function SeedGrid(props: { words: string[]; setWords: (update: (words: string[]) => string[]) => void; editing: boolean }) {
  const [list, setList] = useState<string[]>([]);
  const [focus, setFocus] = useState<number | null>(null);
  const [active, setActive] = useState(0);
  const boxes = useRef<(HTMLInputElement | null)[]>([]);
  const listId = useId();
  useEffect(() => void seedWordlist().then(setList, () => {}), []);

  // Suggestions are on screen, so they must not give the word away: none for a finished word,
  // and none once the typed start fits only one word (Enter still completes it).
  const prefix = focus === null ? "" : (props.words[focus] ?? "").trim().toLowerCase();
  const matches = prefix && !list.includes(prefix) ? list.filter((w) => w.startsWith(prefix)) : [];
  const suggestions = matches.length > 1 ? matches.slice(0, 6) : [];
  // Typing or pasting several words fills the following boxes and moves on; so does a trailing
  // space. A pasted 24-word phrase switches a 12-word grid to 24.
  const set = (i: number, value: string) => {
    const parts = value.trim().split(/\s+/);
    const end = i + parts.length;
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
              aria-label={s.wordN(i + 1)}
              aria-autocomplete="list"
              aria-controls={listId}
              aria-activedescendant={focus === i && suggestions.length ? `${listId}-${active}` : undefined}
              value={word}
              onChange={(e) => set(i, e.target.value)}
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
                } else if (e.key === "Enter") {
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
