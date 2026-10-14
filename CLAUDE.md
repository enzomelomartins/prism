# Prism

Downloader de vídeo/áudio para desktop, construído sobre o
[yt-dlp](https://github.com/yt-dlp/yt-dlp). Repositório:
github.com/enzomelomartins/prism.

Interface React dentro de uma janela nativa Tauri; quem roda o yt-dlp e o
ffmpeg é um backend em Rust. O usuário final não instala Python nem Node —
só abre o app, que baixa as ferramentas sozinho na primeira vez.

**Status: alpha, projeto pessoal.** Hoje é "baixador de vídeo do YouTube",
mas a intenção é crescer bem além disso. Não feche portas com soluções
hiperespecíficas de YouTube quando uma solução genérica (via yt-dlp, que
suporta centenas de sites) custar o mesmo.

## Como funciona

```
WebView2 (React/Tailwind, src/)  <-- invoke/listen -->  Rust (src-tauri/src/)
                                                              |
                                                              v
                                                    yt-dlp.exe + ffmpeg.exe
```

- A interface nunca toca em arquivo nem processo direto: só chama comandos
  Rust (`invoke`) e escuta eventos de progresso (`listen`).
- Um download por vez. Estado mora em `job.rs`; a janela pede um snapshot
  ao abrir.
- Progresso vem de um `--progress-template` próprio do yt-dlp.
- Pausa usa `NtSuspendProcess` (API do Windows) — congela o processo sem
  matar a conexão.
- Preferências: `%APPDATA%\app.prism.desktop\config.json`.
- Ferramentas baixadas: `%APPDATA%\app.prism.desktop\bin\`.

### Arquivos que importam

```
src/App.tsx             tela inteira (React)
src/lib/backend.ts       única porta de entrada pro lado Rust
src/lib/formats.ts       rótulos, filtros, prévia de nome (puro, testável)
src-tauri/src/main.rs    comandos Tauri e estado compartilhado
src-tauri/src/job.rs     download em curso e seus eventos
src-tauri/src/ytdlp.rs   consulta, seleção de formato, args, progresso
src-tauri/src/tools.rs   localizar/baixar yt-dlp e ffmpeg
src-tauri/src/config.rs  preferências persistidas
```

Regra do projeto: lógica fica em módulos puros (`ytdlp.rs`, `formats.ts` não
tocam rede/disco/tela); `main.rs` e `App.tsx` só ligam as pontas.

## Tecnologias

- **Frontend:** TypeScript, React 19, Tailwind CSS 4, Vite 8, Vitest.
- **Backend nativo:** Rust, framework Tauri 2 (IPC `invoke`/`listen`,
  capabilities, bundler).
- **Ferramentas externas baixadas em runtime:** yt-dlp, FFmpeg (não ficam no
  repo nem no bundle).
- **CI/CD:** GitHub Actions — `.github/workflows/release.yml` builda e
  publica o instalador Windows quando uma tag `v*` é enviada
  (`tauri-apps/tauri-action`).
- Plataforma alvo única por ora: Windows 10/11 (WebView2).

## Usuário é iniciante — modo de explicação

Quem pede as mudanças está aprendendo. Em toda alteração, explicar em
português simples, sem economizar nessa parte (ela não entra na régua de
"poucos tokens" abaixo — isso é só pra evitar leitura/exploração
desnecessária, não pra cortar explicação pro usuário):

- **O que mudou e por quê**, em 1-2 frases, antes de mostrar código.
- **Efeito visual**: se a mudança mexe na interface, descrever o que vai
  aparecer diferente na tela (e usar a skill `run` pra efetivamente abrir o
  app e mostrar, quando fizer sentido).
- **Git**: ao commitar, explicar o que o commit representa, não só rodar o
  comando.
- **Documentação**: se o código mudou mas o README ou este CLAUDE.md ficou
  desatualizado, avisar — e atualizar quando a mudança for estrutural.

## Conventional Commits

Todo commit segue:

```
tipo(escopo): descrição curta
```

Exemplo real do projeto: `feat(notifications): download progress with taste
notifications`.

Tipos usados: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `build`,
`ci`, `perf`, `style`. Escopo é a área tocada (`notifications`, `ytdlp`,
`job`, `ui`, `config`...). Descrição curta, em inglês, minúscula, sem ponto
final.

### Releases

Versão é publicada por tag, não por push na branch:

```
git tag v1.0.0 && git push origin v1.0.0
```

Isso dispara `release.yml`, que builda o instalador e cria um release
*draft* no GitHub. Não existe passo manual de build de release — o CI faz
isso.

## Economizar tokens nas tarefas

A estrutura do projeto já está documentada acima — **não re-explorar o
repo inteiro a cada tarefa** (nada de `ls -R`, `find .`, ou abrir todos os
arquivos de `src/`/`src-tauri/src/` "pra entender o contexto"). Ler só os
arquivos que a tarefa realmente toca.

- Antes de rodar um comando, perguntar: essa tarefa precisa disso? Comando
  caro/lento sem necessidade real é desperdício.
- `npm run app:build` compila o instalador Tauri inteiro (lento, caro). Só
  rodar se o usuário pedir instalador/build final. Para checar tipo e
  build da interface, `npm run build` (tsc + vite) já basta.
- `npm run app` abre a janela do app (GUI) — só quando o pedido for
  efetivamente rodar/ver o app funcionando.
- Para lógica Rust, `cargo test --manifest-path src-tauri/Cargo.toml` roda
  contra um substituto do yt-dlp (sem rede) — preferir isso a abrir o app
  pra testar manualmente.
- Preferir Grep/Glob a comandos de busca amplos no shell.
- Não reler um arquivo já lido na mesma sessão se ele não mudou.
