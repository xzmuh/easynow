// Gasto de tokens de cada aba e limites de uso (sessão de 5h e semana) de cada conta.
//
// - Claude: <config>/sessions/<pid>.json diz qual sessão aquele processo está usando
//   (e se está "busy"); o histórico fica em <config>/projects/<pasta>/<sessão>.jsonl.
//   Os limites vêm do mesmo endereço que o /usage do Claude Code consulta.
// - Codex: o arquivo da sessão (~/.codex/sessions/...jsonl) fica aberto pelo processo;
//   cada evento "token_count" traz os tokens e os limites.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

// ---------- contas do Claude ----------

#[derive(Clone, Serialize)]
pub struct Conta {
    pub nome: String,
    /// pasta de configuração (~/.claude, ~/.claude-2...)
    pub dir: String,
    /// a padrão roda sem CLAUDE_CONFIG_DIR
    pub padrao: bool,
}

/// Acha as contas: ~/.claude (padrão) e cada ~/.claude-* que tenha login salvo.
pub fn contas() -> Vec<Conta> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let mut lista = vec![];
    let padrao = home.join(".claude");
    if padrao.join(".credentials.json").exists() {
        lista.push(Conta {
            nome: nome_da_conta(&home.join(".claude.json")).unwrap_or_else(|| "Claude".into()),
            dir: padrao.to_string_lossy().into(),
            padrao: true,
        });
    }
    let mut outras: Vec<PathBuf> = std::fs::read_dir(&home)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(".claude-"))
                && p.join(".credentials.json").exists()
        })
        .collect();
    outras.sort();
    for dir in outras {
        let nome = nome_da_conta(&dir.join(".claude.json")).unwrap_or_else(|| {
            dir.file_name().unwrap().to_string_lossy().trim_start_matches('.').to_string()
        });
        lista.push(Conta { nome, dir: dir.to_string_lossy().into(), padrao: false });
    }
    lista
}

/// Pasta para uma conta nova: a primeira ~/.claude-N ainda sem login.
pub fn pasta_conta_nova() -> PathBuf {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    (2..)
        .map(|n| home.join(format!(".claude-{n}")))
        .find(|p| !p.join(".credentials.json").exists())
        .unwrap()
}

/// "Next SI" (nome da organização) ou, se for o nome automático, o começo do e-mail ("Nuveto").
fn nome_da_conta(arquivo: &Path) -> Option<String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(arquivo).ok()?).ok()?;
    let conta = v.get("oauthAccount")?;
    let org = conta.get("organizationName").and_then(Value::as_str).unwrap_or("");
    if !org.is_empty() && !org.contains("'s Organization") {
        return Some(org.to_string());
    }
    let email = conta.get("emailAddress")?.as_str()?;
    let primeiro = email.split(['.', '@', '_', '-']).next()?;
    let mut c = primeiro.chars();
    Some(c.next()?.to_uppercase().chain(c).collect())
}

// ---------- tokens por aba ----------

#[derive(Clone, Default, Serialize)]
pub struct Tokens {
    pub entrada: u64,
    pub saida: u64,
    pub cache_escrita: u64,
    pub cache_leitura: u64,
    /// tamanho da conversa na última resposta
    pub contexto: u64,
    pub modelo: String,
    /// só Claude: "busy" / "idle"
    pub status: Option<String>,
}

#[derive(Default)]
struct Leitura {
    offset: u64,
    sobra: String,
    vistos: HashSet<String>,
    t: Tokens,
}

#[derive(Default)]
pub struct Leitor {
    arquivos: HashMap<PathBuf, Leitura>,
}

impl Leitor {
    /// Tokens da sessão do Claude rodando em algum desses pids.
    pub fn claude(&mut self, config: &Path, pids: &[u32]) -> Option<Tokens> {
        for pid in pids {
            let Ok(txt) = std::fs::read_to_string(config.join("sessions").join(format!("{pid}.json")))
            else {
                continue;
            };
            let v: Value = serde_json::from_str(&txt).ok()?;
            let sessao = v.get("sessionId")?.as_str()?;
            let cwd = v.get("cwd")?.as_str()?;
            let arquivo = config.join("projects").join(pasta_do_projeto(cwd)).join(format!("{sessao}.jsonl"));
            let mut t = self.ler(&arquivo, linha_claude, &mut None).unwrap_or_default();
            t.status = v.get("status").and_then(Value::as_str).map(String::from);
            return Some(t);
        }
        None
    }

    /// Tokens (e estado) da sessão do Codex nesse arquivo.
    pub fn ler_codex(&mut self, arquivo: &Path, limites: &mut Option<Limite>) -> Option<Tokens> {
        self.ler(arquivo, linha_codex, limites)
    }

    /// Lê só o que foi acrescentado no arquivo desde a última vez.
    fn ler(
        &mut self,
        arquivo: &Path,
        linha: fn(&Value, &mut Leitura, &mut Option<Limite>),
        limites: &mut Option<Limite>,
    ) -> Option<Tokens> {
        let mut f = std::fs::File::open(arquivo).ok()?;
        let tam = f.metadata().ok()?.len();
        let l = self.arquivos.entry(arquivo.to_path_buf()).or_default();
        if tam < l.offset {
            *l = Leitura::default();
        }
        f.seek(SeekFrom::Start(l.offset)).ok()?;
        let mut novo = String::new();
        f.read_to_string(&mut novo).ok()?;
        l.offset = tam;
        let texto = std::mem::take(&mut l.sobra) + &novo;
        let mut partes: Vec<&str> = texto.split('\n').collect();
        l.sobra = partes.pop().unwrap_or("").to_string();
        for p in partes {
            if let Ok(v) = serde_json::from_str::<Value>(p) {
                linha(&v, l, limites);
            }
        }
        Some(l.t.clone())
    }
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn linha_claude(v: &Value, l: &mut Leitura, _: &mut Option<Limite>) {
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let Some(msg) = v.get("message") else { return };
    let Some(u) = msg.get("usage") else { return };
    // a mesma resposta aparece em várias linhas (uma por bloco); conta uma vez só
    if let Some(id) = msg.get("id").and_then(Value::as_str) {
        if !l.vistos.insert(id.to_string()) {
            return;
        }
    }
    let t = &mut l.t;
    t.entrada += n(u, "input_tokens");
    t.saida += n(u, "output_tokens");
    t.cache_escrita += n(u, "cache_creation_input_tokens");
    t.cache_leitura += n(u, "cache_read_input_tokens");
    t.contexto = n(u, "input_tokens") + n(u, "cache_creation_input_tokens") + n(u, "cache_read_input_tokens");
    if let Some(m) = msg.get("model").and_then(Value::as_str) {
        if !m.starts_with('<') {
            t.modelo = m.to_string();
        }
    }
}

fn linha_codex(v: &Value, l: &mut Leitura, limites: &mut Option<Limite>) {
    let Some(p) = v.get("payload") else { return };
    match (v.get("type").and_then(Value::as_str), p.get("type").and_then(Value::as_str)) {
        (Some("event_msg"), Some("task_started")) => l.t.status = Some("busy".into()),
        (Some("event_msg"), Some("task_complete" | "turn_aborted")) => l.t.status = Some("idle".into()),
        (Some("turn_context"), _) => {
            if let Some(m) = p.get("model").and_then(Value::as_str) {
                l.t.modelo = m.to_string();
            }
        }
        (Some("event_msg"), Some("token_count")) => {
            if let Some(total) = p.pointer("/info/total_token_usage") {
                let cache = n(total, "cached_input_tokens");
                l.t.entrada = n(total, "input_tokens").saturating_sub(cache);
                l.t.cache_leitura = cache;
                l.t.saida = n(total, "output_tokens");
            }
            if let Some(ultimo) = p.pointer("/info/last_token_usage") {
                l.t.contexto = n(ultimo, "input_tokens");
            }
            if let Some(r) = p.get("rate_limits") {
                *limites = Some(limite_codex(r));
            }
        }
        _ => {}
    }
}

// ---------- limites ----------

#[derive(Clone, Serialize)]
pub struct Janela {
    pub pct: f64,
    /// quando zera: texto ISO (Claude) ou segundos desde 1970 (Codex)
    pub zera: Value,
}

#[derive(Clone, Serialize)]
pub struct Limite {
    /// pasta da conta do Claude, ou "codex"
    pub chave: String,
    pub nome: String,
    pub tipo: String,
    pub sessao: Option<Janela>,
    pub semana: Option<Janela>,
    pub erro: Option<String>,
}

fn limite_codex(r: &Value) -> Limite {
    let janela = |k: &str| {
        r.get(k).filter(|j| !j.is_null()).map(|j| Janela {
            pct: j.get("used_percent").and_then(Value::as_f64).unwrap_or(0.0),
            zera: j.get("resets_at").cloned().unwrap_or(Value::Null),
        })
    };
    Limite {
        chave: "codex".into(),
        nome: "Codex".into(),
        tipo: "codex".into(),
        sessao: janela("primary"),
        semana: janela("secondary"),
        erro: None,
    }
}

pub fn limite_claude(conta: &Conta) -> Limite {
    let mut lim = Limite { chave: conta.dir.clone(), nome: conta.nome.clone(), tipo: "claude".into(), sessao: None, semana: None, erro: None };
    let token = std::fs::read_to_string(Path::new(&conta.dir).join(".credentials.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.pointer("/claudeAiOauth/accessToken")?.as_str().map(String::from));
    let Some(token) = token else {
        lim.erro = Some("sem login".into());
        return lim;
    };
    let resp = ureq::get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .call()
        .and_then(|mut r| r.body_mut().read_json::<Value>());
    match resp {
        Ok(v) => {
            let janela = |k: &str| {
                v.get(k).filter(|j| !j.is_null()).map(|j| Janela {
                    pct: j.get("utilization").and_then(Value::as_f64).unwrap_or(0.0),
                    zera: j.get("resets_at").cloned().unwrap_or(Value::Null),
                })
            };
            lim.sessao = janela("five_hour");
            lim.semana = janela("seven_day");
        }
        // o login expira quando o Claude não está aberto; ele renova sozinho ao abrir
        Err(ureq::Error::StatusCode(401)) => lim.erro = Some("abra o Claude dessa conta para atualizar".into()),
        // esse endereço tem cota baixa e o próprio Claude também consulta; tenta de novo depois
        Err(ureq::Error::StatusCode(429)) => lim.erro = Some("consultando de novo em instantes".into()),
        Err(_) => lim.erro = Some("sem conexão para ver os limites".into()),
    }
    lim
}

/// Limites do Codex pelo arquivo de sessão mais recente (quando não tem Codex aberto).
pub fn limite_codex_recente() -> Option<Limite> {
    let raiz = PathBuf::from(std::env::var("HOME").ok()?).join(".codex/sessions");
    let mut arquivos = vec![];
    let mut pastas = vec![raiz];
    while let Some(p) = pastas.pop() {
        for e in std::fs::read_dir(&p).into_iter().flatten().flatten() {
            let caminho = e.path();
            if caminho.is_dir() {
                pastas.push(caminho);
            } else if caminho.extension().is_some_and(|x| x == "jsonl") {
                if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                    arquivos.push((m, caminho));
                }
            }
        }
    }
    let (_, mais_novo) = arquivos.into_iter().max_by_key(|(m, _)| *m)?;
    let texto = std::fs::read_to_string(mais_novo).ok()?;
    texto.lines().rev().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        v.pointer("/payload/rate_limits").filter(|r| !r.is_null()).map(limite_codex)
    })
}

/// Nome da pasta do projeto como o Claude grava: tudo que não é letra/número vira "-".
fn pasta_do_projeto(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}
