// Uma aba = um programa (claude, codex ou shell) rodando num terminal virtual (PTY).
// Uma thread lê o que o programa escreve e manda para a janela, onde o xterm.js desenha.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use anyhow::Result;
use base64::Engine;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
struct Saida {
    id: u32,
    dados: String,
}

pub struct Atividade {
    pub ultima_saida: Instant,
    pub bytes: u64,
}

pub struct Aba {
    pub pid: Option<u32>,
    pub atividade: Arc<Mutex<Atividade>>,
    pub ultima_entrada: Instant,
    pub codigo_saida: Option<u32>,
    master: Box<dyn MasterPty + Send>,
    escritor: Box<dyn Write + Send>,
    filho: Box<dyn Child + Send + Sync>,
}

fn programa(tipo: &str) -> String {
    match tipo {
        "claude" => "claude".into(),
        "codex" => "codex".into(),
        _ => std::env::var("SHELL").unwrap_or_else(|_| "bash".into()),
    }
}

impl Aba {
    /// `conta`: pasta de configuração do Claude a usar (None = a padrão).
    pub fn abrir(
        app: AppHandle,
        id: u32,
        tipo: &str,
        conta: Option<&str>,
        pasta: PathBuf,
        linhas: u16,
        colunas: u16,
    ) -> Result<Aba> {
        let par = native_pty_system().openpty(PtySize {
            rows: linhas.max(2),
            cols: colunas.max(2),
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(programa(tipo));
        cmd.cwd(&pasta);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // Se o easynow foi aberto de dentro de um Claude, não passa as variáveis dele adiante.
        for (k, _) in std::env::vars() {
            if k == "CLAUDECODE" || k == "CLAUDE_CONFIG_DIR" || k == "CLAUDE_PID" || k.starts_with("CLAUDE_CODE_") {
                cmd.env_remove(k);
            }
        }
        if let Some(dir) = conta {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        let filho = par.slave.spawn_command(cmd)?;
        drop(par.slave);

        let mut leitor = par.master.try_clone_reader()?;
        let escritor = par.master.take_writer()?;
        let atividade = Arc::new(Mutex::new(Atividade { ultima_saida: Instant::now(), bytes: 0 }));

        let ativ = atividade.clone();
        thread::spawn(move || {
            let b64 = base64::engine::general_purpose::STANDARD;
            let mut buf = [0u8; 32 * 1024];
            loop {
                let n = match leitor.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                {
                    let mut a = ativ.lock().unwrap();
                    a.ultima_saida = Instant::now();
                    a.bytes += n as u64;
                }
                // base64 porque a saída é byte cru (pode cortar um caractere no meio)
                let _ = app.emit("saida", Saida { id, dados: b64.encode(&buf[..n]) });
            }
            let _ = app.emit("fim", id);
        });

        let agora = Instant::now();
        Ok(Aba {
            pid: filho.process_id(),
            atividade,
            ultima_entrada: agora,
            codigo_saida: None,
            master: par.master,
            escritor,
            filho,
        })
    }

    pub fn escrever(&mut self, dados: &[u8]) {
        self.ultima_entrada = Instant::now();
        let _ = self.escritor.write_all(dados);
        let _ = self.escritor.flush();
    }

    pub fn redimensionar(&mut self, linhas: u16, colunas: u16) {
        // o programa vai se redesenhar inteiro; isso não conta como "trabalhar"
        self.ultima_entrada = Instant::now();
        let _ = self.master.resize(PtySize {
            rows: linhas.max(2),
            cols: colunas.max(2),
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    pub fn verificar_saida(&mut self) -> Option<u32> {
        if self.codigo_saida.is_none() {
            if let Ok(Some(s)) = self.filho.try_wait() {
                self.codigo_saida = Some(s.exit_code());
            }
        }
        self.codigo_saida
    }
}

impl Drop for Aba {
    fn drop(&mut self) {
        let _ = self.filho.kill();
    }
}
