mod aba;
mod teclas;
mod tela;
mod telemetria;

use std::collections::VecDeque;
use std::io::stdout;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton,
    MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::SetTitle;
use ratatui::layout::{Position, Rect};

use aba::{Aba, Tipo};
use telemetria::Telemetria;

pub enum Modo {
    Normal,
    NovaAba,
    Fechar,
    Sair,
}

pub struct App {
    pub abas: Vec<Aba>,
    pub ativa: usize,
    pub grade: bool,
    pub paineis: bool,
    pub modo: Modo,
    pub eventos: VecDeque<(Instant, String)>,
    pub telemetria: Arc<Mutex<Telemetria>>,
    pub areas_abas: Vec<(Rect, usize)>,
    pub areas_quadros: Vec<(Rect, usize)>,
    pub iniciou: Instant,
    pids: Arc<Mutex<Vec<u32>>>,
    sujo: Arc<AtomicBool>,
    sair: bool,
}

impl App {
    fn abrir(&mut self, tipo: Tipo, pasta: PathBuf) {
        let (l, c) = self.abas.get(self.ativa).map(|a| a.tamanho).unwrap_or((24, 80));
        match Aba::abrir(tipo, pasta, l, c, self.sujo.clone()) {
            Ok(aba) => {
                self.abas.push(aba);
                self.ativar(self.abas.len() - 1);
                self.evento(self.abas.len() - 1, "aberta".into());
            }
            Err(e) => self.eventos.push_front((Instant::now(), format!("erro ao abrir {}: {e}", tipo.nome()))),
        }
        self.atualizar_pids();
    }

    /// Pasta onde a aba ativa está agora (para a nova aba abrir no mesmo lugar).
    fn pasta_atual(&self) -> PathBuf {
        self.abas
            .get(self.ativa)
            .and_then(|a| a.pid)
            .and_then(|p| std::fs::read_link(format!("/proc/{p}/cwd")).ok())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    fn ativar(&mut self, i: usize) {
        if i < self.abas.len() {
            self.ativa = i;
            self.abas[i].aviso = false;
        }
    }

    fn fechar(&mut self, i: usize) {
        if i >= self.abas.len() {
            return;
        }
        let nome = self.abas[i].tipo.nome();
        self.abas.remove(i);
        self.eventos.push_front((Instant::now(), format!("{} {nome}: fechada", i + 1)));
        if self.ativa >= self.abas.len() {
            self.ativa = self.abas.len().saturating_sub(1);
        }
        self.atualizar_pids();
    }

    fn reiniciar(&mut self, i: usize) {
        let Some(aba) = self.abas.get(i) else { return };
        let (tipo, pasta, (l, c)) = (aba.tipo, aba.pasta_inicial.clone(), aba.tamanho);
        if let Ok(nova) = Aba::abrir(tipo, pasta, l, c, self.sujo.clone()) {
            self.abas[i] = nova;
            self.evento(i, "reiniciada".into());
        }
        self.atualizar_pids();
    }

    fn atualizar_pids(&self) {
        *self.pids.lock().unwrap() = self.abas.iter().filter_map(|a| a.pid).collect();
    }

    fn evento(&mut self, i: usize, txt: String) {
        let nome = self.abas[i].tipo.nome();
        self.eventos.push_front((Instant::now(), format!("{} {nome}: {txt}", i + 1)));
        self.eventos.truncate(50);
    }

    fn atualizar_abas(&mut self) {
        for i in 0..self.abas.len() {
            let ativa = i == self.ativa;
            if let Some(txt) = self.abas[i].atualizar(ativa) {
                self.evento(i, txt);
            }
        }
    }

    fn tecla(&mut self, k: KeyEvent) {
        let sim = matches!(k.code, KeyCode::Char('s' | 'S' | 'y' | 'Y') | KeyCode::Enter);
        match self.modo {
            Modo::NovaAba => {
                self.modo = Modo::Normal;
                let tipo = match k.code {
                    KeyCode::Char(c) => Tipo::de_texto(&c.to_ascii_lowercase().to_string()),
                    _ => None,
                };
                if let Some(t) = tipo {
                    let pasta = self.pasta_atual();
                    self.abrir(t, pasta);
                }
                return;
            }
            Modo::Fechar => {
                self.modo = Modo::Normal;
                if sim {
                    self.fechar(self.ativa);
                }
                return;
            }
            Modo::Sair => {
                self.modo = Modo::Normal;
                self.sair = sim;
                return;
            }
            Modo::Normal => {}
        }

        let alt = k.modifiers.contains(KeyModifiers::ALT) && !k.modifiers.contains(KeyModifiers::CONTROL);
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let n = self.abas.len();

        match k.code {
            KeyCode::Char(c @ '1'..='9') if alt => self.ativar(c as usize - '1' as usize),
            KeyCode::Char('t') if alt => self.modo = Modo::NovaAba,
            KeyCode::Char('w') if alt && n > 0 => {
                if self.abas[self.ativa].codigo_saida.is_some() {
                    self.fechar(self.ativa);
                } else {
                    self.modo = Modo::Fechar;
                }
            }
            KeyCode::Char('r') if alt && n > 0 && self.abas[self.ativa].codigo_saida.is_some() => {
                self.reiniciar(self.ativa)
            }
            KeyCode::Char('g') if alt => self.grade = !self.grade,
            KeyCode::Char('b') if alt => self.paineis = !self.paineis,
            KeyCode::Char('q') if alt => {
                if n == 0 {
                    self.sair = true;
                } else {
                    self.modo = Modo::Sair;
                }
            }
            KeyCode::PageUp if ctrl && n > 0 => self.ativar((self.ativa + n - 1) % n),
            KeyCode::PageDown if ctrl && n > 0 => self.ativar((self.ativa + 1) % n),
            KeyCode::PageUp if shift && n > 0 => {
                let meia = (self.abas[self.ativa].tamanho.0 / 2).max(1) as isize;
                self.abas[self.ativa].rolar(meia);
            }
            KeyCode::PageDown if shift && n > 0 => {
                let meia = (self.abas[self.ativa].tamanho.0 / 2).max(1) as isize;
                self.abas[self.ativa].rolar(-meia);
            }
            _ if n > 0 => {
                let aba = &mut self.abas[self.ativa];
                let app_cursor = aba.dados.lock().unwrap().parser.screen().application_cursor();
                aba.escrever(&teclas::para_bytes(&k, app_cursor));
            }
            _ => {}
        }
    }

    fn colar(&mut self, texto: String) {
        let Some(aba) = self.abas.get_mut(self.ativa) else { return };
        let colchetes = aba.dados.lock().unwrap().parser.screen().bracketed_paste();
        let texto = texto.replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if colchetes {
            format!("\x1b[200~{texto}\x1b[201~")
        } else {
            texto
        };
        aba.escrever(bytes.as_bytes());
    }

    fn mouse(&mut self, m: MouseEvent) {
        let p = Position::new(m.column, m.row);
        let quadro = self.areas_quadros.iter().find(|(r, _)| r.contains(p)).map(|(_, i)| *i);
        match m.kind {
            MouseEventKind::ScrollUp => {
                if let Some(i) = quadro {
                    self.abas[i].rolar(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(i) = quadro {
                    self.abas[i].rolar(-3);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, i)) = self.areas_abas.iter().find(|(r, _)| r.contains(p)) {
                    self.ativar(*i);
                } else if let Some(i) = quadro {
                    self.ativar(i);
                }
            }
            _ => {}
        }
    }
}

fn main() -> Result<()> {
    // easynow claude codex claude shell → abre essas abas (sem nada: claude, codex e shell)
    let mut tipos = vec![];
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("uso: easynow [claude|codex|shell ...]\n\nsem nada abre claude, codex e shell.\nex.: easynow claude claude codex shell");
                return Ok(());
            }
            a => match Tipo::de_texto(a) {
                Some(t) => tipos.push(t),
                None => anyhow::bail!("não conheço \"{a}\" (use claude, codex ou shell)"),
            },
        }
    }
    if tipos.is_empty() {
        tipos = vec![Tipo::Claude, Tipo::Codex, Tipo::Shell];
    }

    let mut terminal = ratatui::init();
    let teclado_novo = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    if teclado_novo {
        // Faz o terminal diferenciar Shift+Enter de Enter, Esc de Alt, etc.
        execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }

    let resultado = rodar(&mut terminal, tipos);

    if teclado_novo {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(stdout(), DisableBracketedPaste, DisableMouseCapture, SetTitle(""));
    ratatui::restore();
    resultado
}

fn rodar(terminal: &mut ratatui::DefaultTerminal, tipos: Vec<Tipo>) -> Result<()> {
    let pids = Arc::new(Mutex::new(vec![]));
    let mut app = App {
        abas: vec![],
        ativa: 0,
        grade: false,
        paineis: true,
        modo: Modo::Normal,
        eventos: VecDeque::new(),
        telemetria: telemetria::iniciar(pids.clone()),
        areas_abas: vec![],
        areas_quadros: vec![],
        iniciou: Instant::now(),
        pids,
        sujo: Arc::new(AtomicBool::new(true)),
        sair: false,
    };

    // Descobre o tamanho do centro antes de abrir, para os programas já nascerem no tamanho certo.
    let area = terminal.size()?;
    let d = tela::dispor(Rect::new(0, 0, area.width, area.height), true);
    let dentro = tela::interno(d.centro);
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    for t in tipos {
        if let Ok(aba) = Aba::abrir(t, pasta.clone(), dentro.height, dentro.width, app.sujo.clone()) {
            app.abas.push(aba);
        } else {
            app.eventos.push_front((Instant::now(), format!("erro ao abrir {}", t.nome())));
        }
    }
    app.atualizar_pids();

    let mut ultimo_desenho = Instant::now() - Duration::from_secs(1);
    let mut ultimo_titulo = String::new();
    while !app.sair {
        app.atualizar_abas();

        let precisa = app.sujo.swap(false, Ordering::Relaxed)
            || ultimo_desenho.elapsed() > Duration::from_millis(200);
        if precisa {
            terminal.draw(|f| tela::desenhar(f, &mut app))?;
            ultimo_desenho = Instant::now();

            let titulo = match app.abas.get(app.ativa) {
                Some(a) => format!("easynow · {}", a.nome_curto()),
                None => "easynow".into(),
            };
            if titulo != ultimo_titulo {
                let _ = execute!(stdout(), SetTitle(&titulo));
                ultimo_titulo = titulo;
            }
        }

        if event::poll(Duration::from_millis(16))? {
            // pega tudo que chegou de uma vez (colar texto gera muitos eventos)
            loop {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => app.tecla(k),
                    Event::Paste(s) => app.colar(s),
                    Event::Mouse(m) => app.mouse(m),
                    _ => {}
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
            app.sujo.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}
