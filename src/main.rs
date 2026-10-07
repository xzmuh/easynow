// easynow: uma janela com abas para claude, codex e shell, e a telemetria de cada agente.
// O Rust cuida dos programas (PTY) e das medições; a interface fica em ui/.

mod pty;
mod telemetria;
mod uso;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, State};

use pty::Aba;
use telemetria::{Alvo, Telemetria};
use uso::{Conta, Limite, Tokens};

struct Estado {
    /// aba -> (programa rodando, o que a telemetria mede)
    abas: Mutex<HashMap<u32, (Aba, Alvo)>>,
    alvos: Arc<Mutex<Vec<Alvo>>>,
    telemetria: Arc<Mutex<Telemetria>>,
    limites: Arc<Mutex<Vec<Limite>>>,
    codex_ao_vivo: Arc<Mutex<Option<Limite>>>,
    contas: Vec<Conta>,
    iniciais: Vec<String>,
    pasta: PathBuf,
}

impl Estado {
    fn atualizar_alvos(&self, abas: &HashMap<u32, (Aba, Alvo)>) {
        *self.alvos.lock().unwrap() = abas.values().map(|(_, alvo)| alvo.clone()).collect();
    }
}

#[derive(Serialize)]
struct Inicio {
    abas: Vec<String>,
    pasta: String,
    home: String,
    contas: Vec<Conta>,
}

#[tauri::command]
fn inicio(estado: State<Estado>) -> Inicio {
    Inicio {
        abas: estado.iniciais.clone(),
        pasta: estado.pasta.to_string_lossy().into(),
        home: std::env::var("HOME").unwrap_or_default(),
        contas: estado.contas.clone(),
    }
}

/// Abre um programa novo. `pasta_de` = id da aba cuja pasta atual a nova aba deve usar;
/// `conta` = pasta de configuração do Claude (só para tipo "claude").
#[tauri::command]
fn abrir(
    app: AppHandle,
    estado: State<Estado>,
    id: u32,
    tipo: String,
    conta: Option<String>,
    pasta_de: Option<u32>,
    linhas: u16,
    colunas: u16,
) -> Result<String, String> {
    let mut abas = estado.abas.lock().unwrap();
    let pasta = pasta_de
        .and_then(|i| abas.get(&i))
        .and_then(|(a, _)| a.pid)
        .and_then(|p| std::fs::read_link(format!("/proc/{p}/cwd")).ok())
        .unwrap_or_else(|| estado.pasta.clone());
    let conta = if tipo == "claude" {
        conta
            .and_then(|dir| estado.contas.iter().find(|c| c.dir == dir))
            .or(estado.contas.first())
    } else {
        None
    };
    let env_conta = conta.filter(|c| !c.padrao).map(|c| c.dir.as_str());
    let aba = Aba::abrir(app, id, &tipo, env_conta, pasta.clone(), linhas, colunas).map_err(|e| e.to_string())?;
    let alvo = Alvo {
        pid: aba.pid.unwrap_or(0),
        tipo: tipo.clone(),
        config: conta.map(|c| PathBuf::from(&c.dir)),
    };
    abas.insert(id, (aba, alvo));
    estado.atualizar_alvos(&abas);
    Ok(pasta.to_string_lossy().into())
}

#[tauri::command]
fn escrever(estado: State<Estado>, id: u32, dados: String) {
    if let Some((a, _)) = estado.abas.lock().unwrap().get_mut(&id) {
        a.escrever(dados.as_bytes());
    }
}

#[tauri::command]
fn redimensionar(estado: State<Estado>, id: u32, linhas: u16, colunas: u16) {
    if let Some((a, _)) = estado.abas.lock().unwrap().get_mut(&id) {
        a.redimensionar(linhas, colunas);
    }
}

#[tauri::command]
fn fechar(estado: State<Estado>, id: u32) {
    let mut abas = estado.abas.lock().unwrap();
    abas.remove(&id);
    estado.atualizar_alvos(&abas);
}

#[derive(Serialize)]
struct StatusAba {
    id: u32,
    ms_saida: u64,
    ms_entrada: u64,
    bytes: u64,
    codigo_saida: Option<u32>,
    pasta: Option<String>,
    branch: Option<String>,
    workspace: Option<String>,
    tokens: Option<Tokens>,
    rodando: String,
    cpu: f32,
    memoria: u64,
    processos: usize,
}

#[derive(Serialize)]
struct Status {
    cpu: f32,
    memoria_usada: u64,
    memoria_total: u64,
    abas: Vec<StatusAba>,
    limites: Vec<Limite>,
}

#[tauri::command]
fn status(estado: State<Estado>) -> Status {
    let t = estado.telemetria.lock().unwrap().clone();
    let agora = Instant::now();
    let mut abas = estado.abas.lock().unwrap();
    let lista = abas
        .iter_mut()
        .map(|(id, (a, _))| {
            let codigo_saida = a.verificar_saida();
            let (ultima_saida, bytes) = {
                let at = a.atividade.lock().unwrap();
                (at.ultima_saida, at.bytes)
            };
            let uso = a.pid.and_then(|p| t.por_pid.get(&p).cloned()).unwrap_or_default();
            StatusAba {
                id: *id,
                ms_saida: agora.duration_since(ultima_saida).as_millis() as u64,
                ms_entrada: agora.duration_since(a.ultima_entrada).as_millis() as u64,
                bytes,
                codigo_saida,
                pasta: uso.pasta.map(|p| p.to_string_lossy().into()),
                branch: uso.branch,
                workspace: uso.workspace,
                tokens: uso.tokens,
                rodando: uso.rodando,
                cpu: uso.cpu,
                memoria: uso.memoria,
                processos: uso.processos,
            }
        })
        .collect();
    // limites das contas; o do Codex vem ao vivo se tiver um Codex aberto
    let mut limites = estado.limites.lock().unwrap().clone();
    if let Some(ao_vivo) = estado.codex_ao_vivo.lock().unwrap().clone() {
        limites.retain(|l| l.tipo != "codex");
        limites.push(ao_vivo);
    }
    Status {
        cpu: t.cpu,
        memoria_usada: t.memoria_usada,
        memoria_total: t.memoria_total,
        abas: lista,
        limites,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("uso: easynow [claude|codex|shell ...]\n\nsem nada abre um claude.\nex.: easynow claude claude codex shell");
        return;
    }
    let mut iniciais = vec![];
    for a in args.iter().filter(|a| *a != "--aqui") {
        match a.as_str() {
            "claude" | "codex" | "shell" => iniciais.push(a.clone()),
            outro => {
                eprintln!("não conheço \"{outro}\" (use claude, codex ou shell)");
                std::process::exit(1);
            }
        }
    }
    if iniciais.is_empty() {
        iniciais = vec!["claude".into()];
    }

    // Solta o terminal: a janela continua aberta mesmo se você fechar o terminal de onde rodou.
    if !args.iter().any(|a| a == "--aqui") {
        if let Ok(exe) = std::env::current_exe() {
            let ok = std::process::Command::new("setsid")
                .arg(exe)
                .args(&args)
                .arg("--aqui")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok();
            if ok {
                return;
            }
        }
    }

    // Evita janela em branco no Wayland com placa NVIDIA.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // seguro: ainda não existe nenhuma outra thread neste ponto
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }

    let alvos = Arc::new(Mutex::new(vec![]));
    let codex_ao_vivo = Arc::new(Mutex::new(None));
    let contas = uso::contas();
    let estado = Estado {
        abas: Mutex::new(HashMap::new()),
        telemetria: telemetria::iniciar(alvos.clone(), codex_ao_vivo.clone()),
        limites: uso::iniciar_limites(contas.clone(), codex_ao_vivo.clone()),
        alvos,
        codex_ao_vivo,
        contas,
        iniciais,
        pasta: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
    };

    tauri::Builder::default()
        .manage(estado)
        .invoke_handler(tauri::generate_handler![inicio, abrir, escrever, redimensionar, fechar, status])
        .run(tauri::generate_context!())
        .expect("erro ao abrir a janela");
}
