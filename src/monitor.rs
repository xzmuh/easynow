// Acompanha o estado de cada aba sem ficar perguntando toda hora (sem "polling").
// Tudo aqui dorme até alguma coisa acontecer:
//
// - Claude: grava em <config>/sessions/<pid>.json se está "busy" ou "idle". O Linux
//   avisa (inotify) quando esse arquivo muda; aí lemos o estado e, quando termina, os tokens.
// - Codex: grava "task_started" / "task_complete" e os tokens no arquivo da sessão
//   (~/.codex/sessions/...jsonl). Mesmo esquema: o Linux avisa quando o arquivo cresce.
// - Shell: não grava nada, então a própria saída do terminal acorda a thread da aba;
//   depois de 1,5 s em silêncio ela marca "parado" e volta a dormir.
// - Limites de uso: só são buscados quando você troca de aba ou um agente termina,
//   no máximo uma vez por minuto por conta.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::uso::{self, Conta, Leitor, Limite, Tokens};

const LIMITES_VALEM: Duration = Duration::from_secs(60);

#[derive(Clone, Serialize)]
struct Estado {
    id: u32,
    trabalhando: bool,
}

#[derive(Clone, Serialize)]
struct Info {
    id: u32,
    pasta: Option<String>,
    workspace: Option<String>,
    branch: Option<String>,
    tokens: Option<Tokens>,
}

struct Vigiada {
    tipo: String,
    pid: u32,
    /// pasta de configuração da conta (só Claude)
    config: Option<PathBuf>,
    trabalhando: bool,
    /// arquivo da sessão do Codex, quando já descoberto
    arquivo: Option<PathBuf>,
}

pub struct Monitor {
    app: AppHandle,
    abas: Mutex<HashMap<u32, Vigiada>>,
    leitor: Mutex<Leitor>,
    contas: Vec<Conta>,
    /// chave (pasta da conta ou "codex") -> (quando buscou, limite)
    limites: Mutex<HashMap<String, (Instant, Limite)>>,
    _vigia: Mutex<Option<notify::RecommendedWatcher>>,
}

impl Monitor {
    pub fn iniciar(app: AppHandle, contas: Vec<Conta>) -> Arc<Monitor> {
        let m = Arc::new(Monitor {
            app,
            abas: Mutex::new(HashMap::new()),
            leitor: Mutex::new(Leitor::default()),
            contas,
            limites: Mutex::new(HashMap::new()),
            _vigia: Mutex::new(None),
        });

        // Pede ao Linux para avisar quando os arquivos de estado mudarem.
        let (tx, rx) = mpsc::channel();
        if let Ok(mut vigia) = notify::recommended_watcher(tx) {
            for c in &m.contas {
                let dir = Path::new(&c.dir).join("sessions");
                if dir.is_dir() {
                    let _ = vigia.watch(&dir, RecursiveMode::NonRecursive);
                }
            }
            if let Ok(home) = std::env::var("HOME") {
                let codex = Path::new(&home).join(".codex/sessions");
                if codex.is_dir() {
                    let _ = vigia.watch(&codex, RecursiveMode::Recursive);
                }
            }
            *m._vigia.lock().unwrap() = Some(vigia);
        }
        let eu = m.clone();
        thread::spawn(move || {
            for evento in rx.into_iter().flatten() {
                for caminho in evento.paths {
                    eu.arquivo_mudou(&caminho);
                }
            }
        });

        // limites do Codex pela última sessão (até abrir um Codex aqui)
        let eu = m.clone();
        thread::spawn(move || {
            if let Some(l) = uso::limite_codex_recente() {
                eu.guardar_limite(l);
            }
        });
        m
    }

    pub fn registrar(&self, id: u32, tipo: &str, pid: u32, config: Option<PathBuf>) {
        self.abas.lock().unwrap().insert(
            id,
            Vigiada { tipo: tipo.into(), pid, config, trabalhando: false, arquivo: None },
        );
        self.emitir_info(id, None);
    }

    pub fn remover(&self, id: u32) {
        self.abas.lock().unwrap().remove(&id);
    }

    /// Thread do shell: a saída do terminal chega pelo canal e acorda a thread.
    /// `ultima_entrada` serve para não confundir o eco do que você digitou com trabalho.
    pub fn vigiar_shell(self: &Arc<Self>, id: u32, ultima_entrada: Arc<Mutex<Instant>>) -> SyncSender<()> {
        let (tx, rx): (SyncSender<()>, Receiver<()>) = mpsc::sync_channel(1);
        let eu = self.clone();
        thread::spawn(move || loop {
            // parado: dorme até sair alguma coisa
            if rx.recv().is_err() {
                return;
            }
            if ultima_entrada.lock().unwrap().elapsed() < Duration::from_millis(400) {
                continue;
            }
            eu.definir_trabalhando(id, true);
            // trabalhando: espera ficar 1,5 s em silêncio
            loop {
                match rx.recv_timeout(Duration::from_millis(1500)) {
                    Ok(()) => {}
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            eu.definir_trabalhando(id, false);
            // o comando terminou: pode ter mudado de pasta ou de branch
            eu.emitir_info(id, None);
        });
        tx
    }

    fn definir_trabalhando(&self, id: u32, valor: bool) -> bool {
        let mudou = match self.abas.lock().unwrap().get_mut(&id) {
            Some(a) if a.trabalhando != valor => {
                a.trabalhando = valor;
                true
            }
            _ => false,
        };
        if mudou {
            let _ = self.app.emit("estado", Estado { id, trabalhando: valor });
        }
        mudou
    }

    fn emitir_info(&self, id: u32, tokens: Option<Tokens>) {
        let Some(pid) = self.abas.lock().unwrap().get(&id).map(|a| a.pid) else { return };
        let pasta = std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
        let repo = pasta.as_deref().and_then(repositorio);
        let workspace = repo
            .as_ref()
            .map(|(dir, _)| dir.as_path())
            .or(pasta.as_deref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into());
        let _ = self.app.emit(
            "info",
            Info {
                id,
                pasta: pasta.map(|p| p.to_string_lossy().into()),
                workspace,
                branch: repo.and_then(|(_, b)| b),
                tokens,
            },
        );
    }

    fn arquivo_mudou(self: &Arc<Self>, caminho: &Path) {
        let texto = caminho.to_string_lossy();
        if texto.contains("/.codex/sessions/") {
            if texto.ends_with(".jsonl") {
                self.codex_mudou(caminho);
            }
        } else if texto.ends_with(".json") && caminho.parent().is_some_and(|d| d.ends_with("sessions")) {
            self.claude_mudou(caminho);
        }
    }

    /// <config>/sessions/<pid>.json mudou
    fn claude_mudou(self: &Arc<Self>, caminho: &Path) {
        let Some(pid) = caminho.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u32>().ok()) else {
            return;
        };
        let Some(config) = caminho.parent().and_then(Path::parent) else { return };
        let id = {
            let abas = self.abas.lock().unwrap();
            abas.iter()
                .find(|(_, a)| {
                    a.tipo == "claude"
                        && a.config.as_deref() == Some(config)
                        && (a.pid == pid || descendentes(a.pid).contains(&pid))
                })
                .map(|(id, _)| *id)
        };
        let Some(id) = id else { return };
        let Ok(txt) = std::fs::read_to_string(caminho) else { return };
        let Ok(v) = serde_json::from_str::<Value>(&txt) else { return };
        let ocupado = v.get("status").and_then(Value::as_str) == Some("busy");
        if self.definir_trabalhando(id, ocupado) && !ocupado {
            // terminou uma rodada: tokens atualizados e limites da conta
            let tokens = self.leitor.lock().unwrap().claude(config, &[pid]);
            self.emitir_info(id, tokens);
            self.pedir_limites(Some(config.to_string_lossy().into()));
        }
    }

    /// arquivo de sessão do Codex cresceu
    fn codex_mudou(self: &Arc<Self>, caminho: &Path) {
        let id = {
            let mut abas = self.abas.lock().unwrap();
            let conhecida = abas.iter().find(|(_, a)| a.arquivo.as_deref() == Some(caminho)).map(|(id, _)| *id);
            conhecida.or_else(|| {
                // arquivo novo: descobre qual Codex daqui é o dono. Primeiro por quem está
                // com ele aberto; se não der, pela pasta gravada na primeira linha do arquivo.
                let livres = || abas.iter().filter(|(_, a)| a.tipo == "codex" && a.arquivo.is_none());
                let dono = livres()
                    .find(|(_, a)| tem_aberto(a.pid, caminho))
                    .or_else(|| {
                        let pasta = pasta_da_sessao_codex(caminho)?;
                        let mut candidatas = livres().filter(|(_, a)| {
                            std::fs::read_link(format!("/proc/{}/cwd", a.pid)).is_ok_and(|p| p == pasta)
                        });
                        let unica = candidatas.next()?;
                        // com dois Codex na mesma pasta não dá para saber qual é
                        candidatas.next().is_none().then_some(unica)
                    })
                    .map(|(id, _)| *id)?;
                abas.get_mut(&dono)?.arquivo = Some(caminho.to_path_buf());
                Some(dono)
            })
        };
        let Some(id) = id else { return };
        let mut limite = None;
        let Some(tokens) = self.leitor.lock().unwrap().ler_codex(caminho, &mut limite) else { return };
        let ocupado = tokens.status.as_deref() == Some("busy");
        if self.definir_trabalhando(id, ocupado) && !ocupado {
            self.emitir_info(id, Some(tokens));
        }
        if let Some(l) = limite {
            self.guardar_limite(l);
        }
    }

    /// Atualiza os limites das contas pedidas (ou de todas) se já passou um minuto.
    pub fn pedir_limites(self: &Arc<Self>, chave: Option<String>) {
        for conta in self.contas.iter().filter(|c| chave.as_ref().is_none_or(|k| *k == c.dir)) {
            let velho = self
                .limites
                .lock()
                .unwrap()
                .get(&conta.dir)
                .is_none_or(|(quando, _)| quando.elapsed() > LIMITES_VALEM);
            if velho {
                let eu = self.clone();
                let conta = conta.clone();
                thread::spawn(move || eu.guardar_limite(uso::limite_claude(&conta)));
            }
        }
        self.emitir_limites();
    }

    fn guardar_limite(&self, mut l: Limite) {
        let mut limites = self.limites.lock().unwrap();
        // consulta falhou: fica com os últimos números que deram certo
        if l.erro.is_some() {
            if let Some((_, velho)) = limites.get(&l.chave).filter(|(_, v)| v.sessao.is_some() || v.semana.is_some()) {
                l = velho.clone();
            }
        }
        limites.insert(l.chave.clone(), (Instant::now(), l));
        drop(limites);
        self.emitir_limites();
    }

    fn emitir_limites(&self) {
        let lista: Vec<Limite> = self.limites.lock().unwrap().values().map(|(_, l)| l.clone()).collect();
        let _ = self.app.emit("limites", lista);
    }
}

/// Todos os processos abaixo de `pid` (filhos, netos...), pelo /proc.
fn descendentes(pid: u32) -> Vec<u32> {
    let mut saida = vec![];
    let mut pilha = vec![pid];
    while let Some(p) = pilha.pop() {
        for tarefa in std::fs::read_dir(format!("/proc/{p}/task")).into_iter().flatten().flatten() {
            let Ok(txt) = std::fs::read_to_string(tarefa.path().join("children")) else { continue };
            for filho in txt.split_whitespace().filter_map(|s| s.parse().ok()) {
                saida.push(filho);
                pilha.push(filho);
            }
        }
    }
    saida
}

/// O processo `pid` (ou algum filho dele) está com esse arquivo aberto?
fn tem_aberto(pid: u32, arquivo: &Path) -> bool {
    std::iter::once(pid).chain(descendentes(pid)).any(|p| {
        std::fs::read_dir(format!("/proc/{p}/fd"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|fd| std::fs::read_link(fd.path()).is_ok_and(|alvo| alvo == arquivo))
    })
}

/// Pasta onde a sessão do Codex foi aberta (primeira linha do arquivo, "session_meta").
fn pasta_da_sessao_codex(arquivo: &Path) -> Option<PathBuf> {
    use std::io::BufRead;
    let mut linha = String::new();
    std::io::BufReader::new(std::fs::File::open(arquivo).ok()?).read_line(&mut linha).ok()?;
    let v: Value = serde_json::from_str(&linha).ok()?;
    Some(PathBuf::from(v.pointer("/payload/cwd")?.as_str()?))
}

/// Sobe as pastas procurando o .git: devolve a pasta do repositório e a branch
/// (lendo o HEAD, sem chamar o comando git).
fn repositorio(pasta: &Path) -> Option<(PathBuf, Option<String>)> {
    for dir in pasta.ancestors() {
        let git = dir.join(".git");
        let head = if git.is_dir() {
            git.join("HEAD")
        } else if git.is_file() {
            // worktree: o .git é um arquivo "gitdir: caminho"
            let txt = std::fs::read_to_string(&git).ok()?;
            PathBuf::from(txt.strip_prefix("gitdir:")?.trim()).join("HEAD")
        } else {
            continue;
        };
        let branch = std::fs::read_to_string(head).ok().map(|txt| {
            let txt = txt.trim();
            match txt.strip_prefix("ref: refs/heads/") {
                Some(b) => b.to_string(),
                None => txt.chars().take(7).collect(),
            }
        });
        return Some((dir.to_path_buf(), branch));
    }
    None
}
