// An agent's request waiting for the owner (ADR-0012): which one it is, and the two confirms
// (an edit that says where a key is used, and a delete). Store and replace open the Editor; see
// Home. Everything shown here is a visible detail, never a value. Titles and details were
// written by the agent, so they are drawn as plain text, in quotes.

import { useEffect, useId, useRef, useState } from "react";
import { agentCancelRequest, agentConfirmDelete, agentConfirmEdit, agentPending, onAgentRequest, type AgentPendingView } from "./api";
import { ErrorLine, Modal, NativeActionModal, useAction } from "./components";
import { chainLabel, label } from "./Note";
import { strings as s } from "./strings";

/** How often the screens ask whether the request is still waiting. Rust also says when one
 *  starts; a request also ends by itself (five minutes, a lock, Stop), and nothing says that. */
const POLL_MS = 2000;

/** The request waiting now: undefined until the first answer, null when there is none. */
export function useAgentPending() {
  const [pending, setPending] = useState<AgentPendingView | null | undefined>(undefined);
  const ask = useRef(() => {});
  useEffect(() => {
    let alive = true;
    // Only the newest answer counts: an older one could bring back a request that ended.
    let asked = 0;
    const get = () => {
      const n = ++asked;
      agentPending().then(
        (next) => alive && n === asked && setPending((now) => (now && next && now.request_id === next.request_id ? now : next)),
        () => {},
      );
    };
    ask.current = get;
    get();
    const unlisten = onAgentRequest(get);
    const timer = window.setInterval(get, POLL_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
      void unlisten.then((u) => u());
    };
  }, []);
  return { pending, refresh: () => ask.current() };
}

/** One line per changed field. A chain is named as the Editor names it. */
function changeLines(request: AgentPendingView) {
  return request.changes.map((c) => {
    const name = c.field === "title" ? s.title : label(request.kind, c.field);
    const value = (v: string) => (c.field === "chain" && v ? chainLabel(v) : v);
    return s.agentChange(name, value(c.from), value(c.to));
  });
}

/** The confirm for an edit or a delete, then the Master Password panel where it is needed: for
 *  a delete always, for an edit of a Seed Phrase or Private Key. Cancel holds the keyboard, as in
 *  every confirm. Leaving any way but through the change cancels the request, so the agent hears
 *  "cancelled" at once. `ended`: the request stopped waiting; a confirm still on screen then
 *  closes by itself (`onEnded`). */
export function AgentRequestDialog({ request, ended, retryInMs, onDone, onClose, onEnded }: {
  request: AgentPendingView;
  ended: boolean;
  retryInMs: number | null;
  /** The change was made. */
  onDone: () => void;
  /** The owner said no. */
  onClose: () => void;
  onEnded: () => void;
}) {
  const remove = request.request === "delete";
  const [step, setStep] = useState<"confirm" | "password">("confirm");
  const { busy, error, run } = useAction();
  const done = useRef(false);
  const text = useId();
  const id = request.request_id;
  useEffect(() => {
    if (ended && step === "confirm" && !busy) onEnded();
  }, [ended, step, busy]);
  const decline = () => {
    void agentCancelRequest(id).catch(() => {});
    onClose();
  };
  const allow = () => {
    // A delete always opens the Master Password panel, whatever the view says.
    if (remove || request.needs_password) return setStep("password");
    run(() => agentConfirmEdit(id)).then((ok) => ok && onDone());
  };
  if (step === "password")
    return (
      <NativeActionModal
        title={s.noteAction(remove ? s.moveToTrash : s.edit, request.title)}
        text={remove ? s.agentDeletePasswordText : s.agentEditPasswordText}
        onSubmit={async () => {
          await (remove ? agentConfirmDelete(id) : agentConfirmEdit(id));
          done.current = true;
        }}
        // Also after a Cancel in the panel, or on this sheet: then the agent hears so.
        onClose={() => (done.current ? onDone() : decline())}
        retryInMs={retryInMs}
      />
    );
  const lines = changeLines(request);
  return (
    <Modal title={remove ? s.agentDeleteTitle(request.title) : s.agentEditTitle} onClose={decline} alert describedBy={text}>
      <div id={text} className="stack">
        {remove ? (
          <p>{s.agentDeleteText(request.title)}</p>
        ) : (
          <>
            <p>{s.agentEditIntro(request.title)}</p>
            <ul className="changes">
              {lines.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
            <p className="hint">{s.agentEditWhy}</p>
            {request.needs_password && <p className="hint">{s.agentEditPassword}</p>}
          </>
        )}
      </div>
      <ErrorLine error={error} />
      <div className="actions">
        {/* Cancel has the keyboard: a stray Enter or Space changes nothing. A key still held down
            doesn't press it either. */}
        <button autoFocus onKeyDown={(e) => e.repeat && e.preventDefault()} onClick={decline}>
          {s.cancel}
        </button>
        <button className={remove ? "danger" : "primary"} disabled={busy} onClick={allow}>
          {remove ? s.moveToTrash : s.agentAllow}
        </button>
      </div>
    </Modal>
  );
}
