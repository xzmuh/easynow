// Uma thread que a cada segundo e meio mede CPU/RAM do sistema e de cada aba
// (somando o programa e todos os processos filhos dele), pasta, workspace, branch e tokens.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::uso::{Leitor, Limite, Tokens};

/// O que medir: o processo da aba, o tipo e (no Claude) a pasta de configuração da conta.
#[derive(Clone)]
pub struct Alvo {
    pub pid: u32,
    pub tipo: String,
    pub config: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub struct Uso {
    pub cpu: f32,
    pub memoria: u64,
    pub processos: usize,
    /// O processo "mais de baixo" da árvore: no shell é o comando rodando agora.
    pub rodando: String,
    pub pasta: Option<PathBuf>,
    pub branch: Option<String>,
    /// nome do projeto: a pasta do repositório git, ou a pasta atual
    pub workspace: Option<String>,
    pub tokens: Option<Tokens>,
}

#[derive(Clone, Default)]
pub struct Telemetria {
    pub cpu: f32,
    pub memoria_usada: u64,
    pub memoria_total: u64,
    pub por_pid: HashMap<u32, Uso>,
}

pub fn iniciar(alvos: Arc<Mutex<Vec<Alvo>>>, codex_ao_vivo: Arc<Mutex<Option<Limite>>>) -> Arc<Mutex<Telemetria>> {
    let saida = Arc::new(Mutex::new(Telemetria::default()));
    let destino = saida.clone();
    thread::spawn(move || {
        let mut sys = System::new();
        let mut leitor = Leitor::default();
        loop {
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            sys.refresh_processes(ProcessesToUpdate::All, true);

            let mut filhos: HashMap<Pid, Vec<Pid>> = HashMap::new();
            for (pid, p) in sys.processes() {
                if let Some(pai) = p.parent() {
                    filhos.entry(pai).or_default().push(*pid);
                }
            }

            let mut por_pid = HashMap::new();
            for alvo in alvos.lock().unwrap().clone() {
                let (mut uso, arvore) = medir(&sys, &filhos, Pid::from_u32(alvo.pid));
                uso.tokens = match (alvo.tipo.as_str(), &alvo.config) {
                    ("claude", Some(config)) => leitor.claude(config, &arvore),
                    ("codex", _) => {
                        let mut lim = None;
                        let t = leitor.codex(&arvore, &mut lim);
                        if lim.is_some() {
                            *codex_ao_vivo.lock().unwrap() = lim;
                        }
                        t
                    }
                    _ => None,
                };
                por_pid.insert(alvo.pid, uso);
            }

            *destino.lock().unwrap() = Telemetria {
                cpu: sys.global_cpu_usage(),
                memoria_usada: sys.used_memory(),
                memoria_total: sys.total_memory(),
                por_pid,
            };
            thread::sleep(Duration::from_millis(1500));
        }
    });
    saida
}

/// Mede a árvore de processos a partir de `raiz`; devolve também os pids da árvore.
fn medir(sys: &System, filhos: &HashMap<Pid, Vec<Pid>>, raiz: Pid) -> (Uso, Vec<u32>) {
    let mut uso = Uso::default();
    let mut arvore = vec![];
    let mut pilha = vec![(raiz, 0usize)];
    let mut mais_fundo = 0;
    while let Some((pid, nivel)) = pilha.pop() {
        let Some(p) = sys.process(pid) else { continue };
        // no Linux as threads também aparecem como processos; pula elas
        if p.thread_kind().is_some() {
            continue;
        }
        uso.cpu += p.cpu_usage();
        uso.memoria += p.memory();
        uso.processos += 1;
        arvore.push(pid.as_u32());
        if nivel >= mais_fundo {
            mais_fundo = nivel;
            uso.rodando = p.name().to_string_lossy().to_string();
        }
        for f in filhos.get(&pid).into_iter().flatten() {
            pilha.push((*f, nivel + 1));
        }
    }
    uso.pasta = std::fs::read_link(format!("/proc/{raiz}/cwd")).ok();
    if let Some(pasta) = uso.pasta.as_deref() {
        let repo = repositorio(pasta);
        uso.workspace = repo
            .as_ref()
            .map(|(dir, _)| dir.as_path())
            .unwrap_or(pasta)
            .file_name()
            .map(|n| n.to_string_lossy().into());
        uso.branch = repo.and_then(|(_, b)| b);
    }
    (uso, arvore)
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
