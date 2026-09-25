// The Unlocked Vault: sidebar, search, list, detail pane, Trash, status line.

import { useEffect, useRef, useState } from "react";
import {
  deleteForever,
  emptyTrash,
  KINDS,
  listNotes,
  lock,
  restoreNote,
  type AppStatus,
  type Filter,
  type Kind,
  type NoteSummary,
  type NoteView,
} from "./api";
import { Confirm, dragRegion, ErrorLine, formatDate, mac, minSec, noAssist, seconds, TitlebarDrag, useAction } from "./components";
import { Icon, KIND_ICONS, KindBadge, type IconName } from "./icons";
import { Detail, Editor } from "./Note";
import { Settings } from "./Settings";
import { strings as s } from "./strings";

type Nav = Filter | { type: "settings" };
const ALL: Nav = { type: "all" };
const SETTINGS: Nav = { type: "settings" };
const TRASH: Nav = { type: "trash" };
const BY_KIND = KINDS.map((kind): Nav => ({ type: "kind", kind }));
const navLabel = (n: Nav) => (n.type === "kind" ? s.kindPlurals[n.kind] : s.nav[n.type]);
const NAV_ICONS: Record<Exclude<Nav["type"], "kind">, IconName> = { all: "all", favorites: "star", trash: "trash", settings: "settings" };
const navIcon = (n: Nav) => (n.type === "kind" ? KIND_ICONS[n.kind] : NAV_ICONS[n.type]);

type Pane =
  | { type: "pick" }
  | { type: "note"; id: string }
  | { type: "edit"; note: NoteView }
  | { type: "new"; kind: Kind }
  | { type: "trashed" }
  | null;

export function Home({ status, onKit }: { status: AppStatus; onKit: (recoveryKey: string) => void }) {
  const [nav, setNav] = useState(ALL);
  const [query, setQuery] = useState("");
  // Undefined until this nav's list arrives, so nothing (not "empty") shows while loading.
  const [list, setList] = useState<{ nav: Nav; notes: NoteSummary[] }>();
  const notes = list?.nav === nav ? list.notes : undefined;
  const [pane, setPane] = useState<Pane>(null);
  const [version, setVersion] = useState(0);
  // An Editor with unsaved changes asks before it's replaced.
  const dirty = useRef(false);
  const [leaving, setLeaving] = useState<(() => void) | null>(null);
  const search = useRef<HTMLInputElement>(null);
  const changed = () => setVersion((v) => v + 1);
  const open = (next: Pane) => {
    dirty.current = false;
    setPane(next);
  };
  const onDirty = () => {
    dirty.current = true;
  };
  const guard = (fn: () => void) => (dirty.current ? setLeaving(() => fn) : fn());
  const go = (n: Nav, next: Pane = null) =>
    guard(() => {
      setNav(n);
      setQuery("");
      open(next);
    });

  useEffect(() => {
    if (nav.type === "settings") return;
    let live = true;
    listNotes(nav, query).then((n) => live && setList({ nav, notes: n }), () => {});
    return () => {
      live = false;
    };
  }, [nav, query, version]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "n") {
        e.preventDefault();
        go(ALL, { type: "pick" });
      } else if (key === "f") {
        e.preventDefault();
        setNav((n) => (n.type === "settings" || n.type === "trash" ? ALL : n));
        setTimeout(() => search.current?.focus());
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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
        onEdit={(note) => open({ type: "edit", note })}
        onChanged={changed}
        onTrashed={() => {
          open({ type: "trashed" });
          changed();
        }}
      />
    );
  else if (pane?.type === "edit")
    detail = (
      <Editor
        kind={pane.note.kind}
        note={pane.note}
        status={status}
        onSaved={saved}
        onCancel={() => open({ type: "note", id: pane.note.id })}
        onDirty={onDirty}
      />
    );
  else if (pane?.type === "new")
    detail = <Editor key={pane.kind} kind={pane.kind} status={status} onSaved={saved} onCancel={() => open(null)} onDirty={onDirty} />;
  else if (pane?.type === "trashed")
    detail = (
      <p className="empty-state" role="status">
        {s.movedToTrash}
      </p>
    );
  else if (pane?.type === "pick" || (nav === ALL && !query && notes?.length === 0))
    detail = (
      <div className="stack">
        <h1>{pane ? s.pickKind : s.firstNote}</h1>
        <div className="tiles">
          {KINDS.map((kind) => (
            <button key={kind} className="tile" onClick={() => open({ type: "new", kind })}>
              <KindBadge kind={kind} />
              <strong>{s.kindNames[kind]}</strong>
              <span>{s.kindBlurbs[kind]}</span>
            </button>
          ))}
        </div>
      </div>
    );
  else if (notes?.length) detail = <p className="empty-state">{s.pickNote}</p>;

  const capture = status.capture_hiding_active;
  const listed = nav.type !== "settings" && nav.type !== "trash";
  const navButton = (n: Nav) => (
    <button key={navLabel(n)} className={n === nav ? "nav on" : "nav"} aria-current={n === nav} onClick={() => go(n)}>
      <Icon name={navIcon(n)} />
      {navLabel(n)}
    </button>
  );
  return (
    <div className="home">
      <nav className="sidebar">
        <TitlebarDrag />
        {navButton(ALL)}
        {navButton({ type: "favorites" })}
        <p className="sidebar-heading">{s.navKinds}</p>
        {BY_KIND.map(navButton)}
        <div className="sidebar-gap" />
        {navButton(TRASH)}
        <div className="bottom" />
        {navButton(SETTINGS)}
        <button className="nav" onClick={() => lock().catch(() => {})}>
          <Icon name="lock" />
          {s.lock}
          <kbd>{s.lockShortcut(mac)}</kbd>
        </button>
      </nav>
      <div className="main">
        <header className="toolbar" {...dragRegion}>
          <div className="toolbar-title">
            <h1>{navLabel(nav)}</h1>
            {nav.type !== "settings" && notes && <span>{s.noteCount(notes.length)}</span>}
          </div>
          {listed && (
            <>
              <label className="search">
                <Icon name="search" />
                <input
                  ref={search}
                  placeholder={s.search}
                  aria-label={s.searchLabel}
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  {...noAssist}
                />
              </label>
              <button className="primary" onClick={() => guard(() => open({ type: "pick" }))}>
                <Icon name="plus" />
                {s.newNote}
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
        {nav.type === "settings" ? (
          <Settings status={status} onKit={onKit} />
        ) : nav.type === "trash" ? (
          <Trash notes={notes} onChanged={changed} />
        ) : (
          <div className="columns">
            <section className="list-pane">
              <ul className="list">
                {notes?.map((n) => (
                  <li key={n.id}>
                    <button
                      className={pane?.type === "note" && pane.id === n.id ? "row on" : "row"}
                      onClick={() => guard(() => open({ type: "note", id: n.id }))}
                    >
                      <KindBadge kind={n.kind} />
                      <span className="row-title">
                        <span>{n.title}</span>
                        {n.favorite && <Icon name="star" className="icon fav" />}
                      </span>
                      <span className="hint">
                        {s.kindNames[n.kind]}
                        {n.subtitle && ` · ${n.subtitle}`}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
              {notes?.length === 0 && (query || nav !== ALL) && <p className="hint">{query ? s.noMatches : s.nothingHere}</p>}
            </section>
            <section className="detail-pane">{detail}</section>
          </div>
        )}
        <footer className="status-line">
          <span>
            {s.statusLine}
            {status.lock_in_ms !== null && ` · ${s.locksIn(minSec(status.lock_in_ms))}`}
            {" · "}
            {status.backup.last_backup_at ? s.lastBackup(formatDate(status.backup.last_backup_at)) : s.neverBackedUp}
          </span>
          <span className={capture ? "badge" : "badge warn"}>
            {capture ? s.captureOn(status.platform === "mac") : capture === false ? s.captureOff : s.captureUnknown}
          </span>
        </footer>
      </div>
      {/* Announced once; the ticking toast itself stays out of the screen reader's way. */}
      <p className="sr-only" role="status">
        {status.clipboard_clear_in_ms !== null && s.copiedAnnounce}
      </p>
      {status.clipboard_clear_in_ms !== null && (
        <div className="toast" aria-hidden="true">
          {s.copied(seconds(status.clipboard_clear_in_ms))}
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

function Trash({ notes, onChanged }: { notes: NoteSummary[] | undefined; onChanged: () => void }) {
  // id = Delete forever for one Note; "" = Empty Trash.
  const [confirm, setConfirm] = useState<string | null>(null);
  const { error, run } = useAction();
  const act = (fn: () => Promise<unknown>) => run(fn).then(onChanged);
  return (
    <section className="page stack">
      <p className="hint">{s.trashText}</p>
      <ErrorLine error={error} />
      {!!notes?.length && (
        <ul className="card rows">
          {notes.map((n) => (
            <li key={n.id} className="trash-row">
              <KindBadge kind={n.kind} />
              <span>
                <span className="row-title">{n.title}</span>
                <span className="hint">
                  {s.kindNames[n.kind]}
                  {n.deleted_at && ` · ${s.deletedOn(formatDate(n.deleted_at))}`}
                </span>
              </span>
              <button onClick={() => act(() => restoreNote(n.id))}>{s.restore}</button>
              <button className="danger" onClick={() => setConfirm(n.id)}>
                {s.deleteForever}
              </button>
            </li>
          ))}
        </ul>
      )}
      {notes?.length === 0 && <p className="hint">{s.nothingHere}</p>}
      {!!notes?.length && (
        <div className="actions end">
          <button className="danger" onClick={() => setConfirm("")}>
            {s.emptyTrash}
          </button>
        </div>
      )}
      {confirm !== null && (
        <Confirm
          title={confirm ? s.deleteForever : s.emptyTrash}
          text={confirm ? s.deleteForeverText : s.emptyTrashText}
          confirmLabel={confirm ? s.deleteForever : s.emptyTrash}
          onConfirm={() => {
            setConfirm(null);
            act(() => (confirm ? deleteForever(confirm) : emptyTrash()));
          }}
          onClose={() => setConfirm(null)}
        />
      )}
    </section>
  );
}
