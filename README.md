# Prism

Aplicativo de desktop para baixar vídeos e áudio com o
[yt-dlp](https://github.com/yt-dlp/yt-dlp). Interface em React dentro de uma
janela nativa (Tauri), com o controle do yt-dlp escrito em Rust.

**Não é preciso instalar Python, Node.js nem nada além do próprio aplicativo.**
Na primeira execução o Prism baixa o `yt-dlp` e o `FFmpeg` para a própria pasta
de dados, com um clique.

## O que ele faz

- **Quatro modos de download:** vídeo com áudio, somente vídeo, somente áudio e
  vídeo/áudio em arquivos separados.
- **Escolha real da qualidade:** lista os formatos que aquele link oferece de
  verdade, com resolução, FPS, tamanho e codec, e deixa filtrar por resolução e
  por FPS.
- **Conversão:** MP4 ou MKV para vídeo; MP3, M4A, WAV, FLAC ou OPUS para áudio.
- **Progresso ao vivo**, com velocidade, tamanho e tempo restante — e botões de
  pausar, continuar e cancelar que funcionam de verdade (o processo é suspenso,
  então o download continua de onde parou).
- **Nome do arquivo configurável**, com prévia antes de baixar.
- Metadados, miniatura como capa e legendas incorporadas, se você quiser.
- Log completo do processo, para copiar quando algo der errado.

## Instalação

Baixe o instalador mais recente em
[Releases](https://github.com/enzomelomartins/prism/releases), execute e abra o
Prism. Na primeira vez, clique em **Instalar ferramentas**.

Requisitos: Windows 10 ou 11. O WebView2, usado para desenhar a interface, já
vem com o Windows 11 e com o Edge no Windows 10.

> O `yt-dlp` precisa de atualizações frequentes para continuar funcionando com o
> YouTube. Por isso ele é baixado em vez de embutido, e há um botão
> **Atualizar o yt-dlp** em *Configurações avançadas*.

## Como funciona

```
┌────────────────────────── janela nativa (Tauri) ─────────────────────────────┐
│                                                                              │
│   WebView2 ──── React + Tailwind        │      Rust                          │
│   src/                                  │      src-tauri/src/                │
│                                         │                                    │
│   App.tsx         a interface           │  main.rs    comandos e estado      │
│   lib/backend.ts  invoke + listen  ◄────┼─► job.rs    o download em curso    │
│   lib/formats.ts  rótulos e filtros     │  ytdlp.rs   a linha de comando     │
│                                         │  tools.rs   acha/baixa as          │
│                                         │             ferramentas           │
└─────────────────────────────────────────┴────────────────────────────────────┘
                                                      │
                                                      ▼
                                            yt-dlp.exe  +  ffmpeg.exe
```

A interface nunca executa um programa nem toca em arquivos: ela chama comandos
Rust (`invoke`) e recebe o progresso por eventos (`listen`). Todo o resto —
consultar o link, montar os argumentos do yt-dlp, ler o progresso, pausar,
cancelar, gravar as preferências — acontece em Rust.

Pontos que valem saber:

- **Um download por vez**, que é o que a interface oferece. O estado dele fica
  em `job.rs`; a janela pede um `snapshot` ao abrir, então reabrir a janela
  durante um download mostra o progresso de onde está.
- **Progresso** vem do `--progress-template` do yt-dlp, com um marcador próprio
  que separa as linhas estruturadas do resto do log.
- **Pausar** usa `NtSuspendProcess`: congela o yt-dlp sem matá-lo, mantendo a
  conexão aberta. Fica indisponível durante a mesclagem, que é feita pelo
  ffmpeg filho.
- **Preferências** (pasta, formato, padrão de nome…) vão para
  `%APPDATA%\app.prism.desktop\config.json`.
- **Ferramentas baixadas** vão para `%APPDATA%\app.prism.desktop\bin\`. Uma
  cópia colocada à mão ao lado do `.exe` do Prism, ou no PATH, também é
  encontrada.

## Desenvolvimento

Pré-requisitos:

- [Node.js](https://nodejs.org/) 20 ou mais novo
- [Rust](https://rustup.rs/) (toolchain `stable-x86_64-pc-windows-msvc`)
- Visual Studio Build Tools com **Desktop development with C++** (o compilador
  e o SDK do Windows que o Rust usa para linkar)

```bash
npm install
npm run app          # abre a janela com recarregamento automático
npm run app:build    # gera o instalador em src-tauri/target/release/bundle/
```

Verificações:

```bash
npm run build                  # type-check (tsc) + build da interface
npm test                       # testes da camada de apresentação
cargo test --manifest-path src-tauri/Cargo.toml   # testes da lógica do yt-dlp
```

Os testes de `job.rs` rodam o pipeline inteiro — iniciar, ler a saída, pausar,
retomar, cancelar — contra um substituto do yt-dlp, sem tocar na rede.

### Estrutura

```
prism/
├── index.html              casca da página carregada pela WebView
├── src/                    interface
│   ├── App.tsx             a tela inteira
│   ├── styles.css          tema e layout
│   ├── components/ui/      o botão da interface
│   └── lib/
│       ├── backend.ts      a única porta de entrada para o lado Rust
│       └── formats.ts      rótulos, filtros e prévia do nome (puro, testável)
└── src-tauri/              aplicativo nativo
    ├── tauri.conf.json     janela, CSP, empacotamento
    ├── capabilities/       o que a janela pode chamar
    └── src/
        ├── main.rs         comandos e estado compartilhado
        ├── job.rs          o download em curso e seus eventos
        ├── ytdlp.rs        consulta, seleção de formato, argumentos, progresso
        ├── tools.rs        localizar e baixar yt-dlp e FFmpeg
        └── config.rs       preferências persistidas
```

A regra que mantém o projeto navegável: **toda a lógica fica em módulos puros**
(`ytdlp.rs` e `formats.ts` não falam com a rede, com o disco nem com a tela), e
`main.rs` e `App.tsx` só ligam as pontas.

## Licença

[MIT](LICENSE). O yt-dlp e o FFmpeg são projetos independentes, baixados em
tempo de execução e cobertos pelas suas próprias licenças (Unlicense e
GPL/LGPL). Baixe apenas conteúdos que você tem permissão para usar.
