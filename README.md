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
- `src/pty.rs`: cada aba abre o programa numa PTY (terminal virtual). Uma thread lê a saída e manda para a janela (evento `saida`).
- `src/telemetria.rs`: mede CPU/RAM de cada aba (somando os processos filhos), pasta atual e branch do git.
- `src/main.rs`: os comandos que a janela chama (`abrir`, `escrever`, `redimensionar`, `fechar`, `status`).
- `ui/app.js`: abas, estados ("trabalhando" = saiu coisa na tela há pouco e não foi o eco do que você digitou), atalhos e painéis.
- `ui/estilo.css`: o visual (cinza, azul-escuro e azul-claro neon).

## Próximo

- Voz: segurar uma tecla, falar, soltar, e o texto ser digitado na aba ativa (Whisper local).
