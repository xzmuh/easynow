// Uma aba = um programa (claude, codex ou shell) rodando num terminal virtual (PTY).
// O programa escreve na PTY, uma thread lê essa saída e joga no `vt100::Parser`,
// que monta a "tela" que depois desenhamos no meio da interface.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

const LINHAS_HISTORICO: usize = 5000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Claude,
    Codex,
    Shell,
}

impl Tipo {
    pub fn nome(self) -> &'static str {
        match self {
            Tipo::Claude => "claude",
            Tipo::Codex => "codex",
            Tipo::Shell => "shell",
        }
    }

    pub fn de_texto(s: &str) -> Option<Tipo> {
        match s {
            "claude" | "c" => Some(Tipo::Claude),
            "codex" | "x" => Some(Tipo::Codex),
            "shell" | "s" | "sh" | "bash" | "zsh" | "fish" => Some(Tipo::Shell),
            _ => None,
        }
    }

    fn programa(self) -> String {
        match self {
            Tipo::Claude => "claude".into(),
            Tipo::Codex => "codex".into(),
            Tipo::Shell => std::env::var("SHELL").unwrap_or_else(|_| "bash".into()),
        }
    }
}

type Escritor = Arc<Mutex<Box<dyn Write + Send>>>;

/// Recebe avisos do parser enquanto ele lê a saída do programa.
pub struct Avisos {
    pub titulo: Option<String>,
    pub sino: bool,
    /// Respostas para perguntas que o programa faz ao terminal
    /// (posição do cursor, cor de fundo...). Sem isso o codex trava esperando.
    respostas: Vec<u8>,
}

impl vt100::Callbacks for Avisos {
    fn set_window_title(&mut self, _: &mut vt100::Screen, titulo: &[u8]) {
        let t = String::from_utf8_lossy(titulo).trim().to_string();
        self.titulo = if t.is_empty() { None } else { Some(t) };
    }

    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.sino = true;
    }

    fn unhandled_csi(
        &mut self,
        tela: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let p0 = params.first().and_then(|p| p.first().copied()).unwrap_or(0);
        match (i1, c) {
            // ESC[6n: onde está o cursor?
            (None, 'n') if p0 == 6 => {
                let (l, col) = tela.cursor_position();
                self.respostas
                    .extend(format!("\x1b[{};{}R", l + 1, col + 1).into_bytes());
            }
            // ESC[5n: o terminal está ok?
            (None, 'n') if p0 == 5 => self.respostas.extend(b"\x1b[0n"),
            // ESC[c: que terminal é você?
            (None, 'c') if p0 == 0 => self.respostas.extend(b"\x1b[?62;22c"),
            (Some(b'>'), 'c') => self.respostas.extend(b"\x1b[>1;10;0c"),
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        // ESC]10;? e ESC]11;?: qual a cor do texto / do fundo?
        match params {
            [b"10", b"?"] => self.respostas.extend(b"\x1b]10;rgb:dcdc/d2d2/c8c8\x1b\\"),
            [b"11", b"?"] => self.respostas.extend(b"\x1b]11;rgb:1717/1414/1212\x1b\\"),
            _ => {}
        }
    }
}

/// O que a thread de leitura e a tela principal dividem.
pub struct Compartilhado {
    pub parser: vt100::Parser<Avisos>,
    pub ultima_saida: Instant,
    pub bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    Trabalhando,
    Parado,
    Encerrado,
}

pub struct Aba {
    pub tipo: Tipo,
    pub dados: Arc<Mutex<Compartilhado>>,
    master: Box<dyn MasterPty + Send>,
    escritor: Escritor,
    filho: Box<dyn Child + Send + Sync>,
    pub pid: Option<u32>,
    pub pasta_inicial: PathBuf,
    pub aberta_em: Instant,
    pub ultima_entrada: Instant,
    pub tamanho: (u16, u16),
    pub titulo: String,
    pub codigo_saida: Option<u32>,

    // telemetria calculada a cada volta do loop
    pub trabalhando_desde: Option<Instant>,
    pub tempo_trabalhando: Duration,
    pub rodadas: u32,
    /// Terminou alguma coisa enquanto você olhava outra aba.
    pub aviso: bool,
}

impl Aba {
    pub fn abrir(
        tipo: Tipo,
        pasta: PathBuf,
        linhas: u16,
        colunas: u16,
        sujo: Arc<AtomicBool>,
    ) -> Result<Aba> {
        let pty = native_pty_system();
        let par = pty.openpty(PtySize {
            rows: linhas,
            cols: colunas,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(tipo.programa());
        cmd.cwd(&pasta);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let filho = par.slave.spawn_command(cmd)?;
        drop(par.slave);

        let mut leitor = par.master.try_clone_reader()?;
        let escritor: Escritor = Arc::new(Mutex::new(par.master.take_writer()?));

        let avisos = Avisos { titulo: None, sino: false, respostas: Vec::new() };
        let dados = Arc::new(Mutex::new(Compartilhado {
            parser: vt100::Parser::new_with_callbacks(linhas, colunas, LINHAS_HISTORICO, avisos),
            ultima_saida: Instant::now(),
            bytes: 0,
        }));

        {
            let dados = dados.clone();
            let escritor = escritor.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    let n = match leitor.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let respostas = {
                        let mut d = dados.lock().unwrap();
                        d.parser.process(&buf[..n]);
                        d.ultima_saida = Instant::now();
                        d.bytes += n as u64;
                        std::mem::take(&mut d.parser.callbacks_mut().respostas)
                    };
                    if !respostas.is_empty() {
                        let mut e = escritor.lock().unwrap();
                        let _ = e.write_all(&respostas);
                        let _ = e.flush();
                    }
                    sujo.store(true, Ordering::Relaxed);
                }
                sujo.store(true, Ordering::Relaxed);
            });
        }

        let agora = Instant::now();
        Ok(Aba {
            tipo,
            dados,
            pid: filho.process_id(),
            master: par.master,
            escritor,
            filho,
            pasta_inicial: pasta,
            aberta_em: agora,
            ultima_entrada: agora,
            tamanho: (linhas, colunas),
            titulo: String::new(),
            codigo_saida: None,
            trabalhando_desde: None,
            tempo_trabalhando: Duration::ZERO,
            rodadas: 0,
            aviso: false,
        })
    }

    pub fn escrever(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.codigo_saida.is_some() {
            return;
        }
        self.ultima_entrada = Instant::now();
        let mut e = self.escritor.lock().unwrap();
        let _ = e.write_all(bytes);
        let _ = e.flush();
        // digitou: volta para o fim do histórico
        self.dados.lock().unwrap().parser.screen_mut().set_scrollback(0);
    }

    pub fn redimensionar(&mut self, linhas: u16, colunas: u16) {
        if (linhas, colunas) == self.tamanho || linhas == 0 || colunas == 0 {
            return;
        }
        self.tamanho = (linhas, colunas);
        // o programa vai se redesenhar inteiro; isso não é "trabalhar"
        self.ultima_entrada = Instant::now();
        self.dados.lock().unwrap().parser.screen_mut().set_size(linhas, colunas);
        let _ = self.master.resize(PtySize {
            rows: linhas,
            cols: colunas,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    pub fn rolar(&mut self, delta: isize) {
        let mut d = self.dados.lock().unwrap();
        let tela = d.parser.screen_mut();
        let atual = tela.scrollback() as isize;
        tela.set_scrollback((atual + delta).max(0) as usize);
    }

    pub fn encerrar(&mut self) {
        let _ = self.filho.kill();
    }

    /// Atualiza título, estado e contadores. Devolve um texto se algo
    /// digno de aparecer nos eventos aconteceu.
    pub fn atualizar(&mut self, ativa: bool) -> Option<String> {
        let (ultima_saida, sino, novo_titulo) = {
            let mut d = self.dados.lock().unwrap();
            let cb = d.parser.callbacks_mut();
            let sino = std::mem::take(&mut cb.sino);
            let titulo = cb.titulo.clone();
            (d.ultima_saida, sino, titulo)
        };
        if let Some(t) = novo_titulo {
            self.titulo = t;
        }

        if self.codigo_saida.is_none() {
            if let Ok(Some(status)) = self.filho.try_wait() {
                self.codigo_saida = Some(status.exit_code());
                self.fechar_rodada(ativa);
                return Some(format!("encerrou (código {})", status.exit_code()));
            }
        }

        if sino && !ativa {
            self.aviso = true;
        }

        // "Trabalhando" = saiu coisa na tela há pouco, e não foi só o eco do que você digitou.
        let agora = Instant::now();
        let saida_recente = agora.duration_since(ultima_saida) < Duration::from_millis(1500);
        let nao_e_eco = ultima_saida
            .checked_duration_since(self.ultima_entrada)
            .is_some_and(|d| d > Duration::from_millis(400));
        let ocupada = saida_recente && nao_e_eco;

        match (ocupada, self.trabalhando_desde) {
            (true, None) => self.trabalhando_desde = Some(agora),
            (false, Some(_)) => return self.fechar_rodada(ativa),
            _ => {}
        }
        None
    }

    fn fechar_rodada(&mut self, ativa: bool) -> Option<String> {
        let inicio = self.trabalhando_desde.take()?;
        let durou = Instant::now().duration_since(inicio);
        self.tempo_trabalhando += durou;
        // ignora piscadas curtas (abrir menu, redesenhar a tela) e a abertura do programa
        let na_abertura = inicio.duration_since(self.aberta_em) < Duration::from_secs(5);
        if durou < Duration::from_secs(3) || na_abertura {
            return None;
        }
        self.rodadas += 1;
        if !ativa {
            self.aviso = true;
        }
        Some(format!("terminou ({})", crate::tela::duracao(durou)))
    }

    pub fn estado(&self) -> Estado {
        if self.codigo_saida.is_some() {
            Estado::Encerrado
        } else if self.trabalhando_desde.is_some() {
            Estado::Trabalhando
        } else {
            Estado::Parado
        }
    }

    pub fn nome_curto(&self) -> String {
        let t = self.titulo_limpo();
        if t.is_empty() {
            self.tipo.nome().to_string()
        } else {
            t
        }
    }

    /// Tira os ícones de spinner que o claude/codex põem no começo do título.
    pub fn titulo_limpo(&self) -> String {
        self.titulo
            .trim_start_matches(|c: char| !c.is_alphanumeric() && c != '~' && c != '/')
            .trim()
            .to_string()
    }
}

impl Drop for Aba {
    fn drop(&mut self) {
        let _ = self.filho.kill();
    }
}
