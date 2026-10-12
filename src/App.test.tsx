import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { FileDetails, IndexStatus, Root, SearchHit, SearchResponse } from "./lib/ipc";

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
  score: 1,
  width: null,
  height: null,
  durationMs: null,
};
const details: FileDetails = {
  record: {
    id: 7,
    root_id: 1,
    path: hit.path,
    parent_path: "C:\\docs",
    name: hit.name,
    ext: "md",
    kind: "file",
    category: "document",
    size: 2048,
    mtime: 1_700_000_000,
    ctime: null,
    mime: "text/markdown",
    index_state: "text",
    index_error: null,
  },
  meta: {
    width: null,
    height: null,
    duration_ms: null,
    page_count: null,
    word_count: 12,
    camera_make: null,
    taken_at: null,
    gps_lat: null,
    gps_lon: null,
    dominant_colors: null,
    title: null,
    author: null,
  },
  textExcerpt: "The Ferrari F40 is red.",
  textChars: 23,
  extractor: "plain",
};

function response(query: string, hits: SearchHit[]): SearchResponse {
  return {
    query: {
      raw: query,
      terms: [query],
      phrases: [],
      semantic: query,
      chips: [{ kind: "terms", label: query }],
    },
    hits,
    tookMs: 3,
    hasMore: false,
  };
}

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
    case "search": {
      const q = args?.query as string;
      return response(q, q.startsWith("ferr") ? [hit] : []);
    }
    case "search_explain":
      return response(args?.query as string, []).query;
    case "file_details":
      return details;
    case "file_open":
    case "file_reveal":
      return null;
    default:
      throw new Error(`unexpected command ${cmd}`);
  }
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (c: string, a?: Record<string, unknown>) => invoke(c, a),
}));
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

  it("searches as you type, shows chips and preview, opens on Enter", async () => {
    render(<App />);
    const input = screen.getByRole("searchbox", { name: /search query/i });
    fireEvent.change(input, { target: { value: "ferr" } });
    expect((await screen.findAllByText("Ferrari notes.md")).length).toBeGreaterThan(0);
    expect(screen.getByText("2.0 KB", { selector: ".result-meta div" })).toBeTruthy();
    expect(screen.getByText("Ferrari", { selector: "mark" })).toBeTruthy();
    expect(screen.getByText("ferr", { selector: ".chip" })).toBeTruthy();

    // preview pane loads the selected hit's details
    expect(await screen.findByText("The Ferrari F40 is red.")).toBeTruthy();
    expect(screen.getByText("12")).toBeTruthy();

    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("file_open", { id: 7 }));

    fireEvent.change(input, { target: { value: "zzz" } });
    expect(await screen.findByText(/no matches/i)).toBeTruthy();
  });
});
