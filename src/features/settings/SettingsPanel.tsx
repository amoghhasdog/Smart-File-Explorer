import { useEffect, useState } from "react";
import { ipc, isIpcError, type IndexStatus, type Problem, type Stats } from "../../lib/ipc";
import { formatBytes, formatCount } from "../../lib/format";

interface Props {
  status: IndexStatus | null;
}

/** Problems list, storage statistics and the (double-confirmed) purge. */
export function SettingsPanel({ status }: Props) {
  const [open, setOpen] = useState<"problems" | "stats" | null>(null);
  const [problems, setProblems] = useState<Problem[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);
  const [confirmPurge, setConfirmPurge] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const failed = status?.failed ?? 0;

  async function guard<T>(f: () => Promise<T>): Promise<T | undefined> {
    setError(null);
    try {
      return await f();
    } catch (e) {
      setError(isIpcError(e) ? e.message : String(e));
      return undefined;
    }
  }

  useEffect(() => {
    if (open === "problems") void guard(ipc.indexProblems).then((p) => p && setProblems(p));
    if (open === "stats") void guard(ipc.appStats).then((s) => s && setStats(s));
    if (open !== "stats") setConfirmPurge(false);
  }, [open, failed]);

  return (
    <section className="panel settings" aria-label="Status and settings">
      <div className="settings-tabs">
        <button
          className={open === "problems" ? "active" : undefined}
          onClick={() => setOpen(open === "problems" ? null : "problems")}
        >
          {failed === 1 ? "1 problem" : `${formatCount(failed)} problems`}
        </button>
        <button
          className={open === "stats" ? "active" : undefined}
          onClick={() => setOpen(open === "stats" ? null : "stats")}
        >
          Status & storage
        </button>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}

      {open === "problems" && (
        <div className="problems">
          {problems.length === 0 ? (
            <p className="muted">No failed files.</p>
          ) : (
            <>
              <button
                onClick={() =>
                  guard(ipc.indexRetryFailed).then(() => guard(ipc.indexProblems).then((p) => p && setProblems(p)))
                }
              >
                Retry all
              </button>
              <ul className="problem-list">
                {problems.map((p) => (
                  <li key={p.id} className="problem">
                    <div className="problem-name" title={p.path}>
                      {p.name}
                    </div>
                    <div className="problem-error muted">{p.error}</div>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}

      {open === "stats" && stats && (
        <div className="stats">
          <dl className="meta">
            {(
              [
                ["Files", formatCount(stats.files)],
                ["Text chunks", formatCount(stats.textChunks)],
                ["Image vectors", formatCount(stats.imageChunks)],
                ["Database", formatBytes(stats.dbBytes)],
                ["Vectors", formatBytes(stats.vectorsBytes)],
                ["Thumbnails", formatBytes(stats.thumbsBytes)],
                ["Models", formatBytes(stats.modelsBytes)],
                ["Version", stats.version],
              ] as [string, string][]
            ).map(([k, v]) => (
              <div key={k} className="meta-row">
                <dt>{k}</dt>
                <dd>{v}</dd>
              </div>
            ))}
          </dl>
          <p className="muted small" title={stats.dataDir}>
            Index location: {stats.dataDir}
          </p>
          {confirmPurge ? (
            <div className="purge-confirm">
              <p className="error">
                This removes all indexed folders, extracted text, vectors and thumbnails. Your
                files are not touched. Models stay installed.
              </p>
              <button
                className="danger"
                onClick={() =>
                  guard(ipc.indexPurge).then(() => {
                    setConfirmPurge(false);
                    void guard(ipc.appStats).then((s) => s && setStats(s));
                  })
                }
              >
                Yes, purge everything
              </button>
              <button onClick={() => setConfirmPurge(false)}>Cancel</button>
            </div>
          ) : (
            <button onClick={() => setConfirmPurge(true)}>Purge index…</button>
          )}
        </div>
      )}
    </section>
  );
}
