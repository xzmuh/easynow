// easynow: uma janela com abas para claude, codex e shell.
// O Rust roda os programas (pty.rs) e acompanha o estado deles por eventos (monitor.rs);
// a interface fica em ui/.

mod monitor;
mod pty;
mod som;
mod uso;
mod voz;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use monitor::Monitor;
use pty::Aba;
use uso::Conta;

struct Estado {
    abas: Mutex<HashMap<u32, Aba>>,
    contas: Mutex<Vec<Conta>>,
    iniciais: Vec<String>,
    pasta: PathBuf,
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
        contas: estado.contas.lock().unwrap().clone(),
    }
}

/// Abre um programa novo. `pasta_de` = id da aba cuja pasta atual a nova aba deve usar;
/// `conta` = pasta de configuração da conta do Claude ou do Codex.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn abrir(
    app: AppHandle,
    estado: State<Estado>,
    monitor: State<Arc<Monitor>>,
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
        .and_then(|a| a.pid)
        .and_then(|p| std::fs::read_link(format!("/proc/{p}/cwd")).ok())
        .unwrap_or_else(|| estado.pasta.clone());
    let contas = estado.contas.lock().unwrap().clone();
    let conta = if tipo == "shell" {
        None
    } else {
        match conta {
            // pasta de uma conta nova (aba de login), que ainda não está na lista
            Some(dir) if PathBuf::from(&dir) == uso::pasta_conta_nova(&tipo) => {
                // o Codex não cria a própria pasta; e a conta nova já começa com a configuração da padrão
                if tipo == "codex" {
                    let _ = std::fs::create_dir_all(&dir);
                    let destino = Path::new(&dir).join("config.toml");
                    if !destino.exists() {
                        let _ = std::fs::copy(uso::pasta_padrao("codex").join("config.toml"), destino);
                    }
                }
                let c = Conta { tipo: tipo.clone(), nome: "Nova conta".into(), dir, padrao: false };
                monitor.vigiar_conta(&c);
                Some(c)
            }
            c => {
                let do_tipo = || contas.iter().filter(|c| c.tipo == tipo);
                c.and_then(|dir| do_tipo().find(|c| c.dir == dir)).or(do_tipo().next()).cloned()
            }
        }
    };
    let env_conta = conta.as_ref().filter(|c| !c.padrao).map(|c| c.dir.as_str());
    // sem conta escolhida, o programa usa a pasta padrão
    let config = (tipo != "shell")
        .then(|| conta.as_ref().map_or_else(|| uso::pasta_padrao(&tipo), |c| PathBuf::from(&c.dir)));

    let ultima_entrada = Arc::new(Mutex::new(Instant::now()));
    // só o shell precisa ser acordado pela saída; Claude e Codex avisam por arquivo
    let acordar = (tipo == "shell").then(|| monitor.vigiar_shell(id, ultima_entrada.clone()));
    let aba = Aba::abrir(app, id, &tipo, env_conta, pasta.clone(), linhas, colunas, ultima_entrada, acordar)
        .map_err(|e| e.to_string())?;
    monitor.registrar(id, &tipo, aba.pid.unwrap_or(0), config);
    abas.insert(id, aba);
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
fn fechar(estado: State<Estado>, monitor: State<Arc<Monitor>>, id: u32) {
    estado.abas.lock().unwrap().remove(&id);
    monitor.remover(id);
}

/// Depois do evento "fim": com que código o programa saiu.
#[tauri::command]
fn codigo_saida(estado: State<Estado>, id: u32) -> Option<u32> {
    estado.abas.lock().unwrap().get_mut(&id)?.codigo_saida()
}

/// A janela pede ao trocar de aba; só busca de novo se passou um minuto.
#[tauri::command]
fn pedir_limites(monitor: State<Arc<Monitor>>, chave: Option<String>) {
    monitor.pedir_limites(chave);
}

/// Segurou espaço numa aba do Claude: liga a medição de volume para o orb.
#[tauri::command]
fn voz_ouvir(app: AppHandle, mic: State<voz::Microfone>) {
    mic.iniciar(app);
}

#[tauri::command]
fn voz_parar(mic: State<voz::Microfone>) {
    mic.parar();
}

#[tauri::command]
fn voz_ligada(dir: String) -> bool {
    voz::ligada(&dir)
}

/// Pasta onde a aba "Adicionar conta" vai fazer o login (tipo "claude" ou "codex").
#[tauri::command]
fn conta_nova(tipo: String) -> String {
    uso::pasta_conta_nova(&tipo).to_string_lossy().into()
}

/// Procura contas de novo (depois de um login) e já liga o som e o monitor nas novas.
#[tauri::command]
fn recarregar_contas(estado: State<Estado>, monitor: State<Arc<Monitor>>) -> Vec<Conta> {
    let mut contas = estado.contas.lock().unwrap();
    for c in uso::contas() {
        if !contas.iter().any(|x| x.dir == c.dir) {
            som::instalar(std::slice::from_ref(&c));
            monitor.adicionar_conta(c.clone());
            contas.push(c);
        }
    }
    contas.clone()
}

#[tauri::command]
fn som_mudo() -> bool {
    som::mudo()
}

#[tauri::command]
fn som_mutar(mudo: bool) {
    som::mutar(mudo);
}

/// Abre um link do terminal no navegador padrão.
#[tauri::command]
fn abrir_link(url: String) {
    if url.starts_with("http://") || url.starts_with("https://") {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
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

    let contas = uso::contas();
    som::instalar(&contas);
    let estado = Estado {
        abas: Mutex::new(HashMap::new()),
        contas: Mutex::new(contas.clone()),
        iniciais,
        pasta: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
    };

    tauri::Builder::default()
        .manage(estado)
        .manage(voz::Microfone::default())
        .setup(move |app| {
            app.manage(Monitor::iniciar(app.handle().clone(), contas.clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            inicio,
            abrir,
            escrever,
            redimensionar,
            fechar,
            codigo_saida,
            pedir_limites,
            voz_ouvir,
            voz_parar,
            voz_ligada,
            abrir_link,
            som_mudo,
            conta_nova,
            recarregar_contas,
            som_mutar
        ])
        .run(tauri::generate_context!())
        .expect("erro ao abrir a janela");
}
