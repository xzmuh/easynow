# easynow

Um terminal só para os agentes. Em vez de 4 janelas abertas (claude, codex, shell...),
cada uma vira uma aba aqui dentro. O meio mostra o terminal da aba ativa e as laterais
mostram o que importa: quem está trabalhando, quem terminou e está te esperando, pasta,
branch, CPU/RAM de cada agente.

```
 EASYNOW  1 ⠹ arrumando login   2 ✓ testes   3   deploy   4   shell
┌ SESSÕES ──────────┐┌ 1 arrumando login ────────────────┐┌ AGENTE ATIVO ─────┐
│▌1 claude trabalh… ││                                   ││ pasta, branch,    │
│ 2 codex  pronto ✓ ││     terminal da aba ativa         ││ tempo trabalhando,│
│ 3 claude parado   ││                                   ││ CPU, RAM...       │
├ SISTEMA ──────────┤│                                   │├ ATALHOS ──────────┤
├ EVENTOS ──────────┤│                                   ││                   │
└───────────────────┘└───────────────────────────────────┘└───────────────────┘
```

## Usar

Abra o terminal na pasta do projeto e digite:

```sh
easynow
```

Abre claude, codex e shell. Para escolher as abas: `easynow claude claude codex shell`.
Todas abrem na pasta onde você rodou o comando.

## Instalar

```sh
cargo build --release
ln -s "$PWD/target/release/easynow" ~/.local/bin/easynow   # ~/.local/bin precisa estar no PATH
```

Depois de mudar o código, é só rodar `cargo build --release` de novo.

## Atalhos

| Tecla | O quê |
|---|---|
| `Alt+1..9` | ir para a aba |
| `Ctrl+PgUp` / `Ctrl+PgDn` | aba anterior / próxima |
| `Alt+t` | nova aba (depois `c` claude, `x` codex, `s` shell), na mesma pasta da aba atual |
| `Alt+w` | fechar aba |
| `Alt+r` | reiniciar uma aba que encerrou |
| `Alt+g` | ver todas em grade / só a ativa |
| `Alt+b` | esconder os painéis laterais |
| `Shift+PgUp` / `Shift+PgDn` ou rodinha do mouse | histórico |
| `Shift+arrastar` | selecionar texto |
| `Alt+q` | sair |

Todo o resto vai direto para o programa da aba.

## Como funciona

- `src/aba.rs`: cada aba abre o programa numa PTY (um terminal virtual). Uma thread lê o que ele
  escreve e passa para o `vt100`, que monta a tela. É dali que vem o título que o claude/codex colocam.
  "Trabalhando" = saiu coisa na tela há pouco e não foi só o eco do que você digitou.
- `src/teclas.rs`: transforma a tecla apertada nos bytes que um terminal mandaria.
- `src/telemetria.rs`: uma thread que mede CPU/RAM de cada aba (somando os processos filhos), pasta e branch.
- `src/tela.rs`: desenha tudo com `ratatui`.
- `src/main.rs`: loop principal e atalhos.

## Próximo

- Voz: segurar uma tecla, falar, soltar, e o texto ser digitado na aba ativa (Whisper local).
