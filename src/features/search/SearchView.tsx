import { useEffect, useRef, useState } from "react";
import { ipc, isIpcError, type SearchHit } from "../../lib/ipc";
import { formatBytes, formatDate, splitSnippet } from "../../lib/format";

const DEBOUNCE_MS = 120;

export function SearchView() {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<number>(-1);
  const latest = useRef(0);

  useEffect(() => {
    const id = ++latest.current;
    const q = query.trim();
    if (!q) {
      setHits([]);
      setSelected(-1);
      return;
    }
    const t = setTimeout(async () => {
      try {
        const res = await ipc.search(q, 100, 0);
        if (latest.current === id) {
          setHits(res);
          setSelected(res.length ? 0 : -1);
          setError(null);
        }
      } catch (e) {
        if (latest.current === id) setError(isIpcError(e) ? e.message : String(e));
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [query]);

  function onKey(e: React.KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((s) => Math.min(hits.length - 1, s + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((s) => Math.max(0, s - 1));
    } else if (e.key === "Enter" && selected >= 0) {
      void ipc.fileOpen(hits[selected].id);
    }
  }

  return (
    <section className="search" aria-label="Search">
      <input
        className="search-input"
        type="search"
        autoFocus
        placeholder='Search names and contents… (e.g. ferrari, "machine learning")'
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={onKey}
        aria-label="Search query"
      />
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <ul className="results" role="listbox" aria-label="Results">
        {hits.map((h, i) => (
          <li
            key={h.id}
            role="option"
            aria-selected={i === selected}
            className={`result ${i === selected ? "selected" : ""}`}
            onClick={() => setSelected(i)}
            onDoubleClick={() => ipc.fileOpen(h.id)}
          >
            <span className={`cat cat-${h.category}`} aria-hidden>
              {h.ext || h.category}
            </span>
            <div className="result-main">
              <div className="result-name">{h.name}</div>
              <div className="result-path muted" title={h.path}>
                {h.path}
              </div>
              {h.snippet && h.snippet !== h.name && (
                <div className="result-snippet">
                  {splitSnippet(h.snippet).map((s, k) =>
                    s.hit ? <mark key={k}>{s.text}</mark> : <span key={k}>{s.text}</span>,
                  )}
                </div>
              )}
            </div>
            <div className="result-meta muted">
              <div>{formatBytes(h.size)}</div>
              <div>{formatDate(h.mtime)}</div>
            </div>
            <button
              className="reveal"
              onClick={(e) => {
                e.stopPropagation();
                void ipc.fileReveal(h.id);
              }}
              title="Show in Explorer"
            >
              ↗
            </button>
          </li>
        ))}
        {query.trim() && hits.length === 0 && !error && (
          <li className="muted empty">No matches.</li>
        )}
      </ul>
    </section>
  );
}
