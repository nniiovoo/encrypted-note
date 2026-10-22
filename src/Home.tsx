// The Unlocked Vault: sidebar, search, list, detail pane, Trash, status line.

import { Fragment, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent as KeyEvent } from "react";
import {
  cancelNativeOperation,
  copyField,
  deleteForever,
  emptyTrash,
  getNote,
  isWalletKind,
  isApiError,
  KINDS,
  listNotes,
  restoreNote,
  type AppStatus,
  type Filter,
  type Kind,
  type NoteSummary,
  type NoteView,
} from "./api";
import { Choice, cmdKey, Confirm, daysUntil, dragRegion, ErrorLine, expiry, formatDate, mac, minSec, noAssist, NativeActionModal, seconds, TitlebarDrag, useAction } from "./components";
import { Icon, KIND_ICONS, KindBadge, type IconName } from "./icons";
import { Detail, Editor, type Draft, type Prefill } from "./Note";
import { Settings } from "./Settings";
import { strings as s } from "./strings";

type Nav = Filter | { type: "settings" };
const ALL: Nav = { type: "all" };
const FAVORITES: Nav = { type: "favorites" };
const SETTINGS: Nav = { type: "settings" };
const TRASH: Nav = { type: "trash" };
const BY_KIND = KINDS.map((kind): Nav => ({ type: "kind", kind }));
const API_KEYS: Filter = { type: "kind", kind: "api_key" };
// The banner speaks up once an API Key's expiry date is this near, or past.
const EXPIRY_BANNER_DAYS = 14;
const withinExpiryWarning = (note: NoteSummary) => {
  const days = note.expires ? daysUntil(note.expires) : null;
  return days !== null && days <= EXPIRY_BANNER_DAYS;
};
// Written by its number, so that it can be seen in this file.
const NO_BREAK_SPACE = String.fromCharCode(0xa0);
// By value: the place App hands back is not one of the objects above.
const sameNav = (a: Nav, b: Nav) => (a.type === "kind" ? b.type === "kind" && a.kind === b.kind : a.type === b.type);
/** All, Favorites and the Kinds: the views with the search box and the two columns. */
const listed = (n: Nav) => n.type !== "settings" && n.type !== "trash";
const navLabel = (n: Nav) => (n.type === "kind" ? s.kindPlurals[n.kind] : s.nav[n.type]);
const NAV_ICONS: Record<Exclude<Nav["type"], "kind">, IconName> = { all: "all", favorites: "star", trash: "trash", settings: "settings" };
const navIcon = (n: Nav) => (n.type === "kind" ? KIND_ICONS[n.kind] : NAV_ICONS[n.type]);

type Pane =
  | { type: "pick" }
  | { type: "note"; id: string }
  // `field`: Replace on a Hidden Field, which takes the keyboard in the Editor. `draft`, here and
  // in "new": what the Editor held when the Sharing Guard shield covered it.
  | { type: "edit"; note: NoteView; field?: string | undefined; draft?: Draft }
  // `seed`: Duplicate, the title and visible fields the Editor starts with. `savedBefore`: it
  // follows "Save and add another", and says so.
  | { type: "new"; kind: Kind; seed?: Prefill; savedBefore?: boolean; draft?: Draft }
  // The draft of an edit is coming back: its Note is being fetched, and nothing is drawn yet.
  | { type: "restoring"; id: string; draft: Draft }
  | { type: "trashed" }
  | null;

type Sort = "recent" | "az";
const SORTS = [["recent", s.sortRecent], ["az", s.sortAZ]] as const;
const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });
/** The rows in the order shown. Recent is the order Rust returns. A to Z keeps Favorites first;
 *  with `byService` (API Keys) it sorts by service before that, Notes without a service last. */
function ordered(notes: NoteSummary[], sort: Sort, byService: boolean) {
  if (sort === "recent") return notes;
  const service = (a: NoteSummary, b: NoteSummary) =>
    Number(!a.subtitle) - Number(!b.subtitle) || collator.compare(a.subtitle ?? "", b.subtitle ?? "");
  return [...notes].sort(
    (a, b) => (byService ? service(a, b) : 0) || Number(b.favorite) - Number(a.favorite) || collator.compare(a.title, b.title),
  );
}

// The one Hidden Field that can be copied straight from a list row: the Kind's first, the one
// NoteSummary.has_hidden is about. Never a Wallet Kind: those ask for the Master Password every
// time (ADR-0004).
const ROW_COPY: Partial<Record<Kind, string>> = { login: "password", api_key: "key" };

/** Where the owner is in Home. App keeps it across a lock and the Shield: a filter, a Note id and
 *  the sort choice, never a title, the search text or a draft. */
export interface Place {
  nav: Nav;
  noteId: string | null;
  sort: Sort;
}
export const START: Place = { nav: ALL, noteId: null, sort: "recent" };

export function Home({ status, place, onPlace, readDraft, onDraft, onLock, onOperationOutcome }: {
  status: AppStatus;
  /** Where to start, and how to tell App where the owner is now. */
  place: Place;
  onPlace: (place: Place) => void;
  /** An Editor with unsaved changes that the Sharing Guard shield covered: it opens again as it
   *  was. Called once, at the start, and never kept as a prop: App's copy is then the only one
   *  outside the pane it opens, and a lock empties it. `onDraft` tells App what the Editor holds
   *  now, or null once that is saved or left. */
  readDraft: () => Draft | null;
  onDraft: (draft: Draft | null) => void;
  /** The Lock button. */
  onLock: () => void;
  onOperationOutcome: (message: string) => void;
}) {
  const [nav, setNav] = useState(place.nav);
  // For the key handler below, which is set up only once.
  const navNow = useRef(nav);
  navNow.current = nav;
  const [query, setQuery] = useState("");
  const [expiryOnly, setExpiryOnly] = useState(false);
  const [sort, setSort] = useState(place.sort);
  // Undefined until this nav's list arrives, so nothing (not "empty") shows while loading. The
  // query it answers is kept with it, so Enter in the search box never opens an older search's row.
  const [list, setList] = useState<{ nav: Nav; query: string; notes: NoteSummary[] }>();
  const notes = list && sameNav(list.nav, nav) ? (expiryOnly ? list.notes.filter(withinExpiryWarning) : list.notes) : undefined;
  const byService = sort === "az" && nav.type === "kind" && nav.kind === "api_key";
  const shown = useMemo(() => (notes ? ordered(notes, sort, byService) : []), [notes, sort, byService]);
  // The one place the draft is read: from here on it lives in the pane it opens.
  const [pane, setPane] = useState<Pane>(() => {
    const draft = readDraft();
    return draft
      ? draft.id
        ? { type: "restoring", id: draft.id, draft }
        : { type: "new", kind: draft.kind, draft }
      : place.noteId
        ? { type: "note", id: place.noteId }
        : null;
  });
  // For the answer to the fetch below, which arrives later.
  const paneNow = useRef(pane);
  paneNow.current = pane;
  // Counts panes opened: a second New <Kind> in a row must be a fresh Editor, not the old draft.
  const [opened, setOpened] = useState(0);
  const [version, setVersion] = useState(0);
  // An Editor with unsaved changes asks before it's replaced. A draft that came back is one: on
  // the first render, the only one this value is taken from, `pane` is what the draft made it.
  const dirty = useRef(pane?.type === "restoring" || (pane?.type === "new" && !!pane.draft));
  const [leaving, setLeaving] = useState<(() => void) | null>(null);
  const search = useRef<HTMLInputElement>(null);
  const firstTile = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const rowId = (id: string) => `${listId}-${id}`;
  // The row the arrow keys are on while the search box has the keyboard (not the open row).
  const [marked, setMarked] = useState<string | null>(null);
  // A failed Copy from a list row. The next action clears it.
  const [copyError, setCopyError] = useState<string | null>(null);
  // Every API Key, for the expiry banner: fetched apart from the list on screen, which a filter
  // or a search narrows.
  const [apiKeys, setApiKeys] = useState<NoteSummary[]>([]);
  // The seconds a new copy has before the clipboard is cleared, as the status first gave them:
  // what is read out. `n` counts the copies, so that a second one is read out too.
  const [copied, setCopied] = useState<{ seconds: number; n: number; source: string | undefined } | null>(null);
  // Starts as the count now: a copy made before this screen appeared is not a new one.
  const copiesWas = useRef(status.copies);
  const changed = () => setVersion((v) => v + 1);
  const open = (next: Pane) => {
    void cancelNativeOperation().catch(() => {});
    dirty.current = false;
    // Whatever Editor was open is saved, cancelled or discarded by now: App lets go of its draft.
    onDraft(null);
    setCopyError(null);
    setOpened((n) => n + 1);
    setPane(next);
  };
  const onDirty = () => {
    dirty.current = true;
  };
  // What the Editor holds goes up to App only once something in it has changed.
  const report = (d: Draft) => {
    if (dirty.current) onDraft(d);
  };
  const guard = (fn: () => void) => (dirty.current ? setLeaving(() => fn) : fn());
  // Whether the Note in the pane would also be listed under `n`. Not knowing counts as no.
  const paneFits = (n: Nav) => {
    const note =
      pane?.type === "note"
        ? notes?.find((x) => x.id === pane.id)
        : pane?.type === "edit"
          ? pane.note
          : pane?.type === "new"
            ? { kind: pane.kind, favorite: false }
            : undefined;
    return !!note && (n.type === "all" || (n.type === "favorites" ? note.favorite : n.type === "kind" && n.kind === note.kind));
  };
  const go = (n: Nav, expiring = false) => {
    if (sameNav(n, nav) && expiryOnly === expiring) return;
    void cancelNativeOperation().catch(() => {});
    setCopyError(null);
    // Among All, Favorites and the Kinds the search stays, and so does a Note that still belongs.
    const keepSearch = listed(n) && listed(nav);
    if (keepSearch && !expiring && (!pane || paneFits(n))) {
      setExpiryOnly(false);
      return setNav(n);
    }
    guard(() => {
      setNav(n);
      setExpiryOnly(expiring);
      if (!keepSearch || expiring) setQuery("");
      open(null);
    });
  };
  // In a Kind filter New goes straight to that Kind's Editor and the filter stays. From Settings
  // or Trash it goes to All.
  const newNote = () =>
    guard(() => {
      const n = navNow.current;
      if (n.type === "kind") return open({ type: "new", kind: n.kind });
      if (!listed(n)) setNav(ALL);
      open({ type: "pick" });
    });
  const copyRow = (id: string, field: string) => {
    setCopyError(null);
    copyField(id, field).catch((e) => setCopyError(isApiError(e) ? e.message : s.somethingWentWrong));
  };
  // From the search box: arrows move the mark, Enter opens it (or the first match), Escape clears.
  const onSearchKey = (e: KeyEvent) => {
    // Not while an input method is composing (WebKit reports the Enter that ends it as keyCode
    // 229, not composing), with a modifier, or under a dialog.
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || document.querySelector("[role=dialog]")) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (!shown.length) return;
      e.preventDefault();
      const at = shown.findIndex((n) => n.id === marked);
      const to = e.key === "ArrowDown" ? Math.min(at + 1, shown.length - 1) : at < 0 ? shown.length - 1 : Math.max(at - 1, 0);
      setMarked(shown[to]?.id ?? null);
    } else if (e.key === "Enter") {
      // list_notes answers later: act only when the rows are the answer to what is in the box.
      if (!notes || list?.query !== query) return;
      const id = marked ?? (query ? shown[0]?.id : undefined);
      if (id) guard(() => open({ type: "note", id }));
    } else if (e.key === "Escape" && query) {
      e.preventDefault();
      setQuery("");
    }
  };

  useEffect(() => {
    if (nav.type === "settings") return;
    let live = true;
    listNotes(nav, query).then((n) => live && setList({ nav, query, notes: n }), () => {});
    return () => {
      live = false;
    };
  }, [nav, query, version]);

  // Again whenever Notes change, and on a change of view: opening a Backup from Settings changes
  // every Note without telling this screen.
  useEffect(() => {
    let live = true;
    listNotes(API_KEYS, "").then((n) => live && setApiKeys(n), () => {});
    return () => {
      live = false;
    };
  }, [nav, version]);

  // A copy is new when Rust's count of copies has gone up. The time left can't say: a Wallet
  // Kind copy may have less of it than the copy before, and a clear being tried again has more.
  const clipboard = status.clipboard_clear_in_ms;
  const copies = status.copies;
  const source = status.clipboard_source;
  const copiedFrom = source ? s.copiedFrom(s.fields[source.kind][source.field] ?? source.field, source.title) : undefined;
  useEffect(() => {
    const isNew = copies !== copiesWas.current;
    copiesWas.current = copies;
    if (clipboard === null) setCopied(null);
    else if (isNew) setCopied((c) => ({ seconds: seconds(clipboard), n: (c?.n ?? 0) + 1, source: copiedFrom }));
  }, [clipboard, copies, copiedFrom]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!cmdKey(e)) return;
      if (document.querySelector("[role=dialog]")) return;
      const key = e.key.toLowerCase();
      if (key === "n") {
        e.preventDefault();
        newNote();
      } else if (key === "f") {
        e.preventDefault();
        // Settings and Trash have no search box (and no Editor that could hold a draft).
        if (!listed(navNow.current)) {
          setNav(ALL);
          open(null);
        }
        setTimeout(() => {
          search.current?.focus();
          search.current?.select();
        });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The draft of an edit comes back once its Note has been fetched again. If the Note is gone,
  // so is the draft. Any other refusal means a lock or the shield is on its way: the status that
  // follows takes this screen away, and App drops the draft on a lock and keeps it under the shield.
  useEffect(() => {
    if (pane?.type !== "restoring") return;
    let live = true;
    // The pane that is still waiting, with the draft it holds meanwhile.
    const waiting = () => (live && paneNow.current?.type === "restoring" ? paneNow.current : null);
    getNote(pane.id).then(
      (note) => {
        const from = waiting();
        if (from) setPane({ type: "edit", note, draft: from.draft });
      },
      (e) => waiting() && isApiError(e) && e.code === "not_found" && open(null),
    );
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    const noteId = pane?.type === "note" || pane?.type === "restoring" ? pane.id : pane?.type === "edit" ? pane.note.id : null;
    onPlace({ nav, noteId, sort });
  }, [nav, pane, sort]);
  useEffect(() => setMarked(null), [list, sort, query, expiryOnly]);
  useEffect(() => {
    if (marked) document.getElementById(rowId(marked))?.scrollIntoView({ block: "nearest" });
  }, [marked]);
  // The picker the owner asked for takes the keyboard.
  useEffect(() => {
    if (pane?.type === "pick") firstTile.current?.focus();
  }, [pane]);

  const saved = (id: string) => {
    open({ type: "note", id });
    changed();
  };

  let detail;
  if (pane?.type === "note")
    detail = (
      <Detail
        key={pane.id}
        id={pane.id}
        status={status}
        onEdit={(note, field) => open({ type: "edit", note, field })}
        onDuplicate={(kind, seed) => open({ type: "new", kind, seed })}
        onChanged={changed}
        onTrashed={() => {
          open({ type: "trashed" });
          changed();
        }}
        // The Note that was open before a lock may be gone by now.
        onMissing={() => setPane((p) => (p?.type === "note" && p.id === pane.id ? null : p))}
      />
    );
  else if (pane?.type === "edit")
    detail = (
      <Editor
        kind={pane.note.kind}
        note={pane.note}
        focusField={pane.field}
        draft={pane.draft}
        status={status}
        onSaved={saved}
        onCancel={() => guard(() => open({ type: "note", id: pane.note.id }))}
        onDirty={onDirty}
        onDraft={report}
      />
    );
  else if (pane?.type === "new")
    detail = (
      <Editor
        key={opened}
        kind={pane.kind}
        seed={pane.seed}
        savedBefore={pane.savedBefore}
        draft={pane.draft}
        status={status}
        onSaved={saved}
        // Save and add another: `open` counts a new pane, so this is a fresh, empty Editor.
        onSavedAndNew={() => {
          open({ type: "new", kind: pane.kind, savedBefore: true });
          changed();
        }}
        onCancel={() => guard(() => open(null))}
        onDirty={onDirty}
        onDraft={report}
      />
    );
  else if (pane?.type === "trashed")
    detail = (
      <p className="empty-state" role="status">
        {s.movedToTrash}
      </p>
    );
  else if (pane?.type === "restoring") detail = null;
  else if (pane?.type === "pick" || (nav.type === "all" && !query && notes?.length === 0)) {
    // The same tiles are the first-run page (no pane); only the picker the owner asked for takes
    // the keyboard: focus on the first tile, and 1 to 5 choose.
    const picking = !!pane;
    const byDigit = (e: KeyEvent) => {
      const kind = e.metaKey || e.ctrlKey || e.altKey ? undefined : KINDS[Number(e.key) - 1];
      if (!kind) return;
      e.preventDefault(); // or the digit is typed into the Editor's title box
      open({ type: "new", kind });
    };
    detail = (
      <div className="stack">
        <h1>{picking ? s.pickKind : s.firstNote}</h1>
        <div className="tiles" onKeyDown={picking ? byDigit : undefined}>
          {KINDS.map((kind, i) => (
            <button
              key={kind}
              ref={i === 0 ? firstTile : undefined}
              className="tile"
              aria-keyshortcuts={picking ? String(i + 1) : undefined}
              onClick={() => open({ type: "new", kind })}
            >
              <KindBadge kind={kind} />
              <strong>{s.kindNames[kind]}</strong>
              <span>{s.kindBlurbs[kind]}</span>
              {picking && <kbd aria-hidden="true">{i + 1}</kbd>}
            </button>
          ))}
        </div>
      </div>
    );
  } else if (notes?.length) detail = <p className="empty-state">{s.pickNote}</p>;

  const markedNote = marked ? shown.find((n) => n.id === marked) : undefined;
  // Counted against today's date at every draw (the status redraws this screen each second), so
  // the banner is right after midnight too.
  const affectedKeys = apiKeys.filter(withinExpiryWarning);
  const expiredKeys = affectedKeys.filter((n) => expiry(n.expires)?.past).length;
  const expiringKeys = affectedKeys.length - expiredKeys;
  const capture = status.capture_hiding_active;
  const lastBackup = status.backup.last_backup_at;
  const unbacked = status.backup.unbacked_changes;
  const backupText = lastBackup
    ? (unbacked ? s.lastBackupUnbacked : s.lastBackup)(formatDate(lastBackup))
    : unbacked
      ? s.neverBackedUpUnbacked
      : s.neverBackedUp;
  const navButton = (n: Nav) => (
    <button key={navLabel(n)} className={sameNav(n, nav) ? "nav on" : "nav"} aria-current={sameNav(n, nav)} onClick={() => go(n)}>
      <Icon name={navIcon(n)} />
      {navLabel(n)}
    </button>
  );
  return (
    <div className="home">
      <nav className="sidebar">
        <TitlebarDrag />
        {navButton(ALL)}
        {navButton(FAVORITES)}
        <p className="sidebar-heading">{s.navKinds}</p>
        {BY_KIND.map(navButton)}
        <div className="sidebar-gap" />
        {navButton(TRASH)}
        <div className="bottom" />
        {navButton(SETTINGS)}
        <button className="nav" onClick={onLock}>
          <Icon name="lock" />
          {s.lock}
          <kbd>{s.shortcut(mac, "L")}</kbd>
        </button>
      </nav>
      <div className="main">
        <header className="toolbar" {...dragRegion}>
          <div className="toolbar-title">
            <h1>{navLabel(nav)}</h1>
            {nav.type !== "settings" && notes && <span>{s.noteCount(notes.length)}</span>}
          </div>
          {listed(nav) && (
            <>
              <label className="search">
                <Icon name="search" />
                <input
                  ref={search}
                  // Marks the one box where the open Note's copy shortcut still works: see Detail.
                  data-search
                  placeholder={s.search}
                  aria-label={s.searchLabel}
                  aria-controls={listId}
                  value={query}
                  onChange={(e) => {
                    setCopyError(null);
                    setQuery(e.target.value);
                  }}
                  onKeyDown={onSearchKey}
                  {...noAssist}
                />
                {!query && <kbd aria-hidden="true">{s.shortcut(mac, "F")}</kbd>}
              </label>
              <button className="primary" onClick={newNote}>
                <Icon name="plus" />
                {nav.type === "kind" ? s.newTitle(s.kindNames[nav.kind]) : s.newNote}
                <kbd>{s.shortcut(mac, "N")}</kbd>
              </button>
            </>
          )}
        </header>
        {status.backup.reminder_due && nav.type !== "settings" && (
          <div className="banner">
            <Icon name="warning" />
            {s.backupReminder}
            <button onClick={() => go(SETTINGS)}>{s.goToBackups}</button>
          </div>
        )}
        {(expiredKeys > 0 || expiringKeys > 0) && nav.type !== "settings" && (
          <div className="banner">
            <Icon name="warning" />
            {s.expiryBanner(expiredKeys, expiringKeys, EXPIRY_BANNER_DAYS)}
            <button onClick={() => go(API_KEYS, !expiryOnly)}>{expiryOnly ? s.showAllApiKeys : s.showApiKeys}</button>
          </div>
        )}
        {status.purge_trash_failed && <div className="banner" role="alert">{s.trashPurgeFailed}</div>}
        {nav.type === "settings" ? (
          <Settings status={status} onOperationOutcome={onOperationOutcome} />
        ) : nav.type === "trash" ? (
          <Trash
            notes={notes}
            status={status}
            openId={pane?.type === "note" ? pane.id : null}
            onOpen={(id) => open(id ? { type: "note", id } : null)}
            onChanged={changed}
          />
        ) : (
          <div className="columns">
            <section className="list-pane">
              {(!!notes?.length || !!query) && <Choice label={s.sortLabel} options={SORTS} value={sort} onChange={setSort} />}
              <ErrorLine error={copyError} />
              <ul className="list" id={listId}>
                {shown.map((n, i) => {
                  const before = shown[i - 1];
                  // No Copy button on a row whose Note has nothing in that field.
                  const field = n.has_hidden ? ROW_COPY[n.kind] : undefined;
                  const due = expiry(n.expires);
                  const on = pane?.type === "note" && pane.id === n.id;
                  return (
                    <Fragment key={n.id}>
                      {/* API Keys A to Z: a heading per service; Notes without one come last, set apart. */}
                      {byService &&
                        (!before || collator.compare(before.subtitle ?? "", n.subtitle ?? "") !== 0) &&
                        (n.subtitle ? <li className="list-heading">{n.subtitle}</li> : before && <li className="list-gap" aria-hidden="true" />)}
                      <li className={field ? "has-copy" : undefined}>
                        <button
                          id={rowId(n.id)}
                          className={`row${on ? " on" : ""}${marked === n.id ? " marked" : ""}`}
                          onClick={() => guard(() => open({ type: "note", id: n.id }))}
                        >
                          <KindBadge kind={n.kind} />
                          <span className="row-title">
                            <span>{n.title}</span>
                            {n.favorite && <Icon name="star" className="icon fav" />}
                          </span>
                          {/* The expiry first: a long subtitle is cut off at the end of the line. */}
                          <span className="hint">
                            {s.kindNames[n.kind]}
                            {due && " · "}
                            {due && <span className={due.past ? "expiry past" : "expiry"}>{due.text}</span>}
                            {n.subtitle && ` · ${n.subtitle}`}
                          </span>
                        </button>
                        {/* Beside the row button, not inside it. Rust writes the clipboard; no value comes here. */}
                        {field && (
                          <button
                            className="plain row-copy"
                            aria-label={s.copyFrom(s.fields[n.kind][field] ?? field, n.title)}
                            onClick={() => copyRow(n.id, field)}
                          >
                            <Icon name="copy" />
                          </button>
                        )}
                      </li>
                    </Fragment>
                  );
                })}
              </ul>
              {notes?.length === 0 && (query || nav.type !== "all") && <p className="hint">{query ? s.noMatches : s.nothingHere}</p>}
            </section>
            <section className="detail-pane">{detail}</section>
          </div>
        )}
        <footer className="status-line">
          <span>
            {s.statusLine}
            {status.lock_in_ms !== null && ` · ${s.locksIn(minSec(status.lock_in_ms), !!status.settings.count_activity_anywhere)}`}
            {" · "}
            {unbacked && nav.type !== "settings" ? (
              <button className="link" onClick={() => go(SETTINGS)}>
                {backupText}
              </button>
            ) : (
              backupText
            )}
          </span>
          <span className={capture ? "badge" : "badge warn"}>
            {capture ? s.captureOn(status.platform === "mac") : capture === false ? s.captureOff : s.captureUnknown}
          </span>
        </footer>
      </div>
      {/* Announced once per copy, with the time that copy has (30 or 60 seconds as set, 30 for a
          Wallet Kind); the ticking toast itself stays out of the screen reader's way. A space is
          added for every other copy: the same words twice in a row would not be read out again. */}
      <p className="sr-only" role="status">
        {copied && s.copiedAnnounce(copied.seconds, copied.source) + (copied.n % 2 ? "" : NO_BREAK_SPACE)}
      </p>
      {/* The row the arrow keys are on, for a screen reader: the keyboard stays in the search box,
          and this line is the one thing that says which row is marked. (The box does not point
          at the row with aria-activedescendant: a button in a plain list is not something that
          may be pointed at, and a reader that followed it anyway would read the row twice.)
          Always here, so that the change of its text is read out. */}
      <p className="sr-only" role="status">
        {markedNote && s.markedRow(markedNote.title, s.kindNames[markedNote.kind])}
      </p>
      {status.clipboard_clear_in_ms !== null && (
        <div className="toast" aria-hidden="true">
          {s.copied(seconds(status.clipboard_clear_in_ms), copiedFrom)}
        </div>
      )}
      {leaving && (
        <Confirm
          title={s.discardTitle}
          text={s.discardText}
          confirmLabel={s.discard}
          onConfirm={() => {
            setLeaving(null);
            leaving();
          }}
          onClose={() => setLeaving(null)}
        />
      )}
    </div>
  );
}

/** The Notes in Trash, or one of them opened to read (`openId`): Show and Copy work there, so a
 *  key can be read without restoring the Note, which would restart its 30 days. */
function Trash(props: {
  notes: NoteSummary[] | undefined;
  status: AppStatus;
  openId: string | null;
  onOpen: (id: string | null) => void;
  onChanged: () => void;
}) {
  const { notes, openId, onOpen } = props;
  // id = Delete forever for one Note; "" = Empty Trash.
  const [confirm, setConfirm] = useState<string | null>(null);
  // The same, once confirmed, when a Seed Phrase or Private Key would go: that needs the Master
  // Password, like every other change to one.
  const [askPassword, setAskPassword] = useState<{ id: string; titles: string[] } | null>(null);
  const { error, run } = useAction();
  // The keyboard follows into a Note opened from a row, not into one restored after a lock.
  const back = useRef<HTMLButtonElement>(null);
  const followed = useRef(false);
  useEffect(() => {
    if (openId && followed.current) back.current?.focus();
    followed.current = false;
  }, [openId]);
  // And it comes back out when the owner leaves that Note: to its row or, when the row is going
  // (`gone`: restored or deleted), to the list, or to the line that says nothing is left.
  const rows = useRef<Record<string, HTMLButtonElement | null>>({});
  const list = useRef<HTMLUListElement>(null);
  const nothing = useRef<HTMLParagraphElement>(null);
  const left = useRef<{ id: string; gone: boolean } | null>(null);
  const leave = (gone: boolean) => {
    if (openId) left.current = { id: openId, gone };
    onOpen(null);
  };
  useEffect(() => {
    const from = left.current;
    if (openId) left.current = null;
    // Not under a dialog, which keeps the keyboard until it closes, nor before there is a list.
    if (openId || !from || !notes || confirm !== null || askPassword !== null) return;
    const row = rows.current[from.id];
    // A row that is going is still listed until the list has been fetched again: wait for that.
    if (from.gone && row) return;
    left.current = null;
    // Only when nothing else has taken the keyboard meanwhile.
    const at = document.activeElement;
    if (at && at !== document.body) return;
    const to = row ?? list.current ?? nothing.current;
    // The list can hold the keyboard for this once only. Able to hold it all the time, it would
    // be handed it at every click on one of its buttons (WebKit looks for the nearest holder).
    if (to && to !== row) {
      to.tabIndex = -1;
      to.addEventListener("blur", () => to.removeAttribute("tabindex"), { once: true });
    }
    to?.focus();
  }, [openId, notes, confirm, askPassword]);
  // Restoring or deleting the Note being read goes back to the list.
  const act = (fn: () => Promise<unknown>, id?: string) =>
    run(fn).then((ok) => {
      props.onChanged();
      if (ok && id === openId) leave(true);
    });
  // A key still held down from the press that moved the keyboard here doesn't press these too.
  const noRepeat = (e: KeyEvent) => e.repeat && e.preventDefault();
  return (
    <section className="page stack">
      {openId ? (
        <>
          <div className="actions">
            <button ref={back} className="link" onKeyDown={noRepeat} onClick={() => leave(false)}>
              <Icon name="back" />
              {s.backToTrash}
            </button>
          </div>
          <Detail key={openId} id={openId} status={props.status} readOnly onMissing={() => onOpen(null)} />
          <ErrorLine error={error} />
          <div className="actions">
            <button onClick={() => act(() => restoreNote(openId), openId)}>{s.restore}</button>
            <button className="danger" onClick={() => setConfirm(openId)}>
              {s.deleteForever}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="hint">{s.trashText}</p>
          <ErrorLine error={error} />
          {!!notes?.length && (
            <ul ref={list} className="card rows">
              {notes.map((n) => (
                <li key={n.id} className="trash-row">
                  <button
                    ref={(el) => {
                      rows.current[n.id] = el;
                    }}
                    className="row"
                    onKeyDown={noRepeat}
                    onClick={() => {
                      followed.current = true;
                      onOpen(n.id);
                    }}
                  >
                    <KindBadge kind={n.kind} />
                    <span className="row-title">
                      <span>{n.title}</span>
                    </span>
                    {/* The date first: a long subtitle is cut off at the end of the line. */}
                    <span className="hint">
                      {s.kindNames[n.kind]}
                      {n.deleted_at && ` · ${s.deletedOn(formatDate(n.deleted_at))}`}
                      {n.subtitle && ` · ${n.subtitle}`}
                    </span>
                  </button>
                  <button onClick={() => act(() => restoreNote(n.id))}>{s.restore}</button>
                  <button className="danger" onClick={() => setConfirm(n.id)}>
                    {s.deleteForever}
                  </button>
                </li>
              ))}
            </ul>
          )}
          {notes?.length === 0 && (
            <p ref={nothing} className="hint">
              {s.nothingHere}
            </p>
          )}
          {!!notes?.length && (
            <div className="actions end">
              <button className="danger" onClick={() => setConfirm("")}>
                {s.emptyTrash}
              </button>
            </div>
          )}
        </>
      )}
      {confirm !== null && (
        <Confirm
          title={confirm ? s.deleteForever : s.emptyTrash}
          text={confirm ? s.deleteForeverText : s.emptyTrashText}
          confirmLabel={confirm ? s.deleteForever : s.emptyTrash}
          onConfirm={() => {
            setConfirm(null);
            const removing = notes?.filter((n) => !confirm || n.id === confirm) ?? [];
            if (removing.some((n) => isWalletKind(n.kind))) setAskPassword({ id: confirm, titles: removing.map((n) => n.title) });
            else act(() => (confirm ? deleteForever(confirm) : emptyTrash()), confirm);
          }}
          onClose={() => setConfirm(null)}
        />
      )}
      {askPassword !== null && (
        <NativeActionModal
          title={askPassword.id ? s.noteAction(s.deleteForever, askPassword.titles[0]!) : s.emptyTrash}
          text={askPassword.id ? s.walletDeleteText : s.walletEmptyTrashText(askPassword.titles)}
          submitLabel={askPassword.id ? s.deleteForever : s.emptyTrash}
          onSubmit={async () => {
            await (askPassword.id ? deleteForever(askPassword.id) : emptyTrash());
            props.onChanged();
            if (askPassword.id && askPassword.id === openId) leave(true);
          }}
          onClose={() => setAskPassword(null)}
          retryInMs={props.status.retry_in_ms}
        />
      )}
    </section>
  );
}
