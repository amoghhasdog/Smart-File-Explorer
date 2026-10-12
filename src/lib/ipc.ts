// Typed wrappers around Tauri IPC. This is the only module that imports
// `@tauri-apps/api`; components depend on these functions so tests can mock
// one place.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
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
  waitingForModel: number;
  scanning: RootId[];
  paused: boolean;
  lastError: string | null;
}

export type ChipKind =
  | "type"
  | "ext"
  | "size"
  | "duration"
  | "date"
  | "color"
  | "dimensions"
  | "phrase"
  | "about"
  | "path"
  | "terms";

export interface Chip {
  kind: ChipKind;
  label: string;
}

/** Parsed query as returned by the backend; only the parts the UI uses. */
export interface ParsedQuery {
  raw: string;
  terms: string[];
  phrases: string[];
  semantic: string | null;
  chips: Chip[];
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
  snippet: string | null;
  score: number;
  width: number | null;
  height: number | null;
  durationMs: number | null;
}

export interface SearchResponse {
  query: ParsedQuery;
  hits: SearchHit[];
  tookMs: number;
  hasMore: boolean;
}

export interface FileRecord {
  id: FileId;
  root_id: RootId;
  path: string;
  parent_path: string;
  name: string;
  ext: string;
  kind: "file" | "dir";
  category: Category;
  size: number;
  mtime: number;
  ctime: number | null;
  mime: string | null;
  index_state: "discovered" | "metadata" | "text" | "embedded" | "failed";
  index_error: string | null;
}

export interface FileMeta {
  width: number | null;
  height: number | null;
  duration_ms: number | null;
  page_count: number | null;
  word_count: number | null;
  camera_make: string | null;
  taken_at: number | null;
  gps_lat: number | null;
  gps_lon: number | null;
  dominant_colors: string | null;
  title: string | null;
  author: string | null;
}

export interface FileDetails {
  record: FileRecord;
  meta: FileMeta | null;
  textExcerpt: string | null;
  textChars: number | null;
  extractor: string | null;
}

export type ModelKind = "text" | "image";

export interface ModelStatus {
  id: string;
  kind: ModelKind;
  display: string;
  approxBytes: number;
  installed: boolean;
  ready: boolean;
  installing: boolean;
  available: boolean;
}

export interface ModelProgress {
  id: string;
  bytes: number;
  approxTotal: number;
  done: boolean;
  error: string | null;
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
    invoke<SearchResponse>("search", { query, limit, offset }),
  searchExplain: (query: string) => invoke<ParsedQuery>("search_explain", { query }),

  fileDetails: (id: FileId) => invoke<FileDetails | null>("file_details", { id }),
  fileOpen: (id: FileId) => invoke<void>("file_open", { id }),
  fileReveal: (id: FileId) => invoke<void>("file_reveal", { id }),

  modelsStatus: () => invoke<ModelStatus[]>("models_status"),
  modelsInstall: (kind: ModelKind) => invoke<void>("models_install", { kind }),

  /** URL of a file's thumbnail (served by the `thumb` protocol). */
  thumbUrl: (id: FileId) => convertFileSrc(String(id), "thumb"),

  onIndexStatus: (cb: (s: IndexStatus) => void): Promise<UnlistenFn> =>
    listen<IndexStatus>("index-status", (e) => cb(e.payload)),
  onModelProgress: (cb: (p: ModelProgress) => void): Promise<UnlistenFn> =>
    listen<ModelProgress>("model-progress", (e) => cb(e.payload)),
};
