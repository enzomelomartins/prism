// The only module that talks to the Rust side. Everything the interface can
// make the computer do goes through one of these calls, which keeps the React
// components free of any knowledge of yt-dlp or the file system.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { VideoInfo } from "./formats";

export type DownloadMode = "combined" | "video" | "audio" | "separate";

export type JobState =
  | "idle"
  | "downloading"
  | "merging"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

/** States in which a download is still running and can be acted upon. */
export const ACTIVE_STATES: readonly JobState[] = ["downloading", "merging", "paused"];

export interface AppConfig {
  last_folder: string;
  container: string;
  audio_export_format: string;
  embed_metadata: boolean;
  embed_thumbnail: boolean;
  write_subs: boolean;
  sub_langs: string;
  filename_pattern: string;
}

export interface BackendStatus {
  /** Whether the yt-dlp executable has been found or downloaded. */
  ytdlp: boolean;
  /** FFmpeg: required to merge video with audio and to convert audio. */
  ffmpeg: boolean;
  js_runtime: boolean;
  default_folder: string;
  config: AppConfig;
}

export interface Progress {
  fraction: number | null;
  size: string;
  speed: string;
  eta: string;
}

export type JobEvent =
  | { type: "snapshot"; state: JobState; title: string; progress: Progress | null; log: string[] }
  | { type: "state"; state: JobState; title: string }
  | ({ type: "progress" } & Progress)
  | { type: "log"; line: string };

export interface DownloadRequest {
  url: string;
  mode: DownloadMode;
  videoFormatId: string | null;
  audioFormatId: string | null;
  folder: string;
  container: string;
  audioExportFormat: string;
  embedMetadata: boolean;
  embedThumbnail: boolean;
  writeSubs: boolean;
  subLangs: string;
  filenamePattern: string;
}

/** The interface wording for the four modes, and what Rust calls each one. */
const BACKEND_MODES: Record<DownloadMode, string> = {
  combined: "video_audio",
  video: "video_only",
  audio: "audio_only",
  separate: "separate",
};

/** A rejected command carries the message the Rust side wrote, as a string. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  return error instanceof Error ? error.message : String(error);
}

export const fetchStatus = () => invoke<BackendStatus>("status");

/** The current state and log, so a reopened window catches up on a download. */
export const fetchSnapshot = () => invoke<JobEvent>("snapshot");

export const analyze = (url: string) => invoke<VideoInfo>("analyze", { url });

export const startDownload = (request: DownloadRequest) =>
  invoke<void>("start_download", {
    request: { ...request, mode: BACKEND_MODES[request.mode] },
  });

export const pauseDownload = () => invoke<void>("pause");
export const resumeDownload = () => invoke<void>("resume");
export const cancelDownload = () => invoke<void>("cancel");
export const openFolder = (folder: string) => invoke<void>("open_folder", { folder });

/** Opens the native folder picker; resolves to null when it is dismissed. */
export const browseFolder = (initial: string) =>
  invoke<string | null>("pick_folder", { initial });

/** Downloads yt-dlp and FFmpeg; `force` also re-downloads an existing yt-dlp. */
export const installTools = (force = false) => invoke<void>("install_tools", { force });

/**
 * Streams download state, progress and log lines, plus a `tools` notice after
 * an install so the header can refresh. The returned function detaches both.
 */
export function subscribe(
  onJob: (event: JobEvent) => void,
  onTools: (status: BackendStatus) => void,
): () => void {
  const listeners = [
    listen<JobEvent>("job", (event) => onJob(event.payload)),
    listen<BackendStatus>("tools", (event) => onTools(event.payload)),
  ];
  return () => {
    for (const listener of listeners) void listener.then((detach) => detach());
  };
}
