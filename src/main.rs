// easynow: uma janela com abas para claude, codex e shell, e a telemetria de cada agente.
// O Rust cuida dos programas (PTY) e das medições; a interface fica em ui/.

mod pty;
mod telemetria;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, State};

use pty::Aba;
use telemetria::Telemetria;

struct Estado {
    abas: Mutex<HashMap<u32, Aba>>,
    pids: Arc<Mutex<Vec<u32>>>,
    telemetria: Arc<Mutex<Telemetria>>,
    iniciais: Vec<String>,
    pasta: PathBuf,
}

impl Estado {
    fn atualizar_pids(&self, abas: &HashMap<u32, Aba>) {
        *self.pids.lock().unwrap() = abas.values().filter_map(|a| a.pid).collect();
    }
}

#[derive(Serialize)]
struct Inicio {
    abas: Vec<String>,
    pasta: String,
    home: String,
}

#[tauri::command]
fn inicio(estado: State<Estado>) -> Inicio {
    Inicio {
        abas: estado.iniciais.clone(),
        pasta: estado.pasta.to_string_lossy().into(),
        home: std::env::var("HOME").unwrap_or_default(),
    }
}

/// Abre um programa novo. `pasta_de` = id da aba cuja pasta atual a nova aba deve usar.
#[tauri::command]
fn abrir(
    app: AppHandle,
    estado: State<Estado>,
    id: u32,
    tipo: String,
    pasta_de: Option<u32>,
    linhas: u16,
    colunas: u16,
) -> Result<String, String> {
    let mut abas = estado.abas.lock().unwrap();
    let pasta = pasta_de
        .and_then(|i| abas.get(&i))
        .and_then(|a| a.pid)
        .and_then(|p| std::fs::read_link(format!("/proc/{p}/cwd")).ok())
        .unwrap_or_else(|| estado.pasta.clone());
    let aba = Aba::abrir(app, id, &tipo, pasta.clone(), linhas, colunas).map_err(|e| e.to_string())?;
    abas.insert(id, aba);
    estado.atualizar_pids(&abas);
    Ok(pasta.to_string_lossy().into())
}

#[tauri::command]
fn escrever(estado: State<Estado>, id: u32, dados: String) {
    if let Some(a) = estado.abas.lock().unwrap().get_mut(&id) {
        a.escrever(dados.as_bytes());
    }
}

#[tauri::command]
fn redimensionar(estado: State<Estado>, id: u32, linhas: u16, colunas: u16) {
    if let Some(a) = estado.abas.lock().unwrap().get_mut(&id) {
        a.redimensionar(linhas, colunas);
    }
}

#[tauri::command]
fn fechar(estado: State<Estado>, id: u32) {
    let mut abas = estado.abas.lock().unwrap();
    abas.remove(&id);
    estado.atualizar_pids(&abas);
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
}

#[tauri::command]
fn status(estado: State<Estado>) -> Status {
    let t = estado.telemetria.lock().unwrap().clone();
    let agora = Instant::now();
    let mut abas = estado.abas.lock().unwrap();
    let lista = abas
        .iter_mut()
        .map(|(id, a)| {
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
                rodando: uso.rodando,
                cpu: uso.cpu,
                memoria: uso.memoria,
                processos: uso.processos,
            }
        })
        .collect();
    Status {
        cpu: t.cpu,
        memoria_usada: t.memoria_usada,
        memoria_total: t.memoria_total,
        abas: lista,
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

    let pids = Arc::new(Mutex::new(vec![]));
    let estado = Estado {
        abas: Mutex::new(HashMap::new()),
        telemetria: telemetria::iniciar(pids.clone()),
        pids,
        iniciais,
        pasta: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
    };

    tauri::Builder::default()
        .manage(estado)
        .invoke_handler(tauri::generate_handler![inicio, abrir, escrever, redimensionar, fechar, status])
        .run(tauri::generate_context!())
        .expect("erro ao abrir a janela");
}
