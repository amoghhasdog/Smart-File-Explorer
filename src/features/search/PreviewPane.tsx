import { useEffect, useState } from "react";
import { ipc, isIpcError, type FileDetails, type FileId } from "../../lib/ipc";
import {
  formatBytes,
  formatDateTime,
  formatDuration,
  parseColors,
} from "../../lib/format";

interface Props {
  fileId: FileId | null;
}

export function PreviewPane({ fileId }: Props) {
  const [details, setDetails] = useState<FileDetails | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    if (fileId == null) {
      setDetails(null);
      return;
    }
    ipc
      .fileDetails(fileId)
      .then((d) => {
        if (!cancelled) setDetails(d);
      })
      .catch((e) => {
        if (!cancelled) setError(isIpcError(e) ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [fileId]);

  if (fileId == null) {
    return (
      <aside className="preview muted" aria-label="Preview">
        Select a result to see details.
      </aside>
    );
  }
  if (error) {
    return (
      <aside className="preview" aria-label="Preview">
        <p role="alert" className="error">
          {error}
        </p>
      </aside>
    );
  }
  if (!details) {
    return <aside className="preview muted" aria-label="Preview">Loading…</aside>;
  }

  const { record: r, meta: m } = details;
  const colors = parseColors(m?.dominant_colors);
  const rows: [string, string][] = [
    ["Size", formatBytes(r.size)],
    ["Modified", formatDateTime(r.mtime)],
    ["Created", formatDateTime(r.ctime)],
    ["Type", r.mime ?? r.category],
  ];
  if (m?.width && m?.height) rows.push(["Dimensions", `${m.width} × ${m.height}`]);
  if (m?.duration_ms != null) rows.push(["Duration", formatDuration(m.duration_ms)]);
  if (m?.page_count != null) rows.push(["Pages", String(m.page_count)]);
  if (m?.word_count != null) rows.push(["Words", m.word_count.toLocaleString()]);
  if (m?.taken_at) rows.push(["Taken", formatDateTime(m.taken_at)]);
  if (m?.camera_make) rows.push(["Camera", m.camera_make]);
  if (m?.title) rows.push(["Title", m.title]);
  if (m?.author) rows.push(["Author", m.author]);
  if (r.index_state === "failed" && r.index_error) rows.push(["Index error", r.index_error]);

  return (
    <aside className="preview" aria-label="Preview">
      <h3 className="preview-name" title={r.name}>
        {r.name}
      </h3>
      <div className="preview-path muted" title={r.path}>
        {r.parent_path}
      </div>
      <div className="preview-actions">
        <button onClick={() => ipc.fileOpen(r.id)}>Open</button>
        <button onClick={() => ipc.fileReveal(r.id)}>Show in Explorer</button>
      </div>
      {colors.length > 0 && (
        <div className="swatches" aria-label="Dominant colours">
          {colors.map((c, i) => (
            <span
              key={i}
              className="swatch"
              style={{ background: c.rgb, flex: Math.max(c.share, 0.05) }}
              title={`${c.rgb} ${Math.round(c.share * 100)}%`}
            />
          ))}
        </div>
      )}
      <dl className="meta">
        {rows
          .filter(([, v]) => v)
          .map(([k, v]) => (
            <div key={k} className="meta-row">
              <dt>{k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
      </dl>
      {details.textExcerpt && (
        <section className="excerpt" aria-label="Extracted text">
          <h4 className="muted">
            Text{details.textChars ? ` · ${details.textChars.toLocaleString()} chars` : ""}
            {details.extractor ? ` · ${details.extractor}` : ""}
          </h4>
          <pre>{details.textExcerpt}</pre>
        </section>
      )}
    </aside>
  );
}
