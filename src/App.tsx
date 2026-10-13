import { useEffect, useRef, useState } from "react";
import {
  ArrowDownToLine,
  ArrowUpRight,
  AudioLines,
  Check,
  CheckCheck,
  ChevronDown,
  ChevronRight,
  Clipboard,
  Clock,
  Copy,
  Download,
  Folder,
  FolderOpen,
  Gauge,
  HardDriveDownload,
  Link2,
  LoaderCircle,
  Maximize2,
  Minimize2,
  MonitorPlay,
  Music2,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
  Settings2,
  SquareTerminal,
  Video,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import * as api from "@/lib/backend";
import type { DownloadMode } from "@/lib/backend";
import {
  AUDIO_EXPORT_FORMATS,
  CONTAINER_FORMATS,
  DEFAULT_PATTERN,
  PATTERN_PRESETS,
  applyFilters,
  audioFormatsFor,
  audioOptionLabel,
  audioSummary,
  filterOptions,
  formatDetail,
  previewFilename,
  videoFormatsFor,
  videoOptionLabel,
  type VideoInfo,
} from "@/lib/formats";

const modes = [
  {
    id: "combined",
    name: "Vídeo com áudio",
    description: "Tudo junto, em um único arquivo.",
    icon: MonitorPlay,
  },
  {
    id: "video",
    name: "Somente vídeo",
    description: "A imagem, sem a faixa de áudio.",
    icon: Video,
  },
  {
    id: "audio",
    name: "Somente áudio",
    description: "Sua música, no formato que quiser.",
    icon: AudioLines,
  },
  {
    id: "separate",
    name: "Separados",
    description: "Vídeo e áudio em arquivos distintos.",
    icon: Copy,
  },
] as const;

const STATE_LABELS: Record<api.JobState, string> = {
  idle: "Aguardando",
  downloading: "Baixando…",
  merging: "Processando (mesclagem/conversão)…",
  paused: "Pausado",
  completed: "Concluído com sucesso",
  failed: "Falhou",
  cancelled: "Cancelado",
};

const LOG_LIMIT = 500;

interface Settings {
  folder: string;
  container: string;
  audioExport: string;
  embedMetadata: boolean;
  thumbnail: boolean;
  subtitles: boolean;
  subLangs: string;
  /** One of PATTERN_PRESETS, or "" when the custom pattern is in use. */
  patternPreset: string;
  customPattern: string;
}

interface Job {
  state: api.JobState;
  title: string;
  progress: api.Progress | null;
}

const idleJob: Job = { state: "idle", title: "", progress: null };
const initialSettings: Settings = {
  folder: "",
  container: "mp4",
  audioExport: "mp3",
  embedMetadata: true,
  thumbnail: false,
  subtitles: false,
  subLangs: "pt,en",
  patternPreset: DEFAULT_PATTERN,
  customPattern: DEFAULT_PATTERN,
};

const isHttpUrl = (value: string) => /^https?:\/\//i.test(value);

function siteName(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
}

function logTone(line: string): string | undefined {
  const lower = line.toLowerCase();
  if (lower.includes("error") || lower.includes("erro")) return "error";
  if (lower.includes("warning") || lower.includes("aviso")) return "warning";
  return line.startsWith(">>>") ? "success" : undefined;
}

function settingsFromConfig(config: api.AppConfig, fallbackFolder: string): Settings {
  const isPreset = PATTERN_PRESETS.some((preset) => preset.pattern === config.filename_pattern);
  return {
    folder: config.last_folder || fallbackFolder,
    container: config.container,
    audioExport: config.audio_export_format,
    embedMetadata: config.embed_metadata,
    thumbnail: config.embed_thumbnail,
    subtitles: config.write_subs,
    subLangs: config.sub_langs,
    patternPreset: isPreset ? config.filename_pattern : "",
    customPattern: config.filename_pattern,
  };
}

function Thumbnail({ info }: { info: VideoInfo | null }) {
  const [failed, setFailed] = useState(false);
  return (
    <div className="thumbnail">
      {info?.thumbnail && !failed ? (
        <img
          src={info.thumbnail}
          alt=""
          width={192}
          height={108}
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
        />
      ) : (
        <div className="flex h-full items-center justify-center bg-secondary text-primary">
          <Play size={30} />
        </div>
      )}
      {info?.duration ? <span className="duration">{info.duration_label}</span> : null}
    </div>
  );
}

export default function App() {
  // undefined until the first status answer arrives.
  const [backend, setBackend] = useState<api.BackendStatus | undefined>(undefined);
  const [url, setUrl] = useState("");
  const [analyzedUrl, setAnalyzedUrl] = useState("");
  const [info, setInfo] = useState<VideoInfo | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [mode, setMode] = useState<DownloadMode>("combined");
  const [settings, setSettings] = useState(initialSettings);
  const [filters, setFilters] = useState({ resolution: "", fps: "" });
  const [videoId, setVideoId] = useState("");
  const [audioId, setAudioId] = useState("");
  const [job, setJob] = useState(idleJob);
  const [progressClosed, setProgressClosed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [linkError, setLinkError] = useState("");
  const [actionError, setActionError] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const folderRef = useRef<HTMLInputElement>(null);
  const logRef = useRef<HTMLPreElement>(null);
  const analyzeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  // Last link sent for analysis: answers for any older link are ignored.
  const requestedUrl = useRef("");

  const update = (patch: Partial<Settings>) => setSettings((current) => ({ ...current, ...patch }));

  function applyJobEvent(event: api.JobEvent) {
    if (event.type === "snapshot") {
      setJob({ state: event.state, title: event.title, progress: event.progress });
      setLogs(event.log);
    } else if (event.type === "state") {
      setJob((current) => ({ ...current, state: event.state, title: event.title }));
      setProgressClosed(false);
    } else if (event.type === "progress") {
      const { fraction, size, speed, eta } = event;
      setJob((current) => ({ ...current, progress: { fraction, size, speed, eta } }));
    } else {
      setLogs((current) => [...current, event.line].slice(-LOG_LIMIT));
    }
  }

  useEffect(() => {
    let active = true;
    // Subscribing before the first read means nothing emitted in between is
    // lost: the snapshot carries whatever already happened.
    const unsubscribe = api.subscribe(
      (event) => active && applyJobEvent(event),
      (status) => {
        if (!active) return;
        setBackend(status);
        setInstalling(false);
      },
    );
    void (async () => {
      try {
        const [status, snapshot] = await Promise.all([api.fetchStatus(), api.fetchSnapshot()]);
        if (!active) return;
        setBackend(status);
        setSettings(settingsFromConfig(status.config, status.default_folder));
        applyJobEvent(snapshot);
      } catch (error) {
        if (active) setActionError(api.errorMessage(error));
      }
    })();
    return () => {
      active = false;
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    const element = logRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [logs]);

  const toolsReady = Boolean(backend?.ytdlp);
  const jobActive = api.ACTIVE_STATES.includes(job.state);

  const videoPool = info ? videoFormatsFor(info, mode) : [];
  const filterChoices = filterOptions(videoPool);
  // A filter kept from the previous video only applies if this one has that option too.
  const resolution = filterChoices.resolutions.includes(filters.resolution)
    ? filters.resolution
    : "";
  const fps = filters.fps && filterChoices.fps.includes(Number(filters.fps)) ? filters.fps : "";
  const videoChoices = applyFilters(videoPool, resolution, fps);
  const audioChoices = info ? audioFormatsFor(info) : [];
  const video = videoChoices.find((format) => format.format_id === videoId) ?? videoChoices[0];
  const audio = audioChoices.find((format) => format.format_id === audioId) ?? audioChoices[0];
  // "Separados" always pairs the chosen video with the best audio-only stream.
  const separateAudio = audioChoices[0];
  const chosen = mode === "audio" ? audio : video;

  const pattern = settings.patternPreset || settings.customPattern.trim() || "%(title)s.%(ext)s";
  const outputFormat =
    mode === "audio" ? settings.audioExport : mode === "separate" ? "original" : settings.container;
  const filenamePreview = previewFilename(
    pattern,
    info,
    chosen?.format_id,
    chosen && (outputFormat === "original" ? chosen.ext : outputFormat),
  );

  const ready = analyzedUrl !== "" && url.trim() === analyzedUrl && info !== null;
  const canDownload = ready && !busy && !jobActive && !analyzing && toolsReady;

  const tool = !backend
    ? { tone: "muted", text: "Verificando as ferramentas…" }
    : !backend.ytdlp
      ? { tone: "error", text: "yt-dlp não instalado" }
      : !backend.ffmpeg
        ? { tone: "warning", text: "FFmpeg não encontrado" }
        : { tone: undefined, text: "yt-dlp + FFmpeg prontos" };

  const missingTools = Boolean(backend && (!backend.ytdlp || !backend.ffmpeg));
  const readySummary = chosen
    ? mode === "audio"
      ? audioOptionLabel(chosen)
      : videoOptionLabel(chosen)
    : "";
  const statusText = analyzing
    ? "Analisando link…"
    : jobActive
      ? STATE_LABELS[job.state]
      : ready
        ? "Pronto para baixar"
        : "Aguardando um link";

  function changeUrl(value: string) {
    setUrl(value);
    setLinkError("");
    clearTimeout(analyzeTimer.current);
    // The analysis starts by itself shortly after a link is pasted or typed.
    const target = value.trim();
    if (toolsReady && isHttpUrl(target) && target.length > 10 && target !== requestedUrl.current) {
      analyzeTimer.current = setTimeout(() => void analyzeLink(target), 600);
    }
  }

  function clearUrl() {
    clearTimeout(analyzeTimer.current);
    requestedUrl.current = "";
    setUrl("");
    setAnalyzedUrl("");
    setInfo(null);
    setAnalyzing(false);
    setLinkError("");
  }

  async function paste() {
    try {
      changeUrl((await navigator.clipboard.readText()).trim());
    } catch {
      setLinkError("Não foi possível acessar a área de transferência. Cole o link no campo.");
    }
  }

  async function analyzeLink(target: string) {
    clearTimeout(analyzeTimer.current);
    if (!toolsReady) {
      setLinkError("Instale as ferramentas antes de analisar um link.");
      return;
    }
    if (!isHttpUrl(target)) {
      if (target) setLinkError("O link precisa começar com http:// ou https://");
      return;
    }
    requestedUrl.current = target;
    setLinkError("");
    setAnalyzedUrl("");
    setInfo(null);
    setAnalyzing(true);
    // A new link takes the place of the previous, already finished download.
    if (!jobActive) setProgressClosed(true);
    try {
      const result = await api.analyze(target);
      if (requestedUrl.current !== target) return;
      setInfo(result);
      setAnalyzedUrl(target);
    } catch (error) {
      if (requestedUrl.current !== target) return;
      setLinkError(api.errorMessage(error));
    } finally {
      if (requestedUrl.current === target) setAnalyzing(false);
    }
  }

  async function run(action: () => Promise<unknown>) {
    setActionError("");
    try {
      await action();
    } catch (error) {
      setActionError(api.errorMessage(error));
    }
  }

  async function installTools(force: boolean) {
    setInstalling(true);
    await run(() => api.installTools(force));
  }

  async function download() {
    if (!settings.folder.trim()) {
      setActionError("Informe a pasta de destino.");
      folderRef.current?.focus();
      return;
    }
    setBusy(true);
    await run(() =>
      api.startDownload({
        url: analyzedUrl,
        mode,
        videoFormatId: mode === "audio" ? null : (video?.format_id ?? null),
        audioFormatId:
          (mode === "audio" ? audio : mode === "separate" ? separateAudio : undefined)?.format_id ??
          null,
        folder: settings.folder,
        container: settings.container,
        audioExportFormat: settings.audioExport,
        embedMetadata: settings.embedMetadata,
        embedThumbnail: settings.thumbnail,
        writeSubs: settings.subtitles,
        subLangs: settings.subLangs,
        filenamePattern: pattern,
      }),
    );
    setBusy(false);
  }

  async function browseFolder() {
    await run(async () => {
      const folder = await api.browseFolder(settings.folder);
      if (folder) update({ folder });
    });
  }

  async function copyLogs() {
    try {
      await navigator.clipboard.writeText(logs.join("\n"));
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      setActionError("Não foi possível copiar o log.");
    }
  }

  const fraction = job.progress?.fraction ?? 0;

  return (
    <main className="app-shell">
      <div className="workspace">
        <header className="header-row flex items-center justify-between">
          <div className="flex items-center gap-4">
            <div className="brand-icon">
              <ArrowDownToLine size={25} strokeWidth={2.4} />
            </div>
            <div>
              <h1 className="heading">Prism</h1>
              <p className="text-muted-foreground mt-1 text-xs">
                Seus vídeos. Sua qualidade. Do seu jeito.
              </p>
            </div>
          </div>
          <div className="status" data-tone={tool.tone}>
            <span className="status-dot" /> {tool.text}
          </div>
        </header>

        {missingTools && (
          <section className="setup-panel" aria-label="Instalação das ferramentas">
            <div className="min-w-0 flex-1">
              <h2>
                {backend?.ytdlp ? "Falta o FFmpeg" : "Primeiro uso: falta baixar as ferramentas"}
              </h2>
              <p>
                {backend?.ytdlp
                  ? "O FFmpeg é necessário para juntar vídeo com áudio em alta resolução e para converter áudio. Ele é baixado para a pasta deste aplicativo."
                  : "O aplicativo baixa o yt-dlp e o FFmpeg para a própria pasta de dados. Não é preciso instalar mais nada no computador."}
              </p>
            </div>
            <Button variant="action" disabled={installing} onClick={() => void installTools(false)}>
              {installing ? <LoaderCircle className="animate-spin" /> : <Download />}
              {installing ? "Baixando…" : "Instalar ferramentas"}
            </Button>
          </section>
        )}

        <section className="mt-10" aria-label="Link do vídeo">
          <div className="section-label">
            <span>01</span> LINK DO VÍDEO
          </div>
          <form
            className="link-bar"
            onSubmit={(event) => {
              event.preventDefault();
              void analyzeLink(url.trim());
            }}
          >
            <Link2 className="text-muted-foreground shrink-0" size={17} />
            <input
              aria-label="Link do vídeo"
              placeholder="Cole o link do vídeo do YouTube ou de outras plataformas…"
              value={url}
              onChange={(event) => changeUrl(event.target.value)}
            />
            <div className="link-actions flex gap-2">
              <Button variant="ghost" size="sm" onClick={() => void paste()}>
                <Clipboard />
                Colar
              </Button>
              <Button
                variant="ghost"
                size="icon"
                title="Limpar link"
                aria-label="Limpar link"
                onClick={clearUrl}
              >
                <X />
              </Button>
              <Button variant="action" type="submit" disabled={analyzing || !toolsReady}>
                {analyzing ? <LoaderCircle className="animate-spin" /> : <Search />}
                {analyzing ? "Analisando" : "Analisar"}
              </Button>
            </div>
          </form>
          {linkError && (
            <p role="alert" className="text-destructive mt-3 text-xs whitespace-pre-wrap">
              {linkError}
            </p>
          )}
          {analyzing && (
            <div className="video-preview text-muted-foreground text-xs">
              <LoaderCircle className="animate-spin shrink-0" size={18} />
              Consultando metadados e formatos… o yt-dlp está processando o link.
            </div>
          )}
          {ready && info && (
            <div className="video-preview">
              <Thumbnail key={analyzedUrl} info={info} />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 mb-2">
                  <span className="video-tag">{siteName(analyzedUrl).toUpperCase()}</span>
                  <span className="text-muted-foreground text-[10px]">
                    {info.formats.length} formatos encontrados
                  </span>
                </div>
                <h2 className="video-title">{info.title}</h2>
                <div className="flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground mt-2">
                  {info.uploader && (
                    <>
                      <span>{info.uploader}</span>
                      <span className="opacity-40">/</span>
                    </>
                  )}
                  <span>Duração: {info.duration_label}</span>
                </div>
              </div>
              <CheckCheck size={20} className="text-primary mr-3 shrink-0 hidden sm:block" />
            </div>
          )}
        </section>

        <section className="mt-7" aria-label="Modo de download">
          <div className="section-label">
            <span>02</span> COMO VOCÊ QUER BAIXAR?
          </div>
          <div className="mode-grid" role="group" aria-label="Tipo de download">
            {modes.map((item) => (
              <Button
                key={item.id}
                variant="mode"
                data-selected={mode === item.id}
                aria-pressed={mode === item.id}
                onClick={() => setMode(item.id)}
              >
                <item.icon className="mode-icon" />
                {mode === item.id && <Check size={13} className="mode-check" />}
                <span className="mode-name">{item.name}</span>
                <span className="mode-description">{item.description}</span>
              </Button>
            ))}
          </div>
        </section>

        <section className="mt-7" aria-label="Configurações do download">
          <div className="section-label">
            <span>03</span> OS ÚLTIMOS DETALHES
          </div>
          <div className="settings-grid">
            <div className="setting-panel">
              <label className="field-label" htmlFor="format">
                {mode === "audio" ? "Qualidade do áudio" : "Qualidade do vídeo"}
              </label>
              {mode === "audio" ? (
                <select
                  className="field"
                  id="format"
                  value={audio?.format_id ?? ""}
                  disabled={!audio}
                  onChange={(event) => setAudioId(event.target.value)}
                >
                  {!audio && (
                    <option value="">
                      {info ? "Nenhum formato de áudio disponível" : "Aguardando link…"}
                    </option>
                  )}
                  {audioChoices.map((format) => (
                    <option key={format.format_id} value={format.format_id}>
                      {audioOptionLabel(format)}
                    </option>
                  ))}
                </select>
              ) : (
                <>
                  {videoPool.length > 0 && (
                    <div className="filters">
                      <select
                        className="field"
                        aria-label="Filtrar por resolução"
                        value={resolution}
                        onChange={(event) =>
                          setFilters({ ...filters, resolution: event.target.value })
                        }
                      >
                        <option value="">Todas as resoluções</option>
                        {filterChoices.resolutions.map((phrase) => (
                          <option key={phrase}>{phrase}</option>
                        ))}
                      </select>
                      <select
                        className="field"
                        aria-label="Filtrar por FPS"
                        value={fps}
                        onChange={(event) => setFilters({ ...filters, fps: event.target.value })}
                      >
                        <option value="">Todos os FPS</option>
                        {filterChoices.fps.map((value) => (
                          <option key={value} value={value}>
                            {value}fps
                          </option>
                        ))}
                      </select>
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={!resolution && !fps}
                        onClick={() => setFilters({ resolution: "", fps: "" })}
                      >
                        <X />
                        Limpar filtros
                      </Button>
                    </div>
                  )}
                  <select
                    className="field"
                    id="format"
                    value={video?.format_id ?? ""}
                    disabled={!video}
                    onChange={(event) => setVideoId(event.target.value)}
                  >
                    {!video && (
                      <option value="">
                        {!info
                          ? "Aguardando link…"
                          : videoPool.length
                            ? "Nenhum formato corresponde aos filtros"
                            : "Nenhum formato de vídeo disponível"}
                      </option>
                    )}
                    {videoChoices.map((format) => (
                      <option key={format.format_id} value={format.format_id}>
                        {videoOptionLabel(format)}
                      </option>
                    ))}
                  </select>
                </>
              )}
              {chosen && <p className="format-detail">{formatDetail(chosen)}</p>}
              <div className="conversion">
                <Music2 size={15} className="text-muted-foreground shrink-0" />
                {mode === "audio" ? (
                  <>
                    <label htmlFor="audio-format" className="text-xs text-muted-foreground flex-1">
                      Converter áudio para
                    </label>
                    <select
                      id="audio-format"
                      className="field"
                      value={settings.audioExport}
                      onChange={(event) => update({ audioExport: event.target.value })}
                    >
                      {AUDIO_EXPORT_FORMATS.map((format) => (
                        <option key={format.value} value={format.value}>
                          {format.label}
                        </option>
                      ))}
                    </select>
                  </>
                ) : mode === "separate" ? (
                  <span className="text-xs text-muted-foreground">
                    {separateAudio
                      ? `Áudio selecionado automaticamente: ${audioSummary(separateAudio)}`
                      : info
                        ? "Nenhuma faixa de áudio separada disponível para este link."
                        : "O melhor áudio disponível é escolhido automaticamente."}
                  </span>
                ) : (
                  <>
                    <label htmlFor="container" className="text-xs text-muted-foreground flex-1">
                      Formato de saída
                    </label>
                    <select
                      id="container"
                      className="field"
                      value={settings.container}
                      onChange={(event) => update({ container: event.target.value })}
                    >
                      {CONTAINER_FORMATS.map((format) => (
                        <option key={format.value} value={format.value}>
                          {format.label}
                        </option>
                      ))}
                    </select>
                  </>
                )}
              </div>
            </div>

            <div className="setting-panel">
              <label className="field-label flex items-center gap-2" htmlFor="folder">
                <Folder size={13} />
                Pasta de destino
              </label>
              <input
                ref={folderRef}
                id="folder"
                className="field"
                value={settings.folder}
                onChange={(event) => update({ folder: event.target.value })}
              />
              <div className="flex flex-wrap gap-2 mt-3">
                <Button size="sm" variant="secondary" onClick={() => void browseFolder()}>
                  <FolderOpen />
                  Procurar
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void run(() => api.openFolder(settings.folder))}
                >
                  <ArrowUpRight />
                  Abrir
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => update({ folder: backend?.default_folder ?? "" })}
                >
                  <RotateCcw />
                  Padrão
                </Button>
              </div>
              <div className="advanced">
                <Button
                  variant="ghost"
                  size="sm"
                  className="w-full justify-between px-0 h-5 text-muted-foreground"
                  onClick={() => setAdvanced(!advanced)}
                  aria-expanded={advanced}
                >
                  <span className="flex gap-2 items-center">
                    <Settings2 size={13} />
                    Configurações avançadas
                  </span>
                  {advanced ? <ChevronDown /> : <ChevronRight />}
                </Button>
                {advanced && (
                  <div className="flex flex-col gap-3 mt-4 text-xs text-muted-foreground">
                    <label className="check-row">
                      <input
                        className="accent-primary"
                        type="checkbox"
                        checked={settings.embedMetadata}
                        onChange={(event) => update({ embedMetadata: event.target.checked })}
                      />
                      Incorporar metadados (título, canal, etc.)
                    </label>
                    <label className="check-row">
                      <input
                        className="accent-primary"
                        type="checkbox"
                        checked={settings.thumbnail}
                        onChange={(event) => update({ thumbnail: event.target.checked })}
                      />
                      Incorporar miniatura como capa
                    </label>
                    <div className="check-row">
                      <label className="check-row">
                        <input
                          className="accent-primary"
                          type="checkbox"
                          checked={settings.subtitles}
                          onChange={(event) => update({ subtitles: event.target.checked })}
                        />
                        Baixar e incorporar legendas:
                      </label>
                      <input
                        className="field"
                        aria-label="Idiomas das legendas"
                        value={settings.subLangs}
                        onChange={(event) => update({ subLangs: event.target.value })}
                      />
                    </div>
                    <label className="field-label mt-2 mb-0" htmlFor="pattern">
                      Nome do arquivo
                    </label>
                    <select
                      id="pattern"
                      className="field"
                      value={settings.patternPreset}
                      onChange={(event) => update({ patternPreset: event.target.value })}
                    >
                      {PATTERN_PRESETS.map((preset) => (
                        <option key={preset.pattern} value={preset.pattern}>
                          {preset.label}
                        </option>
                      ))}
                      <option value="">Personalizado…</option>
                    </select>
                    {!settings.patternPreset && (
                      <input
                        className="field"
                        aria-label="Padrão personalizado do nome do arquivo"
                        value={settings.customPattern}
                        onChange={(event) => update({ customPattern: event.target.value })}
                      />
                    )}
                    <p className="preview-name">Prévia: {filenamePreview}</p>
                    <div className="advanced">
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={installing || jobActive}
                        title="Baixa a versão mais recente do yt-dlp"
                        onClick={() => void installTools(true)}
                      >
                        {installing ? <LoaderCircle className="animate-spin" /> : <RefreshCw />}
                        Atualizar o yt-dlp
                      </Button>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>
        </section>

        <div className="download-bar">
          <div className="flex items-center gap-2 text-xs min-w-0">
            <span className={ready ? "text-success" : "text-muted-foreground"}>
              {analyzing ? (
                <LoaderCircle className="animate-spin" size={16} />
              ) : ready ? (
                <Check size={16} />
              ) : (
                <Link2 size={16} />
              )}
            </span>
            <span className={ready ? "text-success shrink-0" : "text-muted-foreground shrink-0"}>
              {statusText}
            </span>
            <span className="text-muted-foreground text-[10px] ml-2 hidden sm:inline truncate">
              {ready && !jobActive ? readySummary : ""}
            </span>
          </div>
          <Button
            variant="action"
            className="min-w-48"
            onClick={() => void download()}
            disabled={!canDownload}
          >
            <ArrowDownToLine />
            Iniciar download
          </Button>
        </div>
        {actionError && (
          <p role="alert" className="text-destructive mt-3 text-xs whitespace-pre-wrap">
            {actionError}
          </p>
        )}

        {job.state !== "idle" && !progressClosed && (
          <section className="progress-panel" data-state={job.state} aria-label="Download atual">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="job-badge" aria-live="polite">
                {jobActive && job.state !== "paused" && (
                  <LoaderCircle className="animate-spin" size={14} />
                )}
                {STATE_LABELS[job.state]}
              </span>
              <div className="flex gap-1">
                {jobActive ? (
                  <>
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={job.state === "merging"}
                      title={
                        job.state === "merging"
                          ? "Não é possível pausar durante a mesclagem"
                          : undefined
                      }
                      onClick={() =>
                        void run(job.state === "paused" ? api.resumeDownload : api.pauseDownload)
                      }
                    >
                      {job.state === "paused" ? <Play /> : <Pause />}
                      {job.state === "paused" ? "Continuar" : "Pausar"}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => void run(api.cancelDownload)}>
                      <X />
                      Cancelar
                    </Button>
                  </>
                ) : (
                  <Button size="sm" variant="ghost" onClick={() => setProgressClosed(true)}>
                    <X />
                    Fechar
                  </Button>
                )}
              </div>
            </div>
            <p className="job-title">{job.title}</p>
            <div
              className="progress-track"
              role="progressbar"
              aria-label="Progresso do download"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(fraction * 100)}
            >
              <div className="progress-fill" style={{ width: `${fraction * 100}%` }} />
            </div>
            <div className="progress-stats">
              <span>{(fraction * 100).toFixed(1)}%</span>
              <span title="Baixado / total">
                <HardDriveDownload size={13} />
                {job.progress?.size ?? "-- / --"}
              </span>
              <span title="Velocidade">
                <Gauge size={13} />
                {job.progress?.speed ?? "--"}
              </span>
              <span title="Tempo restante">
                <Clock size={13} />
                {job.progress?.eta ?? "--"}
              </span>
            </div>
          </section>
        )}

        <section className="mt-6" aria-label="Log do processo">
          <div className="log-panel">
            <div className="log-header">
              <h2 className="text-xs font-semibold flex items-center gap-2">
                <SquareTerminal size={15} className="text-muted-foreground" />
                Log do processo
              </h2>
              <div className="flex gap-1">
                <Button variant="ghost" size="sm" onClick={() => void copyLogs()}>
                  {copied ? <Check /> : <Clipboard />}
                  {copied ? "Copiado" : "Copiar"}
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={expanded ? "Recolher log" : "Expandir log"}
                  title={expanded ? "Recolher log" : "Expandir log"}
                  onClick={() => setExpanded(!expanded)}
                >
                  {expanded ? <Minimize2 /> : <Maximize2 />}
                </Button>
              </div>
            </div>
            <pre ref={logRef} className={`log-content ${expanded ? "expanded" : ""}`}>
              {logs.length === 0 && "Nenhuma mensagem ainda."}
              {logs.map((line, index) => (
                <span key={index} data-tone={logTone(line)}>
                  {line}
                  {"\n"}
                </span>
              ))}
            </pre>
          </div>
        </section>
        <footer className="footer">
          <span className="flex items-center gap-1.5">
            <ArrowDownToLine size={11} />
            Powered by yt-dlp
          </span>
          <span>Baixe apenas conteúdos que você tem permissão para usar.</span>
        </footer>
      </div>
    </main>
  );
}
