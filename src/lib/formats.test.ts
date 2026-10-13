import { describe, expect, it } from "vitest";

import {
  DEFAULT_PATTERN,
  applyFilters,
  audioFormatsFor,
  audioOptionLabel,
  audioSummary,
  filterOptions,
  formatDetail,
  previewFilename,
  resolutionPhrase,
  videoFormatsFor,
  videoOptionLabel,
  type VideoFormat,
  type VideoInfo,
} from "./formats";

function format(overrides: Partial<VideoFormat>): VideoFormat {
  const base: VideoFormat = {
    format_id: "137",
    ext: "mp4",
    resolution: "1920x1080",
    height: 1080,
    fps: 60,
    vcodec: "avc1",
    acodec: "none",
    abr: null,
    tbr: 3000,
    filesize: null,
    filesize_approx: null,
    format_note: "",
    has_video: true,
    has_audio: false,
    size_label: "30.0 MB",
    quality_rank: 3000,
  };
  return { ...base, ...overrides };
}

const muxed = format({
  format_id: "18",
  height: 360,
  fps: 30,
  acodec: "mp4a",
  has_audio: true,
  size_label: "9.0 MB",
  quality_rank: 700,
});
const silent1080 = format({ format_id: "137" });
const silent720 = format({
  format_id: "136",
  height: 720,
  fps: 30,
  quality_rank: 1500,
  size_label: "15.0 MB",
});
const audioTrack = format({
  format_id: "251",
  ext: "webm",
  resolution: "audio only",
  height: null,
  fps: null,
  vcodec: "none",
  acodec: "opus",
  abr: 160,
  tbr: 160,
  has_video: false,
  has_audio: true,
  size_label: "4.3 MB",
  quality_rank: 160,
});

const info: VideoInfo = {
  title: "Um Título Bem Longo Para Testar o Recorte do Padrão de Nome de Arquivo",
  duration: 213,
  duration_label: "03:33",
  uploader: "Canal de Teste",
  thumbnail: null,
  formats: [silent720, muxed, silent1080, audioTrack],
};

describe("which formats each mode offers", () => {
  it('hides muxed streams in "Somente vídeo"', () => {
    const ids = videoFormatsFor(info, "video").map((f) => f.format_id);
    expect(ids).toEqual(["137", "136"]);
  });

  it("keeps muxed streams for the other video modes, best first", () => {
    const ids = videoFormatsFor(info, "combined").map((f) => f.format_id);
    expect(ids).toEqual(["137", "136", "18"]);
  });

  it("offers only audio-only streams as audio", () => {
    expect(audioFormatsFor(info).map((f) => f.format_id)).toEqual(["251"]);
  });
});

describe("labels", () => {
  it("names the well-known resolutions", () => {
    expect(resolutionPhrase(silent1080)).toBe("Full HD - 1080p");
    expect(resolutionPhrase(silent720)).toBe("HD - 720p");
    expect(resolutionPhrase(audioTrack)).toBe("audio only");
  });

  it("warns that a silent stream needs FFmpeg", () => {
    expect(videoOptionLabel(silent1080)).toBe("Full HD - 1080p, 60fps, 30.0 MB, MP4  (requer FFmpeg)");
    expect(videoOptionLabel(muxed)).toBe("360p, 30fps, 9.0 MB, MP4");
  });

  it("describes an audio stream by its bitrate", () => {
    expect(audioOptionLabel(audioTrack)).toBe("160kbps, 4.3 MB, WEBM");
    expect(audioSummary(audioTrack)).toBe("160 kbps · opus (webm) · 4.3 MB");
  });

  it("spells out the stream behind an option", () => {
    expect(formatDetail(silent1080)).toBe("ID 137 · vídeo avc1 · sem áudio");
    expect(formatDetail(audioTrack)).toBe("ID 251 · áudio opus");
  });
});

describe("filters", () => {
  const pool = videoFormatsFor(info, "combined");

  it("offers only what this video really has, highest first", () => {
    expect(filterOptions(pool)).toEqual({
      resolutions: ["Full HD - 1080p", "HD - 720p", "360p"],
      fps: [60, 30],
    });
  });

  it("treats an empty choice as no filter", () => {
    expect(applyFilters(pool, "", "")).toHaveLength(3);
  });

  it("combines resolution and fps", () => {
    expect(applyFilters(pool, "Full HD - 1080p", "60").map((f) => f.format_id)).toEqual(["137"]);
    expect(applyFilters(pool, "Full HD - 1080p", "30")).toHaveLength(0);
  });
});

describe("file name preview", () => {
  it("fills in the template with the analysed video", () => {
    expect(previewFilename(DEFAULT_PATTERN, info, "137", "mp4")).toBe(
      "Um Título Bem Longo Para Testar o Recorte do Padrão de Nome  [abcd1234XYZ] [137].mp4",
    );
  });

  it("uses the channel and falls back when nothing was analysed", () => {
    expect(previewFilename("%(uploader)s - %(title)s.%(ext)s", info, "251", "mp3")).toBe(
      `Canal de Teste - ${info.title}.mp3`,
    );
    expect(previewFilename("%(title)s.%(ext)s", null)).toBe("Nome do Vídeo de Exemplo.mp4");
  });
});
