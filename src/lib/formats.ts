// Pure helpers for presenting what an analysis returned: which formats each
// mode offers, how they are labelled and filtered, and the file name preview.
// No calls out and no DOM here, so every rule can be tested on its own.
import type { DownloadMode } from "./backend";

export interface VideoFormat {
  format_id: string;
  ext: string;
  resolution: string;
  height: number | null;
  fps: number | null;
  vcodec: string;
  acodec: string;
  abr: number | null;
  tbr: number | null;
  filesize: number | null;
  filesize_approx: number | null;
  format_note: string;
  has_video: boolean;
  has_audio: boolean;
  size_label: string;
  quality_rank: number;
}

export interface VideoInfo {
  title: string;
  duration: number | null;
  duration_label: string;
  uploader: string;
  thumbnail: string | null;
  formats: VideoFormat[];
}

export const CONTAINER_FORMATS = [
  { value: "mp4", label: "MP4 (recomendado)" },
  { value: "mkv", label: "MKV" },
  { value: "original", label: "Original (sem conversão)" },
] as const;

export const AUDIO_EXPORT_FORMATS = [
  { value: "original", label: "Original (sem conversão)" },
  { value: "mp3", label: "MP3" },
  { value: "m4a", label: "M4A (AAC)" },
  { value: "wav", label: "WAV" },
  { value: "flac", label: "FLAC" },
  { value: "opus", label: "OPUS" },
] as const;

export const DEFAULT_PATTERN = "%(title).180B [%(id)s] [%(format_id)s].%(ext)s";

export const PATTERN_PRESETS = [
  { label: "Título + ID + Formato (recomendado)", pattern: DEFAULT_PATTERN },
  { label: "Somente título", pattern: "%(title).180B.%(ext)s" },
  { label: "Canal - Título", pattern: "%(uploader)s - %(title).160B.%(ext)s" },
] as const;

const UNKNOWN_SIZE = "desconhecido";
const FRIENDLY_RESOLUTION_NAMES: Record<number, string> = {
  4320: "8K",
  2160: "4K",
  1440: "2K",
  1080: "Full HD",
  720: "HD",
  480: "SD",
};

const byQuality = (formats: VideoFormat[]) =>
  [...formats].sort((a, b) => b.quality_rank - a.quality_rank);

export function resolutionPhrase(format: VideoFormat): string {
  if (!format.height) return format.resolution || "Resolução desconhecida";
  const friendly = FRIENDLY_RESOLUTION_NAMES[format.height];
  return friendly ? `${friendly} - ${format.height}p` : `${format.height}p`;
}

/** e.g. "Full HD - 1080p, 60fps, 30.0 MB, MP4" */
export function videoOptionLabel(format: VideoFormat): string {
  const bits = [resolutionPhrase(format)];
  if (format.fps) bits.push(`${Math.trunc(format.fps)}fps`);
  bits.push(format.size_label, format.ext.toUpperCase());
  return bits.join(", ") + (format.has_audio ? "" : "  (requer FFmpeg)");
}

/** e.g. "160kbps, 4.3 MB, OPUS" */
export function audioOptionLabel(format: VideoFormat): string {
  const bits = format.abr ? [`${Math.trunc(format.abr)}kbps`] : [];
  bits.push(format.size_label, format.ext.toUpperCase());
  return bits.join(", ");
}

/** e.g. "160 kbps · opus (webm) · 4.3 MB" */
export function audioSummary(format: VideoFormat): string {
  const bits = format.abr ? [`${Math.trunc(format.abr)} kbps`] : [];
  bits.push(`${format.acodec} (${format.ext})`);
  if (format.size_label !== UNKNOWN_SIZE) bits.push(format.size_label);
  return bits.join(" · ");
}

/** The real stream behind an option: yt-dlp format ID and codecs. */
export function formatDetail(format: VideoFormat): string {
  const bits = [`ID ${format.format_id}`];
  if (format.has_video) bits.push(`vídeo ${format.vcodec}`);
  bits.push(format.has_audio ? `áudio ${format.acodec}` : "sem áudio");
  return bits.join(" · ");
}

/** Video formats offered by a mode, best first. "Somente vídeo" hides muxed streams. */
export function videoFormatsFor(info: VideoInfo, mode: DownloadMode): VideoFormat[] {
  const videos = info.formats.filter((f) => f.has_video);
  return byQuality(mode === "video" ? videos.filter((f) => !f.has_audio) : videos);
}

export function audioFormatsFor(info: VideoInfo): VideoFormat[] {
  return byQuality(info.formats.filter((f) => f.has_audio && !f.has_video));
}

/** Filter choices built only from what this video really has, highest first. */
export function filterOptions(formats: VideoFormat[]): { resolutions: string[]; fps: number[] } {
  const heights = new Map<string, number>();
  const fps = new Set<number>();
  for (const format of formats) {
    const phrase = resolutionPhrase(format);
    if (!heights.has(phrase)) heights.set(phrase, format.height ?? 0);
    if (format.fps) fps.add(Math.trunc(format.fps));
  }
  return {
    resolutions: [...heights].sort((a, b) => b[1] - a[1]).map(([phrase]) => phrase),
    fps: [...fps].sort((a, b) => b - a),
  };
}

/** `resolution` and `fps` are filter choices; an empty value means "all". */
export function applyFilters(
  formats: VideoFormat[],
  resolution: string,
  fps: string,
): VideoFormat[] {
  return formats.filter(
    (f) =>
      (!resolution || resolutionPhrase(f) === resolution) &&
      (!fps || (f.fps != null && Math.trunc(f.fps) === Number(fps))),
  );
}

/** Approximate, local rendering of the final file name for an output template. */
export function previewFilename(
  pattern: string,
  info: VideoInfo | null,
  formatId?: string,
  ext?: string,
): string {
  const title = info?.title ?? "Nome do Vídeo de Exemplo";
  return pattern
    .replaceAll("%(title).180B", title.slice(0, 60))
    .replaceAll("%(title)s", title)
    .replaceAll("%(id)s", "abcd1234XYZ")
    .replaceAll("%(format_id)s", formatId ?? "137")
    .replaceAll("%(ext)s", ext ?? "mp4")
    .replaceAll("%(uploader)s", info?.uploader || "Canal");
}
