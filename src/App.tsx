import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

export default function App() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("app_version").then(setVersion).catch(() => setVersion("?"));
  }, []);

  return (
    <main className="shell">
      <h1>Smart File Explorer</h1>
      <p className="muted">Phase 0 skeleton · backend v{version ?? "…"}</p>
    </main>
  );
}
