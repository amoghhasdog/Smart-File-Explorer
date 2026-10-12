import { useEffect, useState } from "react";
import { ipc, type IndexStatus } from "./lib/ipc";
import { FoldersPanel } from "./features/folders/FoldersPanel";
import { ModelsCard } from "./features/models/ModelsCard";
import { StatusBar } from "./features/indexing/StatusBar";
import { SearchView } from "./features/search/SearchView";
import "./App.css";

export default function App() {
  const [status, setStatus] = useState<IndexStatus | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    ipc
      .onIndexStatus((s) => {
        if (!cancelled) setStatus(s);
      })
      .then((u) => {
        unlisten = u;
      });
    ipc
      .indexStatus()
      .then((s) => {
        if (!cancelled) setStatus((cur) => cur ?? s);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return (
    <div className="app">
      <aside className="sidebar">
        <h1 className="brand">Smart File Explorer</h1>
        <FoldersPanel status={status} />
        <ModelsCard />
      </aside>
      <main className="main">
        <SearchView />
      </main>
      <StatusBar status={status} />
    </div>
  );
}
