// Interface do easynow. Cada aba tem um xterm.js; o Rust (src/) roda os programas
// e avisa por eventos: "saida" (texto do terminal), "estado" (trabalhando ou não),
// "info" (pasta, workspace, branch, tokens), "limites" e "fim". Nada roda em
// intervalo: a tela só é redesenhada quando chega um evento ou você faz algo.

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
let limites = [];
let contas = []; // contas do Claude (Next SI, Nuveto...)
let confirmar = null; // função a rodar se o usuário disser "sim" no modal

// ---------- abas ----------

async function novaAba(tipo, pastaDe = null, conta = null) {
  if (tipo === "claude" && !conta) conta = contas[0] || null;
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
    cursorBlink: false,
    scrollback: 10000,
    allowProposedApi: true,
    // links que o programa marca (OSC 8), como os do Claude
    linkHandler: { activate: (_ev, uri) => abrirLink(uri) },
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  // endereços soltos no texto (https://...)
  term.loadAddon(new WebLinksAddon.WebLinksAddon((_ev, uri) => abrirLink(uri)));

  const aba = {
    id, tipo, conta, term, fit, el,
    titulo: "", pasta: "", aberta: Date.now(),
    trabalhandoDesde: null, tempoTrabalhando: 0, rodadas: 0,
    aviso: false, codigo: null, info: null, tamanho: [0, 0],
  };
  abas.push(aba);
  ativar(abas.length - 1);

  term.open(el.querySelector(".xterm-host"));
  try { fit.fit(); } catch {}

  term.onData((d) => invoke("escrever", { id, dados: d }));
  term.onTitleChange((t) => {
    // o Claude anima um ícone no título enquanto trabalha; só redesenha se o texto mudou
    const antes = tituloUtil(aba);
    aba.titulo = t;
    if (tituloUtil(aba) !== antes) desenhar();
  });
  term.onBell(() => { if (abas[ativa] !== aba) { aba.aviso = true; desenhar(); } });
  term.attachCustomKeyEventHandler((ev) => !atalho(ev));
  el.querySelector(".quadro-topo").addEventListener("click", () => ativar(abas.indexOf(aba)));
  el.addEventListener("mousedown", () => { if (abas[ativa] !== aba) ativar(abas.indexOf(aba)); });
  new ResizeObserver(() => ajustar(aba)).observe(el);

  try {
    aba.pasta = await invoke("abrir", { id, tipo, conta: conta?.dir ?? null, pastaDe, linhas: term.rows, colunas: term.cols });
    aba.tamanho = [term.rows, term.cols];
  } catch (e) {
    aba.codigo = -1;
    term.write(`\r\n  Não consegui abrir o ${rotulo(aba)}: ${e}\r\n`);
  }
  desenhar();
}

function abrirLink(uri) {
  invoke("abrir_link", { url: uri });
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
  const a = abas[i];
  if (a.tipo === "claude" && a.conta) invoke("pedir_limites", { chave: a.conta.dir });
  conferirVoz(a);
  terminarOrb();
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
    `O ${rotulo(aba)} (${nomeCurto(aba)}) será encerrado.`,
    () => fechar(i),
  );
}

function fechar(i) {
  const aba = abas[i];
  invoke("fechar", { id: aba.id });
  aba.term.dispose();
  aba.el.remove();
  abas.splice(i, 1);
  if (abas.length === 0) { ativa = -1; desenhar(); return; }
  ativar(Math.min(i, abas.length - 1));
}

function alternarGrade() {
  grade = !grade;
  desenhar();
  requestAnimationFrame(() => abas.forEach(ajustar));
}

// ---------- estado de cada agente ----------

const porId = (id) => abas.find((a) => a.id === id);

function aoEstado({ id, trabalhando }) {
  const aba = porId(id);
  if (!aba || aba.codigo !== null) return;
  if (trabalhando && !aba.trabalhandoDesde) aba.trabalhandoDesde = Date.now();
  else if (!trabalhando && aba.trabalhandoDesde) fecharRodada(aba);
  desenhar();
}

function aoInfo(info) {
  const aba = porId(info.id);
  if (!aba) return;
  aba.info = { ...aba.info, ...info, tokens: info.tokens ?? aba.info?.tokens };
  desenhar();
}

async function aoFim(id) {
  const aba = porId(id);
  if (!aba || aba.codigo !== null) return;
  aba.codigo = (await invoke("codigo_saida", { id })) ?? 0;
  fecharRodada(aba);
  desenhar();
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
  const pasta = (aba.info?.pasta || aba.pasta || "").split("/").pop();
  const generico = /^[^\s@]+@[^\s:]+(:|$)/.test(t) || t === pasta || /^(claude code|codex|claude)$/i.test(t);
  return generico ? "" : t;
}

// Cor fixa de cada conta: Next verde, Nuveto roxo, Codex azul (Shell sem cor)
function corDe(aba) {
  if (aba.tipo === "codex") return "cor-codex";
  if (aba.tipo !== "claude") return "";
  const n = (aba.conta?.nome || "").toLowerCase();
  return n.includes("nuveto") ? "cor-nuveto" : n.includes("next") ? "cor-next" : "";
}

// "Claude · Nuveto", "Codex", "Shell"
function rotulo(aba) {
  return aba.tipo === "claude" && aba.conta && contas.length > 1 ? `Claude · ${aba.conta.nome}` : NOME[aba.tipo];
}

// Projeto em que a aba está: pasta do repositório git, ou a pasta atual
function workspace(aba) {
  return aba.info?.workspace || (aba.pasta || "").split("/").pop() || "—";
}

// Nome curto (abas, lista de sessões)
function nomeCurto(aba) {
  const t = tituloUtil(aba);
  if (t) return t;
  return aba.tipo === "shell" ? `Shell · ${curto(aba.info?.pasta || aba.pasta)}` : rotulo(aba);
}

// Nome com o agente na frente (cabeçalho do terminal)
function nomeCompleto(aba) {
  const t = tituloUtil(aba);
  return t ? `${rotulo(aba)} · ${t}` : nomeCurto(aba);
}

// ---------- desenho ----------

function desenhar() {
  const a = abas[ativa];
  const LIXO = '<svg viewBox="0 0 24 24"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/></svg>';
  const X = '<svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18"/></svg>';

  // abas
  pôr("#abas", abas.map((x, i) => `
    <button class="aba ${corDe(x)} ${i === ativa ? "ativa" : ""}" data-i="${i}" title="${esc(`${rotulo(x)} · ${nomeCurto(x)} — ${workspace(x)}`)}">
      <span class="ic ${estadoDe(x)}"></span>
      <span class="nome">${esc(nomeCurto(x))}</span>
      <span class="ws">${esc(workspace(x))}</span>
      <span class="fechar" data-fechar="${i}">${X}</span>
    </button>`).join("") + `
    <button class="aba-mais" id="btn-mais" title="Nova aba (Ctrl+Shift+T)">
      <svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
    </button>`);

  // quadros (títulos na grade)
  abas.forEach((x) => {
    x.el.className = `quadro ${corDe(x)} ${x === a ? "ativo" : ""}`;
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

  desenharLimites();

  // sessões
  $("#n-sessoes").textContent = abas.length;
  pôr("#sessoes", abas.map((x, i) => {
    const e = estadoDe(x);
    const desc = e === "trabalhando" ? `trabalhando desde ${horaDe(x.trabalhandoDesde)}`
      : e === "pronto" ? "terminou, te esperando"
      : e === "encerrado" ? `encerrou (código ${x.codigo})`
      : "parado";
    return `
    <li class="${corDe(x)} ${i === ativa ? "ativa" : ""}" data-i="${i}">
      <span class="ic ${e}"></span>
      <b>${esc(nomeCurto(x))}</b>
      <span class="tag conta">${esc(x.tipo === "claude" && x.conta && contas.length > 1 ? x.conta.nome : NOME[x.tipo])}</span>
      <button class="lixo" data-fechar="${i}" title="Fechar conversa">${LIXO}</button>
      <small>${esc(`${workspace(x)} · ${desc}`)}</small>
    </li>`;
  }).join(""));

  // cabeçalho do terminal
  if (a) {
    $("#ba-ic").className = `ic ${estadoDe(a)}`;
    $("#ba-titulo").textContent = nomeCompleto(a);
    $("#ba-pasta").textContent = workspace(a);
    $("#ba-pasta").title = curto(a.info?.pasta || a.pasta);
    const br = $("#ba-branch");
    br.hidden = !a.info?.branch;
    br.textContent = a.info?.branch ? `⎇ ${a.info.branch}` : "";
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
    : op === "trabalhando" ? `${rotulo(a)} trabalhando em ${workspace(a)} desde ${horaDe(a.trabalhandoDesde)}.`
    : op === "pronto" ? `${rotulo(a)} terminou em ${workspace(a)} e está te esperando.`
    : op === "encerrado" ? `Saiu com código ${a.codigo}.`
    : `${rotulo(a)} parado em ${workspace(a)}, esperando você.`;

  // detalhes
  if (a) {
    const tk = a.info?.tokens;
    const pares = [
      ...(a.tipo === "claude" && a.conta ? [["Conta", a.conta.nome]] : []),
      ...(tk ? [
        ["Modelo", modelo(tk.modelo)],
        ["Contexto", numero(tk.contexto)],
        ["Tokens usados", numero(tk.entrada + tk.cache_escrita + tk.saida)],
      ] : []),
    ];
    pôr("#sinal", pares.map(([k, v]) => `<dt>${k}</dt><dd>${esc(String(v))}</dd>`).join(""));
  } else {
    pôr("#sinal", "");
  }


  document.title = a ? `easynow · ${nomeCurto(a)}` : "easynow";
}

// Só os limites da conta da aba que você está vendo (Shell não tem).
function desenharLimites() {
  const a = abas[ativa];
  const lista = limites.filter((l) =>
    a?.tipo === "codex" ? l.chave === "codex"
    : a?.tipo === "claude" ? l.chave === a.conta?.dir
    : false);
  $("#card-limites").hidden = !a || a.tipo === "shell";
  $("#card-limites").className = `card ${a ? corDe(a) : ""}`;
  $("#limites-conta").textContent = a?.tipo === "claude" ? a.conta?.nome || "" : a?.tipo === "codex" ? "Codex" : "";
  pôr("#limites", lista.length ? lista.map((l) => `
    <div class="limite">
      ${l.erro ? `<small class="limite-erro">${esc(l.erro)}</small>` : [["Sessão", l.sessao], ["Semana", l.semana]]
        .filter(([, j]) => j)
        .map(([nome, j]) => `
        <div class="lim-linha ${j.pct >= 85 ? "alto" : ""}">
          <span>${nome}</span>
          <div class="trilho"><i style="width:${Math.min(j.pct, 100)}%"></i></div>
          <b>${Math.round(j.pct)}%</b>
          <small title="zera ${esc(quando(j.zera, true))}">${esc(quando(j.zera))}</small>
        </div>`).join("")}
    </div>`).join("") : `<small class="limite-erro">carregando…</small>`);
}

// ---------- menu de nova aba ----------

function opcoesNovo() {
  return [
    ...contas.map((c) => ({ tipo: "claude", conta: c, titulo: contas.length > 1 ? `Claude · ${c.nome}` : "Claude", sub: curto(c.dir) })),
    { tipo: "codex", titulo: "Codex", sub: "OpenAI Codex" },
    { tipo: "shell", titulo: "Shell", sub: "Terminal comum" },
  ];
}

function abrirMenu() {
  const menu = $("#menu-novo");
  menu.innerHTML = opcoesNovo().map((o, i) =>
    `<button data-opcao="${i}" class="${corDe(o)}"><b>${esc(o.titulo)}</b><span>${esc(o.sub)}</span><kbd>${i + 1}</kbd></button>`).join("");
  const r = ($("#btn-mais") || $("#btn-grade")).getBoundingClientRect();
  menu.style.left = `${Math.min(r.left, window.innerWidth - 280)}px`;
  menu.style.top = `${r.bottom + 8}px`;
  menu.hidden = false;
}

function escolher(i) {
  const o = opcoesNovo()[i];
  $("#menu-novo").hidden = true;
  if (o) novaAba(o.tipo, abas[ativa]?.id ?? null, o.conta || null);
}

// ---------- voz (orb) ----------
// Quem ouve e transcreve é o /voice do Claude (segurando espaço). O easynow só mostra
// o orb enquanto o espaço está segurado, reagindo ao volume do microfone.

const voz = { ouvindo: false, nivel: 0, alvo: 0, quadro: 0, saida: 0, ligada: {} };

function conferirVoz(aba) {
  const dir = aba?.tipo === "claude" && aba.conta?.dir;
  if (dir) invoke("voz_ligada", { dir }).then((v) => { voz.ligada[dir] = v; });
}

function começarOrb() {
  voz.ouvindo = true;
  clearTimeout(voz.saida);
  const orb = $("#orb");
  orb.classList.remove("transcrevendo");
  $("#orb-texto").textContent = "Ouvindo…";
  orb.classList.add("visivel");
  invoke("voz_ouvir");
  if (!voz.quadro) voz.quadro = requestAnimationFrame(desenharOrb);
}

function terminarOrb() {
  if (!voz.ouvindo) return;
  voz.ouvindo = false;
  voz.alvo = 0;
  invoke("voz_parar");
  const orb = $("#orb");
  orb.classList.add("transcrevendo");
  $("#orb-texto").textContent = "Transcrevendo…";
  voz.saida = setTimeout(() => {
    orb.classList.remove("visivel");
    // espera o sumiço terminar e para a animação (zero gasto parado)
    setTimeout(() => {
      if (!voz.ouvindo) { cancelAnimationFrame(voz.quadro); voz.quadro = 0; }
    }, 300);
  }, 1200);
}

// Esfera neural: pontos espalhados por igual numa bola (espiral de Fibonacci),
// ligados aos vizinhos mais próximos, girando em 3D. A voz faz a superfície ondular
// e acende pontos e ligações. Os pontos e as ligações são calculados uma vez só.
const esfera = (() => {
  const N = 420;
  const pts = [];
  const ouro = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < N; i++) {
    const y = 1 - (i / (N - 1)) * 2;
    const r = Math.sqrt(1 - y * y);
    const a = ouro * i;
    pts.push([Math.cos(a) * r, y, Math.sin(a) * r]);
  }
  const ligacoes = [];
  pts.forEach((p, i) => {
    const perto = pts
      .map((q, j) => [j, (p[0] - q[0]) ** 2 + (p[1] - q[1]) ** 2 + (p[2] - q[2]) ** 2])
      .filter(([j]) => j > i)
      .sort((a, b) => a[1] - b[1])
      .slice(0, 2);
    perto.forEach(([j]) => ligacoes.push([i, j]));
  });
  return { pts, ligacoes, giro: 0, tela: new Float32Array(N * 4) };
})();

function desenharOrb(t) {
  const c = $("#orb canvas");
  const g = c.getContext("2d");
  const W = c.width, m = W / 2;
  voz.nivel += (voz.alvo - voz.nivel) * 0.18;
  const n = voz.nivel;
  g.clearRect(0, 0, W, W);

  esfera.giro += 0.004 + n * 0.025;
  const ay = esfera.giro, ax = 0.4 + 0.15 * Math.sin(t / 2600);
  const cy = Math.cos(ay), sy = Math.sin(ay), cx = Math.cos(ax), sx = Math.sin(ax);
  const R = W * 0.3;
  const respira = voz.ouvindo ? 0 : 0.03 * Math.sin(t / 260);

  // gira, ondula com a voz e projeta cada ponto na tela
  const tl = esfera.tela;
  esfera.pts.forEach(([x, y, z], i) => {
    const onda = Math.sin(x * 4 + t / 170) * Math.cos(y * 5 - t / 210) * Math.sin(z * 3 + t / 250);
    const d = 1 + respira + n * 0.3 * onda;
    let X = x * cy + z * sy, Z = -x * sy + z * cy;
    let Y = y * cx - Z * sx;
    Z = y * sx + Z * cx;
    const perto = 2.6 / (2.6 - Z); // perspectiva
    tl[i * 4] = m + X * d * R * perto;
    tl[i * 4 + 1] = m + Y * d * R * perto;
    tl[i * 4 + 2] = (Z + 1) / 2; // 0 = atrás, 1 = na frente
    tl[i * 4 + 3] = Math.max(0, onda) * n; // quanto a voz acendeu esse ponto
  });

  // ligações (mais fracas atrás, mais fortes com a voz)
  g.lineWidth = W / 500;
  for (const [a, b] of esfera.ligacoes) {
    const prof = (tl[a * 4 + 2] + tl[b * 4 + 2]) / 2;
    const luz = (tl[a * 4 + 3] + tl[b * 4 + 3]) / 2;
    g.strokeStyle = `rgba(74, 222, 128, ${0.07 + prof * 0.23 + luz * 0.6})`;
    g.beginPath();
    g.moveTo(tl[a * 4], tl[a * 4 + 1]);
    g.lineTo(tl[b * 4], tl[b * 4 + 1]);
    g.stroke();
  }

  // pontos
  for (let i = 0; i < esfera.pts.length; i++) {
    const prof = tl[i * 4 + 2], luz = tl[i * 4 + 3];
    const v = Math.min(1, 0.15 + prof * 0.55 + luz * 1.2);
    g.fillStyle = `hsla(142, 76%, ${Math.round(52 + 38 * luz)}%, ${0.15 + 0.85 * v})`;
    g.beginPath();
    g.arc(tl[i * 4], tl[i * 4 + 1], W * (0.0035 + 0.006 * prof + 0.008 * luz), 0, Math.PI * 2);
    g.fill();
  }

  voz.quadro = requestAnimationFrame(desenharOrb);
}

// ---------- atalhos ----------

// Devolve true se a tecla foi um atalho do easynow (aí ela não vai para o programa).
function atalho(ev) {
  // Espaço segurado numa aba do Claude com /voice ligado: mostra o orb.
  // O espaço sempre segue para o Claude (é ele quem grava).
  if (ev.code === "Space" && !ev.ctrlKey && !ev.altKey && !ev.metaKey) {
    const a = abas[ativa];
    if (ev.type === "keydown" && ev.repeat && !voz.ouvindo && a?.tipo === "claude" && voz.ligada[a.conta?.dir]) {
      começarOrb();
    } else if (ev.type === "keyup") {
      terminarOrb();
    }
    return false;
  }
  if (ev.type !== "keydown") return false;
  const k = ev.key.toLowerCase();

  if (!$("#modal").hidden) {
    if (k === "s" || k === "y" || k === "enter") fecharModal(true);
    else if (k === "n" || k === "escape") fecharModal(false);
    return true;
  }
  if (!$("#menu-novo").hidden) {
    if (/^[1-9]$/.test(k)) escolher(Number(k) - 1);
    else $("#menu-novo").hidden = true;
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
    abrirMenu();
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

// soltou o espaço fora do terminal, ou a janela perdeu o foco: some o orb
window.addEventListener("keyup", (ev) => { if (ev.code === "Space") terminarOrb(); });
window.addEventListener("blur", terminarOrb);

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
  if (ev.target.closest("#btn-mais")) {
    ev.stopPropagation();
    return $("#menu-novo").hidden ? abrirMenu() : ($("#menu-novo").hidden = true);
  }
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
    const f = ev.target.closest("[data-fechar]");
    if (f) return pedirFechar(Number(f.dataset.fechar));
    const li = ev.target.closest("[data-i]");
    if (li) ativar(Number(li.dataset.i));
  });
}
document.addEventListener("click", (ev) => {
  const o = ev.target.closest("[data-opcao]");
  const t = ev.target.closest("[data-tipo]");
  if (o) escolher(Number(o.dataset.opcao));
  else if (t) escolher(opcoesNovo().findIndex((x) => x.tipo === t.dataset.tipo));
  else if (!ev.target.closest(".menu")) $("#menu-novo").hidden = true;
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
function hora() {
  return new Date().toLocaleTimeString("pt-BR", { hour12: false });
}
function tamanho(b) {
  if (b >= 1e9) return `${(b / 1e9).toFixed(1)}G`;
  if (b >= 1e6) return `${Math.round(b / 1e6)}M`;
  if (b >= 1e3) return `${Math.round(b / 1e3)}K`;
  return `${b}B`;
}
// "claude-opus-5-5" -> "Opus 5.5"
function modelo(m) {
  if (!m) return "—";
  if (!m.startsWith("claude-")) return m;
  const t = m.slice(7).replace(/-(\d+)-(\d+)(-\d{8})?$/, " $1.$2").replace(/-(\d+)$/, " $1");
  return t.charAt(0).toUpperCase() + t.slice(1);
}
// troca o HTML só se mudou (evita refazer a tela à toa)
function pôr(sel, html) {
  const el = $(sel);
  if (el._html !== html) {
    el._html = html;
    el.innerHTML = html;
  }
}
function horaDe(ms) {
  return new Date(ms).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" });
}
function numero(n) {
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return String(n);
}
// quando um limite zera: "18:59" se for hoje, "sex 08:59" se for outro dia
function quando(z, completo = false) {
  if (z == null) return "—";
  const d = typeof z === "number" ? new Date(z * 1000) : new Date(z);
  if (isNaN(d)) return "—";
  const h = d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" });
  if (completo) return d.toLocaleString("pt-BR", { weekday: "long", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === new Date().toDateString()) return h;
  return `${d.toLocaleDateString("pt-BR", { weekday: "short" }).replace(".", "")} ${h}`;
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
  await listen("estado", ({ payload }) => aoEstado(payload));
  await listen("info", ({ payload }) => aoInfo(payload));
  await listen("fim", ({ payload }) => aoFim(payload));
  await listen("limites", ({ payload }) => { limites = payload; desenhar(); });
  await listen("nivel", ({ payload }) => { voz.alvo = payload; });

  const ini = await invoke("inicio");
  home = ini.home;
  contas = ini.contas || [];

  for (const tipo of ini.abas) await novaAba(tipo);
  ativar(0);
})();
