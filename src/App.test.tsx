import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { IndexStatus, Root, SearchHit } from "./lib/ipc";

const roots: Root[] = [];
const status: IndexStatus = {
  totalFiles: 3,
  discovered: 0,
  metadata: 3,
  text: 0,
  embedded: 0,
  failed: 0,
  pendingJobs: 0,
  scanning: [],
  paused: false,
  lastError: null,
};
const hit: SearchHit = {
  id: 7,
  path: "C:\\docs\\Ferrari notes.md",
  name: "Ferrari notes.md",
  ext: "md",
  category: "document",
  size: 2048,
  mtime: 1_700_000_000,
  mime: "text/markdown",
  snippet: "[Ferrari] notes.md",
};

const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
  switch (cmd) {
    case "roots_list":
      return roots;
    case "roots_pick_and_add": {
      const r: Root = { id: 1, path: "C:\\docs", added_at: 0, enabled: true, last_scan_at: null };
      roots.push(r);
      return r;
    }
    case "roots_remove":
      roots.splice(0, roots.length);
      return null;
    case "index_status":
      return status;
    case "search":
      return (args?.query as string).startsWith("ferr") ? [hit] : [];
    case "file_open":
    case "file_reveal":
      return null;
    default:
      throw new Error(`unexpected command ${cmd}`);
  }
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: (c: string, a?: Record<string, unknown>) => invoke(c, a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import App from "./App";

beforeEach(() => {
  roots.splice(0, roots.length);
  invoke.mockClear();
});

describe("App", () => {
  it("shows the index status and lets the user add and remove a folder", async () => {
    render(<App />);
    expect(await screen.findByText(/3 files indexed/)).toBeTruthy();
    expect(screen.getByText(/add a folder to start/i)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /add folder/i }));
    expect(await screen.findByText("C:\\docs")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /remove C:\\docs/i }));
    await waitFor(() => expect(screen.queryByText("C:\\docs")).toBeNull());
  });

  it("searches as you type and opens the selected hit on Enter", async () => {
    render(<App />);
    const input = screen.getByRole("searchbox", { name: /search query/i });
    fireEvent.change(input, { target: { value: "ferr" } });
    expect(await screen.findByText("Ferrari notes.md")).toBeTruthy();
    expect(screen.getByText("2.0 KB")).toBeTruthy();
    expect(screen.getByText("Ferrari", { selector: "mark" })).toBeTruthy();

    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("file_open", { id: 7 }));

    fireEvent.change(input, { target: { value: "zzz" } });
    expect(await screen.findByText(/no matches/i)).toBeTruthy();
  });
});
