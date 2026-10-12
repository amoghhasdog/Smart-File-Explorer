import { ipc, type IndexStatus } from "../../lib/ipc";
import { formatCount } from "../../lib/format";

interface Props {
  status: IndexStatus | null;
}

export function StatusBar({ status }: Props) {
  if (!status) return <footer className="statusbar muted">Connecting…</footer>;
  const busy = status.pendingJobs > 0 || status.scanning.length > 0;
  const done = status.totalFiles - status.discovered;
  const pct = status.totalFiles ? Math.round((done / status.totalFiles) * 100) : 0;

  let text: string;
  if (status.scanning.length > 0) {
    text = `Scanning… ${formatCount(status.totalFiles)} files found`;
  } else if (status.pendingJobs > 0) {
    text = `Indexing ${formatCount(status.pendingJobs)} remaining · ${pct}%`;
  } else {
    text = `${formatCount(status.totalFiles)} files indexed`;
  }
  if (status.failed > 0) text += ` · ${formatCount(status.failed)} failed`;
  if (!busy && status.waitingForModel > 0) {
    text += ` · ${formatCount(status.waitingForModel)} waiting for semantic model`;
  }

  return (
    <footer className="statusbar" aria-live="polite">
      <span className={busy ? "pulse" : undefined}>{text}</span>
      {status.paused && <span className="badge">paused</span>}
      {status.lastError && (
        <span className="error" title={status.lastError}>
          error
        </span>
      )}
      <span className="spacer" />
      {busy || status.paused ? (
        <button onClick={() => (status.paused ? ipc.indexResume() : ipc.indexPause())}>
          {status.paused ? "Resume" : "Pause"}
        </button>
      ) : null}
    </footer>
  );
}
