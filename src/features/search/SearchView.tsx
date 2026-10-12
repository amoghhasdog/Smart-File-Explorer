import { useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ipc, isIpcError, type Chip, type SearchHit } from "../../lib/ipc";
import { formatBytes, formatDate, formatDuration, splitSnippet } from "../../lib/format";
import { QueryChips } from "./QueryChips";
import { PreviewPane } from "./PreviewPane";
import { Thumb } from "./Thumb";

const DEBOUNCE_MS = 120;
const PAGE = 100;
const ROW_HEIGHT = 64;

export function SearchView() {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [chips, setChips] = useState<Chip[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [tookMs, setTookMs] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<number>(-1);
  const latest = useRef(0);
  const listRef = useRef<HTMLDivElement>(null);

  // Chips update instantly from the parser; results follow after a debounce.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setChips([]);
      return;
    }
    ipc
      .searchExplain(q)
      .then((p) => setChips(p.chips))
      .catch(() => {});
  }, [query]);

  useEffect(() => {
    const id = ++latest.current;
    const q = query.trim();
    if (!q) {
      setHits([]);
      setSelected(-1);
      setHasMore(false);
      setTookMs(null);
      return;
    }
    const t = setTimeout(async () => {
      try {
        const res = await ipc.search(q, PAGE, 0);
        if (latest.current !== id) return;
        setHits(res.hits);
        setChips(res.query.chips);
        setHasMore(res.hasMore);
        setTookMs(res.tookMs);
        setSelected(res.hits.length ? 0 : -1);
        setError(null);
      } catch (e) {
        if (latest.current === id) setError(isIpcError(e) ? e.message : String(e));
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [query]);

  async function loadMore() {
    const id = latest.current;
    try {
      const res = await ipc.search(query.trim(), PAGE, hits.length);
      if (latest.current !== id) return;
      setHits((h) => [...h, ...res.hits]);
      setHasMore(res.hasMore);
    } catch (e) {
      setError(isIpcError(e) ? e.message : String(e));
    }
  }

  const virtualizer = useVirtualizer({
    count: hits.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
    initialRect: { width: 800, height: 600 },
  });

  useEffect(() => {
    if (selected >= 0) virtualizer.scrollToIndex(selected, { align: "auto" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

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

  const selectedId = selected >= 0 && selected < hits.length ? hits[selected].id : null;

  return (
    <section className="search" aria-label="Search">
      <input
        className="search-input"
        type="search"
        autoFocus
        placeholder='Try "images with a lot of blue" or "PDFs about machine learning from last year"'
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={onKey}
        aria-label="Search query"
      />
      <div className="search-meta">
        <QueryChips chips={chips} />
        {tookMs != null && hits.length > 0 && (
          <span className="muted took">
            {hits.length}
            {hasMore ? "+" : ""} results · {tookMs} ms
          </span>
        )}
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <div className="search-body">
        <div className="results-scroll" ref={listRef}>
          {query.trim() && hits.length === 0 && !error && (
            <p className="muted empty">No matches.</p>
          )}
          <ul
            className="results"
            role="listbox"
            aria-label="Results"
            style={{ height: virtualizer.getTotalSize(), position: "relative" }}
          >
            {virtualizer.getVirtualItems().map((row) => {
              const h = hits[row.index];
              const isSel = row.index === selected;
              return (
                <li
                  key={h.id}
                  role="option"
                  aria-selected={isSel}
                  className={`result ${isSel ? "selected" : ""}`}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    right: 0,
                    height: row.size,
                    transform: `translateY(${row.start}px)`,
                  }}
                  onClick={() => setSelected(row.index)}
                  onDoubleClick={() => ipc.fileOpen(h.id)}
                >
                  {h.category === "image" ? (
                    <Thumb id={h.id} alt="" className="result-thumb" />
                  ) : (
                    <span className={`cat cat-${h.category}`} aria-hidden>
                      {h.ext || h.category}
                    </span>
                  )}
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
                    <div>
                      {h.width && h.height
                        ? `${h.width}×${h.height}`
                        : h.durationMs != null
                          ? formatDuration(h.durationMs)
                          : formatDate(h.mtime)}
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
          {hasMore && (
            <button className="load-more" onClick={loadMore}>
              Load more
            </button>
          )}
        </div>
        <PreviewPane fileId={selectedId} />
      </div>
    </section>
  );
}
