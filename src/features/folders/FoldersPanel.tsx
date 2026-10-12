import { useCallback, useEffect, useState } from "react";
import { ipc, isIpcError, type IndexStatus, type Root } from "../../lib/ipc";

interface Props {
  status: IndexStatus | null;
}

export function FoldersPanel({ status }: Props) {
  const [roots, setRoots] = useState<Root[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setRoots(await ipc.rootsList());
    } catch (e) {
      setError(isIpcError(e) ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(isIpcError(e) ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel folders" aria-label="Indexed folders">
      <header className="panel-header">
        <h2>Folders</h2>
        <button onClick={() => run(ipc.rootsPickAndAdd)} disabled={busy}>
          Add folder…
        </button>
      </header>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {roots.length === 0 ? (
        <p className="muted empty">Add a folder to start indexing.</p>
      ) : (
        <ul className="root-list">
          {roots.map((r) => {
            const scanning = status?.scanning.includes(r.id) ?? false;
            return (
              <li key={r.id} className="root">
                <div className="root-path" title={r.path}>
                  {r.path}
                  {scanning && <span className="badge">scanning</span>}
                </div>
                <div className="root-actions">
                  <button onClick={() => run(() => ipc.indexRescan(r.id))} disabled={busy || scanning}>
                    Rescan
                  </button>
                  <button
                    onClick={() => run(() => ipc.rootsRemove(r.id))}
                    disabled={busy}
                    aria-label={`Remove ${r.path}`}
                  >
                    Remove
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
