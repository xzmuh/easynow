// Tudo que é desenhado: barra de abas, painel da esquerda (sessões, sistema,
// eventos), terminal(is) no centro, painel do agente ativo na direita e rodapé.

use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};
use ratatui::Frame;

use crate::aba::{Aba, Estado};
use crate::{App, Modo};

const LARANJA: Color = Color::Rgb(226, 124, 60);
const TEXTO: Color = Color::Rgb(214, 204, 192);
const APAGADO: Color = Color::Rgb(122, 112, 102);
const LINHA: Color = Color::Rgb(66, 58, 52);
const VERDE: Color = Color::Rgb(130, 198, 122);
const VERMELHO: Color = Color::Rgb(214, 96, 86);
const FUNDO_ATIVO: Color = Color::Rgb(44, 38, 34);

const LARGURA_ESQUERDA: u16 = 30;
const LARGURA_DIREITA: u16 = 34;

pub struct Disposicao {
    pub barra: Rect,
    pub esquerda: Option<Rect>,
    pub centro: Rect,
    pub direita: Option<Rect>,
    pub rodape: Rect,
}

pub fn dispor(area: Rect, paineis: bool) -> Disposicao {
    let [barra, meio, rodape] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);

    // Tela estreita: some com os painéis e fica só o terminal.
    let cabe = area.width >= LARGURA_ESQUERDA + LARGURA_DIREITA + 60;
    if !paineis || !cabe {
        return Disposicao { barra, esquerda: None, centro: meio, direita: None, rodape };
    }
    let [esquerda, centro, direita] = Layout::horizontal([
        Constraint::Length(LARGURA_ESQUERDA),
        Constraint::Min(40),
        Constraint::Length(LARGURA_DIREITA),
    ])
    .areas(meio);
    Disposicao { barra, esquerda: Some(esquerda), centro, direita: Some(direita), rodape }
}

/// Onde cada terminal aparece: só a aba ativa, ou todas em grade.
pub fn quadros(centro: Rect, total: usize, ativa: usize, grade: bool) -> Vec<(Rect, usize)> {
    if total == 0 {
        return vec![];
    }
    if !grade || total == 1 {
        return vec![(centro, ativa)];
    }
    let colunas = (total as f64).sqrt().ceil() as usize;
    let linhas = total.div_ceil(colunas);
    let faixas = Layout::vertical(vec![Constraint::Ratio(1, linhas as u32); linhas]).split(centro);
    let mut saida = vec![];
    for (l, faixa) in faixas.iter().enumerate() {
        let nesta = (total - l * colunas).min(colunas);
        let celulas =
            Layout::horizontal(vec![Constraint::Ratio(1, nesta as u32); nesta]).split(*faixa);
        for (c, r) in celulas.iter().enumerate() {
            saida.push((*r, l * colunas + c));
        }
    }
    saida
}

pub fn interno(r: Rect) -> Rect {
    r.inner(Margin { horizontal: 1, vertical: 1 })
}

pub fn desenhar(f: &mut Frame, app: &mut App) {
    let d = dispor(f.area(), app.paineis);
    let qs = quadros(d.centro, app.abas.len(), app.ativa, app.grade);

    // Cada PTY precisa saber o tamanho exato do quadro onde aparece.
    if app.grade {
        for (r, i) in &qs {
            let r = interno(*r);
            app.abas[*i].redimensionar(r.height, r.width);
        }
    } else {
        let r = interno(d.centro);
        for aba in &mut app.abas {
            aba.redimensionar(r.height, r.width);
        }
    }

    barra(f, app, d.barra);
    if let Some(r) = d.esquerda {
        painel_esquerdo(f, app, r);
    }
    if let Some(r) = d.direita {
        painel_direito(f, app, r);
    }
    if qs.is_empty() {
        let txt = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("Nenhuma aba aberta", Style::new().fg(TEXTO))),
            Line::from(Span::styled(
                "Alt+t para abrir claude, codex ou shell",
                Style::new().fg(APAGADO),
            )),
        ])
        .centered()
        .block(bloco("", false));
        f.render_widget(txt, d.centro);
    }
    for (r, i) in &qs {
        quadro_terminal(f, app, *r, *i, app.grade && *i == app.ativa);
    }
    app.areas_quadros = qs;
    rodape(f, app, d.rodape);
}

fn bloco(titulo: &str, destaque: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(if destaque { LARANJA } else { LINHA }))
        .title(Span::styled(
            format!(" {titulo} "),
            Style::new().fg(if destaque { LARANJA } else { APAGADO }).add_modifier(Modifier::BOLD),
        ))
}

fn girando() -> char {
    const QUADROS: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    QUADROS[(ms / 100) as usize % QUADROS.len()]
}

fn icone(aba: &Aba) -> (String, Color) {
    match aba.estado() {
        Estado::Encerrado => ("×".into(), VERMELHO),
        Estado::Trabalhando => (girando().to_string(), LARANJA),
        Estado::Parado if aba.aviso => ("✓".into(), VERDE),
        Estado::Parado => (" ".into(), APAGADO),
    }
}

fn estado_texto(aba: &Aba) -> (String, Color) {
    match aba.estado() {
        Estado::Encerrado => (format!("encerrou ({})", aba.codigo_saida.unwrap_or(0)), VERMELHO),
        Estado::Trabalhando => {
            let t = aba.trabalhando_desde.map(|i| i.elapsed()).unwrap_or_default();
            (format!("trabalhando {}", duracao(t)), LARANJA)
        }
        Estado::Parado if aba.aviso => ("pronto ✓".into(), VERDE),
        Estado::Parado => ("parado".into(), APAGADO),
    }
}

fn barra(f: &mut Frame, app: &mut App, r: Rect) {
    let mut x = r.x;
    let mut spans = vec![Span::styled(
        " CENTRAL ",
        Style::new().fg(LARANJA).add_modifier(Modifier::BOLD),
    )];
    x += 9;
    app.areas_abas.clear();
    for (i, aba) in app.abas.iter().enumerate() {
        let (ic, cor) = icone(aba);
        let nome = cortar(&aba.nome_curto(), 24);
        let texto = format!(" {} {} {} ", i + 1, ic, nome);
        let largura = texto.chars().count() as u16;
        if x + largura > r.right() {
            spans.push(Span::styled(" …", Style::new().fg(APAGADO)));
            break;
        }
        let ativa = i == app.ativa;
        let base = if ativa {
            Style::new().bg(FUNDO_ATIVO).fg(TEXTO).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(APAGADO)
        };
        spans.push(Span::styled(format!(" {} ", i + 1), base.fg(if ativa { LARANJA } else { APAGADO })));
        spans.push(Span::styled(ic.clone(), base.fg(cor)));
        spans.push(Span::styled(format!(" {nome} "), base));
        app.areas_abas.push((Rect::new(x, r.y, largura, 1), i));
        x += largura + 1;
        spans.push(Span::raw(" "));
    }
    f.render_widget(Line::from(spans), r);
}

fn painel_esquerdo(f: &mut Frame, app: &App, r: Rect) {
    let altura_sessoes = (app.abas.len() as u16 * 3 + 2).max(5).min(r.height / 2 + 4);
    let [sessoes, sistema, eventos] = Layout::vertical([
        Constraint::Length(altura_sessoes),
        Constraint::Length(7),
        Constraint::Min(3),
    ])
    .areas(r);

    // Sessões
    let mut linhas = vec![];
    for (i, aba) in app.abas.iter().enumerate() {
        let (txt, cor) = estado_texto(aba);
        let marca = if i == app.ativa { "▌" } else { " " };
        linhas.push(Line::from(vec![
            Span::styled(marca, Style::new().fg(LARANJA)),
            Span::styled(format!("{} ", i + 1), Style::new().fg(APAGADO)),
            Span::styled(format!("{:<7}", aba.tipo.nome()), Style::new().fg(TEXTO)),
            Span::styled(txt, Style::new().fg(cor)),
        ]));
        let titulo = aba.titulo_limpo();
        linhas.push(Line::from(Span::styled(
            format!("{}   {}", marca, cortar(if titulo.is_empty() { "—" } else { &titulo }, 24)),
            Style::new().fg(APAGADO),
        )));
        linhas.push(Line::from(""));
    }
    let trabalhando = app.abas.iter().filter(|a| a.estado() == Estado::Trabalhando).count();
    f.render_widget(
        Paragraph::new(linhas).block(bloco(
            &format!("SESSÕES {trabalhando}/{} ativas", app.abas.len()),
            false,
        )),
        sessoes,
    );

    // Sistema
    let t = app.telemetria.lock().unwrap().clone();
    let w = sistema.width.saturating_sub(16) as usize;
    let mem_frac = if t.memoria_total > 0 {
        t.memoria_usada as f32 / t.memoria_total as f32
    } else {
        0.0
    };
    let linhas = vec![
        medidor("CPU", t.cpu / 100.0, &format!("{:>3.0}%", t.cpu), w),
        medidor("RAM", mem_frac, &format!("{:>3.0}%", mem_frac * 100.0), w),
        Line::from(vec![
            Span::styled("     ", Style::new()),
            Span::styled(
                format!("{} / {}", tamanho(t.memoria_usada), tamanho(t.memoria_total)),
                Style::new().fg(APAGADO),
            ),
        ]),
        Line::from(""),
        par("aberto há", duracao(app.iniciou.elapsed())),
    ];
    f.render_widget(Paragraph::new(linhas).block(bloco("SISTEMA", false)), sistema);

    // Eventos
    let largura = eventos.width.saturating_sub(2) as usize;
    let linhas: Vec<Line> = app
        .eventos
        .iter()
        .take(eventos.height.saturating_sub(2) as usize)
        .map(|(quando, txt)| {
            Line::from(vec![
                Span::styled(format!("{:>4} ", desde(*quando)), Style::new().fg(APAGADO)),
                Span::styled(cortar(txt, largura.saturating_sub(5)), Style::new().fg(TEXTO)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(linhas).block(bloco("EVENTOS", false)), eventos);
}

fn painel_direito(f: &mut Frame, app: &App, r: Rect) {
    let [agente, atalhos] =
        Layout::vertical([Constraint::Min(10), Constraint::Length(12)]).areas(r);

    let Some(aba) = app.abas.get(app.ativa) else {
        f.render_widget(bloco("AGENTE ATIVO", false), agente);
        atalhos_bloco(f, atalhos);
        return;
    };
    let uso = aba
        .pid
        .and_then(|p| app.telemetria.lock().unwrap().por_pid.get(&p).cloned())
        .unwrap_or_default();
    let (ultima_saida, bytes, rolagem) = {
        let d = aba.dados.lock().unwrap();
        (d.ultima_saida, d.bytes, d.parser.screen().scrollback())
    };
    let w = agente.width.saturating_sub(4) as usize;
    let (estado, cor) = estado_texto(aba);
    let titulo = aba.titulo_limpo();
    let pasta = uso.pasta.clone().unwrap_or_else(|| aba.pasta_inicial.clone());
    let aberta = aba.aberta_em.elapsed();
    let trabalhado = aba.tempo_trabalhando
        + aba.trabalhando_desde.map(|i| i.elapsed()).unwrap_or_default();
    let pct = if aberta.as_secs() > 0 {
        trabalhado.as_secs_f32() / aberta.as_secs_f32() * 100.0
    } else {
        0.0
    };

    let mut linhas = vec![
        Line::from(vec![
            Span::styled(format!("{} ", app.ativa + 1), Style::new().fg(APAGADO)),
            Span::styled(aba.tipo.nome(), Style::new().fg(LARANJA).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            cortar(if titulo.is_empty() { "sem título" } else { &titulo }, w),
            Style::new().fg(TEXTO),
        )),
        Line::from(Span::styled(estado, Style::new().fg(cor))),
        Line::from(""),
        par("pasta", cortar_inicio(&encurtar_home(&pasta.to_string_lossy()), w.saturating_sub(13))),
        par("branch", uso.branch.unwrap_or_else(|| "—".into())),
        par("rodando", if uso.rodando.is_empty() { "—".into() } else { uso.rodando }),
        Line::from(""),
        par("aberta há", duracao(aberta)),
        par("trabalhou", format!("{} ({pct:.0}%)", duracao(trabalhado))),
        par("rodadas", aba.rodadas.to_string()),
        par("última saída", format!("há {}", desde(ultima_saida))),
        Line::from(""),
        par("CPU", format!("{:.0}%", uso.cpu)),
        par("RAM", tamanho(uso.memoria)),
        par("processos", uso.processos.to_string()),
        par("saída total", tamanho(bytes)),
    ];
    if rolagem > 0 {
        linhas.push(Line::from(""));
        linhas.push(Line::from(Span::styled(
            format!("↑ histórico: {rolagem} linhas acima"),
            Style::new().fg(LARANJA),
        )));
    }
    f.render_widget(Paragraph::new(linhas).block(bloco("AGENTE ATIVO", false)), agente);
    atalhos_bloco(f, atalhos);
}

fn atalhos_bloco(f: &mut Frame, r: Rect) {
    let item = |tecla: &str, o_que: &str| {
        Line::from(vec![
            Span::styled(format!("{tecla:<15}"), Style::new().fg(LARANJA)),
            Span::styled(o_que.to_string(), Style::new().fg(APAGADO)),
        ])
    };
    let linhas = vec![
        item("Alt+1..9", "trocar de aba"),
        item("Ctrl+PgUp/Dn", "aba anterior/próx."),
        item("Alt+t", "nova aba"),
        item("Alt+w", "fechar aba"),
        item("Alt+r", "reiniciar aba"),
        item("Alt+g", "grade / uma só"),
        item("Alt+b", "esconder painéis"),
        item("Shift+PgUp", "histórico"),
        item("Shift+arrastar", "selecionar texto"),
        item("Alt+q", "sair"),
    ];
    f.render_widget(Paragraph::new(linhas).block(bloco("ATALHOS", false)), r);
}

fn quadro_terminal(f: &mut Frame, app: &App, r: Rect, i: usize, destaque: bool) {
    let aba = &app.abas[i];
    let (ic, cor) = icone(aba);
    let mut b = bloco(&format!("{} {}", i + 1, cortar(&aba.nome_curto(), 50)), destaque || !app.grade);
    if !app.grade {
        b = b.border_style(Style::new().fg(LINHA));
    }
    b = b.title(Line::from(Span::styled(format!(" {ic} "), Style::new().fg(cor))).right_aligned());

    let d = aba.dados.lock().unwrap();
    let tela = d.parser.screen();
    if tela.scrollback() > 0 {
        b = b.title_bottom(
            Line::from(Span::styled(
                format!(" ↑ histórico ({} linhas) · Shift+PgDn volta ", tela.scrollback()),
                Style::new().fg(LARANJA),
            ))
            .right_aligned(),
        );
    }
    if let Some(c) = aba.codigo_saida {
        b = b.title_bottom(Span::styled(
            format!(" encerrou (código {c}) · Alt+r reinicia · Alt+w fecha "),
            Style::new().fg(VERMELHO),
        ));
    }
    let dentro = b.inner(r);
    f.render_widget(b, r);
    f.render_widget(Terminal { tela }, dentro);

    let ativa = i == app.ativa && matches!(app.modo, Modo::Normal);
    if ativa && tela.scrollback() == 0 && !tela.hide_cursor() && aba.codigo_saida.is_none() {
        let (l, c) = tela.cursor_position();
        if l < dentro.height && c < dentro.width {
            f.set_cursor_position((dentro.x + c, dentro.y + l));
        }
    }
}

/// Copia a tela do vt100 célula por célula para o buffer do ratatui.
struct Terminal<'a> {
    tela: &'a vt100::Screen,
}

impl Widget for Terminal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for l in 0..area.height {
            for c in 0..area.width {
                let Some(cel) = self.tela.cell(l, c) else { continue };
                if cel.is_wide_continuation() {
                    continue;
                }
                let mut estilo = Style::new().fg(cor(cel.fgcolor())).bg(cor(cel.bgcolor()));
                if cel.bold() {
                    estilo = estilo.add_modifier(Modifier::BOLD);
                }
                if cel.dim() {
                    estilo = estilo.add_modifier(Modifier::DIM);
                }
                if cel.italic() {
                    estilo = estilo.add_modifier(Modifier::ITALIC);
                }
                if cel.underline() {
                    estilo = estilo.add_modifier(Modifier::UNDERLINED);
                }
                if cel.inverse() {
                    estilo = estilo.add_modifier(Modifier::REVERSED);
                }
                let simbolo = cel.contents();
                buf[(area.x + c, area.y + l)]
                    .set_symbol(if simbolo.is_empty() { " " } else { simbolo })
                    .set_style(estilo);
            }
        }
    }
}

fn cor(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn rodape(f: &mut Frame, app: &App, r: Rect) {
    let destaque = Style::new().fg(LARANJA).add_modifier(Modifier::BOLD);
    let normal = Style::new().fg(TEXTO);
    let linha = match app.modo {
        Modo::NovaAba => Line::from(vec![
            Span::styled(" Nova aba:  ", normal),
            Span::styled("c", destaque),
            Span::styled(" claude   ", normal),
            Span::styled("x", destaque),
            Span::styled(" codex   ", normal),
            Span::styled("s", destaque),
            Span::styled(" shell   ", normal),
            Span::styled("Esc", destaque),
            Span::styled(" cancela", Style::new().fg(APAGADO)),
        ]),
        Modo::Fechar => {
            let nome = app.abas.get(app.ativa).map(|a| a.nome_curto()).unwrap_or_default();
            Line::from(vec![
                Span::styled(format!(" Fechar a aba {} ({nome})? O programa será encerrado.  ", app.ativa + 1), normal),
                Span::styled("s", destaque),
                Span::styled(" sim   ", normal),
                Span::styled("n", destaque),
                Span::styled(" não", normal),
            ])
        }
        Modo::Sair => Line::from(vec![
            Span::styled(" Sair do central? Todas as abas serão encerradas.  ", normal),
            Span::styled("s", destaque),
            Span::styled(" sim   ", normal),
            Span::styled("n", destaque),
            Span::styled(" não", normal),
        ]),
        Modo::Normal => match app.eventos.front() {
            Some((quando, txt)) if quando.elapsed() < Duration::from_secs(8) => Line::from(vec![
                Span::styled(" ● ", Style::new().fg(VERDE)),
                Span::styled(txt.clone(), normal),
            ]),
            _ => Line::from(Span::styled(
                " Alt+1..9 abas · Alt+t nova · Alt+w fecha · Alt+g grade · Alt+b painéis · Alt+q sai",
                Style::new().fg(APAGADO),
            )),
        },
    };
    f.render_widget(linha, r);
}

fn medidor(rotulo: &str, frac: f32, valor: &str, largura: usize) -> Line<'static> {
    let cheio = ((frac.clamp(0.0, 1.0)) * largura as f32).round() as usize;
    let cor_barra = if frac > 0.85 { VERMELHO } else { LARANJA };
    Line::from(vec![
        Span::styled(format!("{rotulo:<4} "), Style::new().fg(APAGADO)),
        Span::styled("━".repeat(cheio), Style::new().fg(cor_barra)),
        Span::styled("━".repeat(largura - cheio), Style::new().fg(LINHA)),
        Span::styled(format!(" {valor}"), Style::new().fg(TEXTO)),
    ])
}

fn par(rotulo: &str, valor: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{rotulo:<13}"), Style::new().fg(APAGADO)),
        Span::styled(valor, Style::new().fg(TEXTO)),
    ])
}

pub fn duracao(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

fn desde(i: Instant) -> String {
    let s = i.elapsed().as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

fn tamanho(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1}G", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0}M", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0}K", b / 1e3)
    } else {
        format!("{bytes}B")
    }
}

fn cortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn cortar_inicio(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        let t: String = s.chars().skip(n - max.saturating_sub(1)).collect();
        format!("…{t}")
    }
}

fn encurtar_home(s: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s.to_string(),
    }
}
