# Alethe Native

Versão em **Rust puro** do [Alethe](https://github.com/Kc1t/alethe-agents): sem Tauri, sem WebView2 e sem
JavaScript. A interface é desenhada com [egui](https://github.com/emilk/egui) e segue a versão web o mais de perto
possível (titlebar com abas de projeto, sidebar com grupos/projetos/terminais, Home com composer, workspace com
panes, painel de tarefas, temas, orquestrador, controle de versão e pane de Markdown). Os terminais usam o mesmo
motor do Alacritty.

O objetivo principal é **consumir menos memória** que a versão Tauri: tudo roda em um único processo nativo,
sem o navegador embutido (WebView2 abre vários processos de browser, GPU, renderer e rede).

## Rodando

```sh
cargo run --release
```

Requer Rust 1.95+. No Windows os terminais usam ConPTY (Windows 10 1809+).

| Variável | Efeito |
|---|---|
| `ALETHE_NATIVE_DATA_DIR` | Usa outra pasta de dados (útil para testar sem tocar nos seus projetos). |
| `ALETHE_NATIVE_SMOKE` | Teste rápido: `agente\|pasta\|prompt` abre uma sessão ao iniciar; `@settings` abre as Preferências; `@git` abre a aba de controle de versão. |

## Dados

Mesmo formato do Alethe Tauri (`profiles.json` + `profiles/<perfil>/projects.json`), em uma pasta própria
(`<LocalAppData>/com.kc1t.alethe.native`). Na primeira execução os projetos do Alethe Tauri são **copiados**
(o original só é lido, nunca alterado). Campos que esta versão não conhece são preservados ao salvar.
Preferências → "Reimportar do Alethe" copia de novo.

## Arquitetura

| Módulo | O que faz |
|---|---|
| `backend/` | `child_env`, `cli_resolver`, `orchestrator_core` (+ `rules`, `shells`) e `git_control` copiados do backend Tauri (só os wrappers `#[tauri::command]` foram removidos); `process` porta o kill da árvore de processos e o Job Object que mata os terminais junto com o app. |
| `orchestrator.rs` | Servidor MCP local (HTTP em 127.0.0.1, com token) que liga os planners Claude/Codex ao core do orquestrador. Só sobe quando o recurso está ligado e um quadro ou planner é aberto. |
| `launch.rs`, `agents.rs` | Port de `buildAgentLaunch`: `--session-id`/`--resume` do Claude, `resume` do Codex, flags de modo YOLO etc. |
| `terminal/` | `alacritty_terminal` + `portable-pty`; renderização, seleção, scrollback, mouse reporting, bracketed paste. |
| `theme/` | Os 17 temas de interface e as paletas de terminal, gerados de `theme.css`/`xtermThemes.ts` por `tools/gen_themes.py`. |
| `ui/` | Titlebar, sidebar, Home, workspace, tarefas, modais, quadro de orquestração (`orchestrator.rs`), controle de versão (`git_panel.rs`) e pane de Markdown (`markdown.rs`). |
| `storage.rs`, `model.rs` | Leitura/escrita atômica de `projects.json`, importação do Tauri. |

### Escolhas para economizar memória

- Um processo só; renderer OpenGL (`glow`) em vez de `wgpu`.
- egui só redesenha quando há entrada ou saída de terminal; parado, não gasta CPU.
- Scrollback igual ao nível econômico da versão web (6 000 linhas para agentes, 3 000 para shells); o grid do
  Alacritty cresce sob demanda, então histórico não usado não ocupa memória.
- Ícones de agentes reduzidos a 64 px; fonte de terminal embutida, fonte de interface lida do sistema.
- Ao fechar um terminal, a árvore inteira (shell → agente → servidores MCP) é encerrada.

Medido no Windows 11 (build release, só o processo do app, sem contar shells/agentes):

| Situação | Working set | Memória privada |
|---|---|---|
| Home, nenhum terminal | ~87 MB | ~34 MB |
| 2 terminais (pwsh + Claude Code) | ~96 MB | ~39 MB |

## O que ainda não foi portado

Voz, plugins, estatísticas de uso, panes de navegador/arquivo/imagem/vídeo/diff, sincronização na nuvem e acesso
remoto. Panes desses tipos aparecem com um aviso, e os dados deles são mantidos.

No orquestrador ainda faltam os shells abertos pelo planner e as arestas animadas; no Markdown, diagramas Mermaid
(aparecem como código) e links clicáveis.

## Licença

AGPL-3.0-or-later, como o projeto original. Veja `LICENSE` e `NOTICE`.
