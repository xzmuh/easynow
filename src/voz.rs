// Firula da voz: quem ouve e transcreve é o /voice do próprio Claude (segurando espaço).
// Aqui só medimos o volume do microfone enquanto você fala, para o orb da janela reagir.
// Nada é gravado: o áudio é lido em pedaços de 50 ms e só o volume vai para a janela.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;

use tauri::{AppHandle, Emitter};

#[derive(Default)]
pub struct Microfone {
    gravador: Mutex<Option<Child>>,
}

impl Microfone {
    pub fn iniciar(&self, app: AppHandle) {
        let mut g = self.gravador.lock().unwrap();
        if g.is_some() {
            return;
        }
        // pw-record (PipeWire) deixa o Claude e o easynow usarem o microfone ao mesmo tempo
        let Ok(mut filho) = Command::new("pw-record")
            .args(["--rate", "16000", "--channels", "1", "--format", "s16", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        let Some(mut saida) = filho.stdout.take() else { return };
        *g = Some(filho);
        thread::spawn(move || {
            let mut buf = [0u8; 1600]; // 800 amostras = 50 ms
            while saida.read_exact(&mut buf).is_ok() {
                let soma: f64 = buf
                    .chunks_exact(2)
                    .map(|b| {
                        let a = i16::from_le_bytes([b[0], b[1]]) as f64;
                        a * a
                    })
                    .sum();
                let rms = (soma / 800.0).sqrt();
                // escala "de ouvido": fala normal fica no meio, grito chega em 1
                let nivel = ((rms / 32768.0).max(1e-5).log10() + 3.5) / 2.5;
                let _ = app.emit("nivel", nivel.clamp(0.0, 1.0));
            }
        });
    }

    pub fn parar(&self) {
        if let Some(mut filho) = self.gravador.lock().unwrap().take() {
            let _ = filho.kill();
            let _ = filho.wait();
        }
    }
}

/// O /voice está ligado nessa conta do Claude? (settings.json -> voice.enabled)
pub fn ligada(dir: &str) -> bool {
    // Só fica de fora quem desligou o /voice de propósito. Sem nada no settings.json
    // (o Claude nem sempre grava lá) o orb aparece ao segurar o espaço.
    let cfg = std::fs::read_to_string(Path::new(dir).join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let Some(cfg) = cfg else { return true };
    // formato novo ("voice": { "enabled": ... }) e o antigo ("voiceEnabled")
    cfg.pointer("/voice/enabled")
        .or_else(|| cfg.get("voiceEnabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}
