# easynow

Uma janela só para os agentes. Em vez de 4 terminais abertos (claude, codex, shell...),
cada um vira uma aba aqui dentro, com telemetria em volta: quem está trabalhando, quem
terminou e está te esperando, pasta, branch, CPU e memória de cada agente, e um log de eventos.

## Usar

Abra o terminal na pasta do projeto e digite:

```sh
easynow
```

Abre um Claude. As outras abas você abre pelo botão "Nova aba", ou já pede na hora: `easynow claude claude codex shell`.
A janela abre solta do terminal (pode fechar o terminal depois).

## Várias contas do Claude

O easynow acha sozinho cada conta do Claude: a padrão (`~/.claude`) e qualquer pasta `~/.claude-<nome>` com login feito.
Para adicionar uma conta, crie a pasta fazendo login nela uma vez:

```sh
CLAUDE_CONFIG_DIR=~/.claude-trabalho claude   # depois digite /login e entre com a outra conta
```

Na próxima vez que abrir o easynow, a conta aparece em "Nova aba", com o nome da organização (ou o começo do e-mail).

## Som ao terminar

Quando um Claude termina uma tarefa, toca um aviso curto e grave. Ao abrir, o easynow coloca esse aviso
(um hook `Stop`) no `settings.json` de cada conta encontrada; ele vale até para o Claude aberto fora do easynow.
O botão de alto-falante no topo da janela muta e desmuta. O som e a marca de mudo ficam em `~/.config/easynow/`.

Precisa de `paplay`, `pw-play` ou `aplay` (qualquer Linux com PulseAudio, PipeWire ou ALSA já tem um deles).
Se apagar o hook, o easynow coloca de novo ao abrir; para não ouvir, use o botão de mudo.

## Atalhos

| Tecla | O quê |
|---|---|
| `Alt+1..9` | ir para a aba |
| `Ctrl+PgUp` / `Ctrl+PgDn` | aba anterior / próxima |
| `Ctrl+Shift+T` | nova aba (`C` claude, `X` codex, `S` shell), na pasta da aba atual |
| `Ctrl+Shift+W` | fechar aba (ou o ✕ / botão do meio na aba) |
| `Ctrl+Shift+G` | ver todas em grade |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | copiar / colar |

## Instalar / atualizar

```sh
cargo build --release
ln -sf "$PWD/target/release/easynow" ~/.local/bin/easynow   # ~/.local/bin precisa estar no PATH
```

Depois de mexer no código (Rust ou `ui/`), rode `cargo build --release` de novo: a interface vai embutida no binário.

## Como funciona

- **Janela:** [Tauri](https://tauri.app). A interface é HTML/CSS/JS em `ui/`, os terminais são [xterm.js](https://xtermjs.org).
- `src/pty.rs`: cada aba abre o programa numa PTY (terminal virtual). Uma thread lê a saída e manda para a janela.
- `src/monitor.rs`: sabe se cada aba está trabalhando **sem ficar perguntando toda hora**. Tudo dorme até algo acontecer:
  - Claude: grava `busy`/`idle` em `<config>/sessions/<pid>.json`; o Linux avisa quando o arquivo muda (inotify).
  - Codex: grava `task_started`/`task_complete` e os tokens no arquivo da sessão em `~/.codex/sessions`; mesmo esquema.
  - Shell: a saída do terminal acorda a thread da aba; 1,5 s de silêncio e ela volta a dormir.
  - Limites de uso: buscados só ao trocar de aba ou quando um agente termina (no máximo 1x por minuto).
- `src/som.rs`: instala o hook do som em cada conta e guarda o mudo.
- `src/uso.rs`: contas do Claude (`~/.claude`, `~/.claude-*`), tokens de cada sessão e limites.
- `ui/app.js`: abas, atalhos e painéis. A tela só é redesenhada quando chega um evento.

Parado, com 4 abas abertas, o easynow usa ~0,2% de CPU e ~260 MB de memória (a maior parte é o WebKit da janela).

## Próximo

- Voz: segurar uma tecla, falar, soltar, e o texto ser digitado na aba ativa (Whisper local).
