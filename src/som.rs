// Som de aviso quando o Claude termina uma tarefa.
// Ao abrir, o easynow põe um hook "Stop" no settings.json de cada conta do Claude que achar.
// O hook toca ~/.config/easynow/aviso.wav, a não ser que exista ~/.config/easynow/mudo
// (é isso que o botão de som da janela liga e desliga).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::uso::Conta;

const AVISO: &[u8] = include_bytes!("../assets/aviso.wav");

/// Tenta os players mais comuns do Linux; o primeiro que existir toca.
const COMANDO: &str = "f=\"$HOME/.config/easynow/aviso.wav\"; [ -e \"$HOME/.config/easynow/mudo\" ] || \
paplay --volume=36000 \"$f\" 2>/dev/null || pw-play --volume=0.55 \"$f\" 2>/dev/null || aplay -q \"$f\" 2>/dev/null; true";

fn pasta() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/easynow")
}

pub fn mudo() -> bool {
    pasta().join("mudo").exists()
}

pub fn mutar(mudo: bool) {
    let f = pasta().join("mudo");
    if mudo {
        let _ = std::fs::create_dir_all(pasta());
        let _ = std::fs::write(&f, "");
    } else {
        let _ = std::fs::remove_file(&f);
    }
}

pub fn instalar(contas: &[Conta]) {
    let _ = std::fs::create_dir_all(pasta());
    let wav = pasta().join("aviso.wav");
    if std::fs::read(&wav).ok().as_deref() != Some(AVISO) {
        let _ = std::fs::write(&wav, AVISO);
    }
    for c in contas.iter().filter(|c| c.tipo == "claude") {
        pôr_hook(&Path::new(&c.dir).join("settings.json"));
    }
}

/// É um hook nosso? (inclui o de antes, que tocava de ~/.claude-2/sounds)
fn nosso(grupo: &Value) -> bool {
    grupo["hooks"].as_array().is_some_and(|hs| {
        hs.iter().any(|h| {
            h["command"].as_str().is_some_and(|c| c.contains(".config/easynow/") || c.contains(".claude-2/sounds/"))
        })
    })
}

fn pôr_hook(arq: &Path) {
    let mut cfg: Value = match std::fs::read_to_string(arq) {
        Ok(t) => match serde_json::from_str(&t) {
            Ok(v) => v,
            Err(_) => return, // arquivo com erro: melhor não mexer
        },
        Err(_) => json!({}),
    };
    let Some(raiz) = cfg.as_object_mut() else { return };
    let hooks = raiz.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else { return };
    let stop = hooks.entry("Stop").or_insert_with(|| json!([]));
    let Some(stop) = stop.as_array_mut() else { return };

    let novo = json!({ "hooks": [{ "type": "command", "command": COMANDO, "async": true }] });
    let nossos: Vec<&Value> = stop.iter().filter(|g| nosso(g)).collect();
    if nossos.len() == 1 && *nossos[0] == novo {
        return; // já está certo
    }
    stop.retain(|g| !nosso(g));
    stop.push(novo);

    let Ok(texto) = serde_json::to_string_pretty(&cfg) else { return };
    // grava num temporário e troca, para nunca deixar o arquivo pela metade
    let tmp = arq.with_extension("json.easynow");
    if std::fs::write(&tmp, texto + "\n").is_ok() {
        let _ = std::fs::rename(&tmp, arq);
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn poe_hook_uma_vez_e_troca_o_antigo() {
        let dir = std::env::temp_dir().join(format!("easynow-som-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let arq = dir.join("settings.json");
        std::fs::write(&arq, r#"{"theme":"dark","hooks":{"Stop":[
            {"hooks":[{"type":"command","command":"echo outro"}]},
            {"hooks":[{"type":"command","command":"paplay ~/.claude-2/sounds/done.wav"}]}]}}"#).unwrap();
        pôr_hook(&arq);
        pôr_hook(&arq);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&arq).unwrap()).unwrap();
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "echo outro");
        assert_eq!(stop[1]["hooks"][0]["command"], COMANDO);
        assert_eq!(v["theme"], "dark");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
