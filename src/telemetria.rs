// Uma thread que a cada segundo e meio mede CPU/RAM do sistema e de cada aba
// (somando o programa e todos os processos filhos dele), pasta atual e branch do git.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Clone, Default)]
pub struct Uso {
    pub cpu: f32,
    pub memoria: u64,
    pub processos: usize,
    /// O processo "mais de baixo" da árvore: no shell é o comando rodando agora.
    pub rodando: String,
    pub pasta: Option<PathBuf>,
    pub branch: Option<String>,
}

#[derive(Clone, Default)]
pub struct Telemetria {
    pub cpu: f32,
    pub memoria_usada: u64,
    pub memoria_total: u64,
    pub por_pid: HashMap<u32, Uso>,
}

pub fn iniciar(pids: Arc<Mutex<Vec<u32>>>) -> Arc<Mutex<Telemetria>> {
    let saida = Arc::new(Mutex::new(Telemetria::default()));
    let destino = saida.clone();
    thread::spawn(move || {
        let mut sys = System::new();
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
            for pid in pids.lock().unwrap().clone() {
                por_pid.insert(pid, medir(&sys, &filhos, Pid::from_u32(pid)));
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

fn medir(sys: &System, filhos: &HashMap<Pid, Vec<Pid>>, raiz: Pid) -> Uso {
    let mut uso = Uso::default();
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
        if nivel >= mais_fundo {
            mais_fundo = nivel;
            uso.rodando = p.name().to_string_lossy().to_string();
        }
        for f in filhos.get(&pid).into_iter().flatten() {
            pilha.push((*f, nivel + 1));
        }
    }
    uso.pasta = std::fs::read_link(format!("/proc/{raiz}/cwd")).ok();
    uso.branch = uso.pasta.as_deref().and_then(branch_git);
    uso
}

/// Sobe as pastas procurando o .git e lê o HEAD (sem chamar o comando git).
fn branch_git(pasta: &Path) -> Option<String> {
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
        let txt = std::fs::read_to_string(head).ok()?;
        let txt = txt.trim();
        return Some(match txt.strip_prefix("ref: refs/heads/") {
            Some(b) => b.to_string(),
            None => txt.chars().take(7).collect(),
        });
    }
    None
}
