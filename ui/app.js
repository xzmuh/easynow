// Interface do easynow. Cada aba tem um xterm.js; o Rust (src/) roda os programas
// e manda a saída pelo evento "saida". A cada meio segundo pedimos o "status"
// (atividade, CPU, RAM, pasta, branch) e redesenhamos os painéis.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (s) => document.querySelector(s);

const TEMA = {
  background: "#131315",
  foreground: "#e4e4e7",
  cursor: "#4ade80",
  cursorAccent: "#131315",
  selectionBackground: "rgba(74, 222, 128, 0.25)",
  black: "#1f1f23", brightBlack: "#52525b",
  red: "#f87171", brightRed: "#fca5a5",
  green: "#4ade80", brightGreen: "#86efac",
  yellow: "#facc15", brightYellow: "#fde68a",
  blue: "#60a5fa", brightBlue: "#93c5fd",
  magenta: "#c084fc", brightMagenta: "#d8b4fe",
  cyan: "#5eead4", brightCyan: "#99f6e4",
  white: "#d4d4d8", brightWhite: "#fafafa",
};
const NOME = { claude: "Claude", codex: "Codex", shell: "Shell" };

const abas = [];
let ativa = -1;
let grade = false;
let proxId = 1;
let home = "";
let sistema = null;
let confirmar = null; // função a rodar se o usuário disser "sim" no modal

// ---------- abas ----------

async function novaAba(tipo, pastaDe = null) {
  const id = proxId++;
  const el = document.createElement("div");
  el.className = "quadro";
  el.innerHTML = `<div class="quadro-topo"><span class="ic"></span><span class="t"></span></div><div class="xterm-host"></div>`;
  $("#terminais").append(el);

  const term = new Terminal({
    fontFamily: '"JetBrainsMono Nerd Font", "JetBrains Mono", monospace',
    fontSize: 13,
    lineHeight: 1.2,
    theme: TEMA,
    cursorBlink: true,
    scrollback: 10000,
    allowProposedApi: true,
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);

  const aba = {
    id, tipo, term, fit, el,
    titulo: "", pasta: "", aberta: Date.now(),
    trabalhandoDesde: null, tempoTrabalhando: 0, rodadas: 0,
    aviso: false, codigo: null, st: null, tamanho: [0, 0],
  };
  abas.push(aba);
  ativar(abas.length - 1);

  term.open(el.querySelector(".xterm-host"));
  try { fit.fit(); } catch {}

  term.onData((d) => invoke("escrever", { id, dados: d }));
  term.onTitleChange((t) => { aba.titulo = t; desenhar(); });
  term.onBell(() => { if (abas[ativa] !== aba) { aba.aviso = true; desenhar(); } });
  term.attachCustomKeyEventHandler((ev) => !atalho(ev));
  el.querySelector(".quadro-topo").addEventListener("click", () => ativar(abas.indexOf(aba)));
  el.addEventListener("mousedown", () => { if (abas[ativa] !== aba) ativar(abas.indexOf(aba)); });
  new ResizeObserver(() => ajustar(aba)).observe(el);

  try {
    aba.pasta = await invoke("abrir", { id, tipo, pastaDe, linhas: term.rows, colunas: term.cols });
    aba.tamanho = [term.rows, term.cols];
    evento(aba, `aberto em ${curto(aba.pasta)}`);
  } catch (e) {
    aba.codigo = -1;
    evento(aba, `não abriu: ${e}`, "erro");
  }
  desenhar();
}

function ajustar(aba) {
  if (!aba.el.offsetParent) return; // escondida
  try { aba.fit.fit(); } catch { return; }
  const { rows, cols } = aba.term;
  if (rows !== aba.tamanho[0] || cols !== aba.tamanho[1]) {
    aba.tamanho = [rows, cols];
    invoke("redimensionar", { id: aba.id, linhas: rows, colunas: cols });
  }
}

function ativar(i) {
  if (i < 0 || i >= abas.length) return;
  ativa = i;
  abas[i].aviso = false;
  abas.forEach((a, j) => a.el.classList.toggle("ativo", j === i));
  desenhar();
  requestAnimationFrame(() => {
    abas.forEach(ajustar);
    abas[i].term.focus();
  });
}

function pedirFechar(i) {
  const aba = abas[i];
  if (!aba) return;
  if (aba.codigo !== null) return fechar(i);
  abrirModal(
    `Fechar a aba ${i + 1}?`,
    `O ${NOME[aba.tipo]} (${nomeCurto(aba)}) será encerrado.`,
    () => fechar(i),
  );
}

function fechar(i) {
  const aba = abas[i];
  invoke("fechar", { id: aba.id });
  aba.term.dispose();
  aba.el.remove();
  abas.splice(i, 1);
  evento(aba, "aba fechada");
  if (abas.length === 0) { ativa = -1; desenhar(); return; }
  ativar(Math.min(i, abas.length - 1));
}

function alternarGrade() {
  grade = !grade;
  desenhar();
  requestAnimationFrame(() => abas.forEach(ajustar));
}

// ---------- estado de cada agente ----------

function atualizarEstados(status) {
  sistema = status;
  const agora = Date.now();
  for (const aba of abas) {
    const s = status.abas.find((x) => x.id === aba.id);
    if (!s) continue;
    aba.st = s;
    if (s.codigo_saida != null && aba.codigo === null) {
      aba.codigo = s.codigo_saida;
      fecharRodada(aba);
      evento(aba, `encerrou (código ${aba.codigo})`, aba.codigo !== 0 ? "erro" : "");
      continue;
    }
    // "Trabalhando" = saiu coisa na tela há pouco, e não foi só o eco do que você digitou.
    const ocupada = aba.codigo === null && s.ms_saida < 1500 && s.ms_entrada - s.ms_saida > 400;
    if (ocupada && !aba.trabalhandoDesde) aba.trabalhandoDesde = agora;
    else if (!ocupada && aba.trabalhandoDesde) fecharRodada(aba);
  }
}

function fecharRodada(aba) {
  const inicio = aba.trabalhandoDesde;
  if (!inicio) return;
  const durou = Date.now() - inicio;
  aba.tempoTrabalhando += durou;
  aba.trabalhandoDesde = null;
  // ignora piscadas curtas e a abertura do programa
  if (durou < 3000 || inicio - aba.aberta < 5000) return;
  aba.rodadas++;
  if (abas[ativa] !== aba) aba.aviso = true;
  evento(aba, `terminou depois de ${duracao(durou)}`, "bom");
}

function estadoDe(aba) {
  if (aba.codigo !== null) return "encerrado";
  if (aba.trabalhandoDesde) return "trabalhando";
  if (aba.aviso) return "pronto";
  return "parado";
}

const TAG = { trabalhando: "Trabalhando", pronto: "Pronto", parado: "Parado", encerrado: "Encerrado" };

// Título que vale mostrar: ignora os genéricos (usuario@maquina:pasta, nome da pasta, "Claude Code").
function tituloUtil(aba) {
  // tira os ícones de spinner que o claude/codex põem no começo do título
  const t = aba.titulo.replace(/^[^\p{L}\p{N}~/]+/u, "").trim();
  const pasta = (aba.st?.pasta || aba.pasta || "").split("/").pop();
  const generico = /^[^\s@]+@[^\s:]+(:|$)/.test(t) || t === pasta || /^(claude code|codex|claude)$/i.test(t);
  return generico ? "" : t;
}

// Nome curto (abas, lista de sessões)
function nomeCurto(aba) {
  const t = tituloUtil(aba);
  if (t) return t;
  return aba.tipo === "shell" ? `Shell · ${curto(aba.st?.pasta || aba.pasta)}` : NOME[aba.tipo];
}

// Nome com o agente na frente (cabeçalho do terminal)
function nomeCompleto(aba) {
  const t = tituloUtil(aba);
  return t ? `${NOME[aba.tipo]} · ${t}` : nomeCurto(aba);
}

// ---------- desenho ----------

function desenhar() {
  const a = abas[ativa];
  const X = '<svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18"/></svg>';

  // abas
  $("#abas").innerHTML = abas.map((x, i) => `
    <button class="aba ${i === ativa ? "ativa" : ""}" data-i="${i}" title="${esc(nomeCurto(x))}">
      <span class="ic ${estadoDe(x)}"></span>
      <span class="nome">${esc(nomeCurto(x))}</span>
      <span class="fechar" data-fechar="${i}">${X}</span>
    </button>`).join("");

  // quadros (títulos na grade)
  abas.forEach((x) => {
    x.el.querySelector(".quadro-topo .ic").className = `ic ${estadoDe(x)}`;
    x.el.querySelector(".quadro-topo .t").textContent = nomeCompleto(x);
  });
  const t = $("#terminais");
  t.classList.toggle("grade", grade && abas.length > 1);
  t.style.setProperty("--colunas", abas.length <= 4 ? Math.min(2, abas.length) : 3);
  $("#btn-grade").classList.toggle("ligado", grade);
  $("#vazio").hidden = abas.length > 0;
  t.hidden = abas.length === 0;
  $("#barra-agente").hidden = abas.length === 0;

  // estado geral
  const trabalhando = abas.filter((x) => estadoDe(x) === "trabalhando").length;
  const esperando = abas.filter((x) => estadoDe(x) === "pronto").length;
  const eg = $("#estado-geral");
  eg.className = `estado ${trabalhando ? "trabalhando" : ""}`;
  eg.textContent = trabalhando ? `${trabalhando} trabalhando`
    : esperando ? `${esperando} te esperando` : "Tudo calmo";

  // visão geral
  if (sistema) {
    const cpu = Math.round(sistema.cpu);
    $("#cpu-txt").textContent = `${cpu}%`;
    $("#cpu-barra").style.width = `${cpu}%`;
    const mem = sistema.memoria_total ? (sistema.memoria_usada / sistema.memoria_total) * 100 : 0;
    $("#ram-txt").textContent = `${tamanho(sistema.memoria_usada)} de ${tamanho(sistema.memoria_total)}`;
    $("#ram-barra").style.width = `${mem}%`;
    $("#ram-agentes").textContent = tamanho(abas.reduce((s, x) => s + (x.st?.memoria || 0), 0));
  }
  $("#n-trabalhando").textContent = trabalhando;
  $("#n-trabalhando").classList.toggle("ativo", trabalhando > 0);
  $("#n-esperando").textContent = esperando;

  // sessões
  $("#n-sessoes").textContent = abas.length;
  $("#sessoes").innerHTML = abas.map((x, i) => {
    const e = estadoDe(x);
    const desc = e === "trabalhando" ? `trabalhando há ${duracao(Date.now() - x.trabalhandoDesde)}`
      : e === "pronto" ? "terminou, te esperando"
      : e === "encerrado" ? `encerrou (código ${x.codigo})`
      : curto(x.st?.pasta || x.pasta);
    return `
    <li class="${i === ativa ? "ativa" : ""}" data-i="${i}">
      <span class="ic ${e}"></span>
      <b>${esc(nomeCurto(x))}</b>
      <span class="tag ${e}">${NOME[x.tipo]}</span>
      <small>${esc(desc)}</small>
    </li>`;
  }).join("");

  // cabeçalho do terminal
  if (a) {
    $("#ba-ic").className = `ic ${estadoDe(a)}`;
    $("#ba-titulo").textContent = nomeCompleto(a);
    $("#ba-pasta").textContent = curto(a.st?.pasta || a.pasta);
    const br = $("#ba-branch");
    br.hidden = !a.st?.branch;
    br.textContent = a.st?.branch ? `⎇ ${a.st.branch}` : "";
  }

  // agente ativo
  $("#op-n").textContent = a ? `aba ${ativa + 1}` : "";
  const op = a ? estadoDe(a) : null;
  const opTag = $("#op-tag");
  opTag.className = `tag ${op || ""}`;
  opTag.textContent = op ? TAG[op] : "Nenhum";
  $("#op-titulo").textContent = !a ? "Aguardando instrução"
    : op === "encerrado" ? "Processo encerrado"
    : tituloUtil(a) || "Aguardando instrução";
  $("#op-desc").textContent = !a ? "Nenhum agente aberto."
    : op === "trabalhando" ? `${NOME[a.tipo]} trabalhando há ${duracao(Date.now() - a.trabalhandoDesde)}.`
    : op === "pronto" ? `${NOME[a.tipo]} terminou e está te esperando.`
    : op === "encerrado" ? `Saiu com código ${a.codigo}.`
    : `${NOME[a.tipo]} parado, esperando você.`;
  $("#op-trilho").className = op === "trabalhando" ? "correndo" : "";

  // detalhes
  if (a) {
    const trabalhou = a.tempoTrabalhando + (a.trabalhandoDesde ? Date.now() - a.trabalhandoDesde : 0);
    const aberta = Date.now() - a.aberta;
    const pares = [
      ["Última atividade", a.st ? `há ${duracao(a.st.ms_saida)}` : "—"],
      ["Tempo trabalhando", `${duracao(trabalhou)} · ${aberta ? Math.round((trabalhou / aberta) * 100) : 0}%`],
      ["Aberto há", duracao(aberta)],
      ["Rodando", a.st?.rodando || "—"],
      ["CPU", a.st ? `${Math.round(a.st.cpu)}%` : "—"],
      ["Memória", a.st ? tamanho(a.st.memoria) : "—"],
      ["Processos", a.st?.processos ?? "—"],
    ];
    $("#sinal").innerHTML = pares.map(([k, v]) => `<dt>${k}</dt><dd>${esc(String(v))}</dd>`).join("");
  } else {
    $("#sinal").innerHTML = "";
  }

  $("#relogio").textContent = a ? relogio(Date.now() - a.aberta) : "00:00:00";
  $("#rodadas-total").textContent = abas.reduce((s, x) => s + x.rodadas, 0);

  document.title = a ? `easynow · ${nomeCurto(a)}` : "easynow";
}

// ---------- eventos (log) ----------

function evento(aba, txt, tipo = "") {
  linhaLog(NOME[aba.tipo] + (abas.includes(aba) ? ` ${abas.indexOf(aba) + 1}` : ""), txt, tipo);
}

function linhaLog(quem, txt, tipo = "") {
  const p = document.createElement("p");
  p.className = tipo;
  p.innerHTML = `<span class="h">${hora()}</span><span class="q">${esc(quem)}</span><span class="txt">${esc(txt)}</span>`;
  const log = $("#linhas-log");
  log.append(p);
  while (log.children.length > 200) log.firstChild.remove();
  log.scrollTop = log.scrollHeight;
}

// ---------- atalhos ----------

// Devolve true se a tecla foi um atalho do easynow (aí ela não vai para o programa).
function atalho(ev) {
  if (ev.type !== "keydown") return false;
  const k = ev.key.toLowerCase();

  if (!$("#modal").hidden) {
    if (k === "s" || k === "y" || k === "enter") fecharModal(true);
    else if (k === "n" || k === "escape") fecharModal(false);
    return true;
  }
  if (!$("#menu-novo").hidden) {
    const tipo = { c: "claude", x: "codex", s: "shell" }[k];
    $("#menu-novo").hidden = true;
    if (tipo) novaAba(tipo, abas[ativa]?.id ?? null);
    return true;
  }

  const csh = ev.ctrlKey && ev.shiftKey && !ev.altKey;
  if (ev.altKey && !ev.ctrlKey && /^Digit[1-9]$/.test(ev.code)) {
    ativar(Number(ev.code.slice(5)) - 1);
  } else if (ev.ctrlKey && !ev.shiftKey && ev.key === "PageUp" && abas.length) {
    ativar((ativa - 1 + abas.length) % abas.length);
  } else if (ev.ctrlKey && !ev.shiftKey && ev.key === "PageDown" && abas.length) {
    ativar((ativa + 1) % abas.length);
  } else if (csh && ev.code === "KeyT") {
    $("#menu-novo").hidden = false;
  } else if (csh && ev.code === "KeyW") {
    pedirFechar(ativa);
  } else if (csh && ev.code === "KeyG") {
    alternarGrade();
  } else if (csh && ev.code === "KeyC") {
    const sel = abas[ativa]?.term.getSelection();
    if (sel) navigator.clipboard.writeText(sel);
  } else if (csh && ev.code === "KeyV") {
    navigator.clipboard.readText().then((t) => abas[ativa]?.term.paste(t)).catch(() => {});
  } else {
    return false;
  }
  ev.preventDefault();
  return true;
}

// teclas quando nenhum terminal está com o foco (tela vazia, menu aberto...)
window.addEventListener("keydown", (ev) => {
  if (ev.target.closest?.(".xterm")) return;
  atalho(ev);
});

// ---------- modal ----------

function abrirModal(titulo, texto, aoConfirmar) {
  $("#modal-titulo").textContent = titulo;
  $("#modal-texto").textContent = texto;
  confirmar = aoConfirmar;
  $("#modal").hidden = false;
  $("#modal-sim").focus();
}

function fecharModal(sim) {
  $("#modal").hidden = true;
  const f = confirmar;
  confirmar = null;
  if (sim && f) f();
  else abas[ativa]?.term.focus();
}

// ---------- cliques ----------

$("#abas").addEventListener("click", (ev) => {
  const f = ev.target.closest("[data-fechar]");
  if (f) return pedirFechar(Number(f.dataset.fechar));
  const b = ev.target.closest("[data-i]");
  if (b) ativar(Number(b.dataset.i));
});
$("#abas").addEventListener("auxclick", (ev) => {
  const b = ev.target.closest("[data-i]");
  if (b && ev.button === 1) pedirFechar(Number(b.dataset.i));
});
for (const sel of ["#sessoes"]) {
  $(sel).addEventListener("click", (ev) => {
    const li = ev.target.closest("[data-i]");
    if (li) ativar(Number(li.dataset.i));
  });
}
$("#btn-novo").addEventListener("click", (ev) => {
  ev.stopPropagation();
  $("#menu-novo").hidden = !$("#menu-novo").hidden;
});
document.addEventListener("click", (ev) => {
  const t = ev.target.closest("[data-tipo]");
  if (t) {
    $("#menu-novo").hidden = true;
    novaAba(t.dataset.tipo, abas[ativa]?.id ?? null);
  } else if (!ev.target.closest(".menu")) {
    $("#menu-novo").hidden = true;
  }
});
$("#btn-grade").addEventListener("click", alternarGrade);
$("#modal-sim").addEventListener("click", () => fecharModal(true));
$("#modal-nao").addEventListener("click", () => fecharModal(false));

// ---------- formatação ----------

function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
}
function curto(p) {
  if (!p) return "—";
  return home && p.startsWith(home) ? "~" + p.slice(home.length) : p;
}
function duracao(ms) {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(s / 3600)}h${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m`;
}
function relogio(ms) {
  const s = Math.floor(ms / 1000);
  return [s / 3600, (s % 3600) / 60, s % 60].map((n) => String(Math.floor(n)).padStart(2, "0")).join(":");
}
function hora() {
  return new Date().toLocaleTimeString("pt-BR", { hour12: false });
}
function tamanho(b) {
  if (b >= 1e9) return `${(b / 1e9).toFixed(1)}G`;
  if (b >= 1e6) return `${Math.round(b / 1e6)}M`;
  if (b >= 1e3) return `${Math.round(b / 1e3)}K`;
  return `${b}B`;
}
function bytes(b64) {
  const bin = atob(b64);
  const u = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) u[i] = bin.charCodeAt(i);
  return u;
}

// ---------- início ----------

(async () => {
  await listen("saida", ({ payload }) => {
    abas.find((a) => a.id === payload.id)?.term.write(bytes(payload.dados));
  });
  await listen("fim", () => {}); // o código de saída chega pelo status

  const ini = await invoke("inicio");
  home = ini.home;
  linhaLog("easynow", `iniciado em ${curto(ini.pasta)}`);

  for (const tipo of ini.abas) await novaAba(tipo);
  ativar(0);

  setInterval(async () => {
    try { atualizarEstados(await invoke("status")); } catch {}
    desenhar();
  }, 500);
})();
