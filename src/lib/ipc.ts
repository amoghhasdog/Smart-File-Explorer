// Typed wrappers around Tauri IPC. This is the only module that imports
// `@tauri-apps/api`; components depend on these functions so tests can mock
// one place.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type RootId = number;
export type FileId = number;

export type Category = "document" | "image" | "video" | "audio" | "code" | "archive" | "other";

export interface Root {
  id: RootId;
  path: string;
  added_at: number;
  enabled: boolean;
  last_scan_at: number | null;
}

export interface IndexStatus {
  totalFiles: number;
  discovered: number;
  metadata: number;
  text: number;
  embedded: number;
  failed: number;
  pendingJobs: number;
  scanning: RootId[];
  paused: boolean;
  lastError: string | null;
}

export interface SearchHit {
  id: FileId;
  path: string;
  name: string;
  ext: string;
  category: Category;
  size: number;
  mtime: number;
  mime: string | null;
  snippet: string;
}

export interface IpcError {
  code: string;
  message: string;
}

export function isIpcError(e: unknown): e is IpcError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

export const ipc = {
  appVersion: () => invoke<string>("app_version"),
  dataDir: () => invoke<string>("data_dir"),

  rootsList: () => invoke<Root[]>("roots_list"),
  rootsPickAndAdd: () => invoke<Root | null>("roots_pick_and_add"),
  rootsRemove: (id: RootId) => invoke<void>("roots_remove", { id }),

  indexStatus: () => invoke<IndexStatus>("index_status"),
  indexPause: () => invoke<void>("index_pause"),
  indexResume: () => invoke<void>("index_resume"),
  indexRescan: (id: RootId) => invoke<void>("index_rescan", { id }),

  search: (query: string, limit = 50, offset = 0) =>
    invoke<SearchHit[]>("search", { query, limit, offset }),

  fileOpen: (id: FileId) => invoke<void>("file_open", { id }),
  fileReveal: (id: FileId) => invoke<void>("file_reveal", { id }),

  onIndexStatus: (cb: (s: IndexStatus) => void): Promise<UnlistenFn> =>
    listen<IndexStatus>("index-status", (e) => cb(e.payload)),
};
