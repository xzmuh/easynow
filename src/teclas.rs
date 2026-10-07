// Converte uma tecla (como o crossterm entrega) nos bytes que um terminal de
// verdade mandaria para o programa. É isso que o claude/codex "lê" quando você digita.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn para_bytes(tecla: &KeyEvent, cursor_de_aplicacao: bool) -> Vec<u8> {
    let m = tecla.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);
    let shift = m.contains(KeyModifiers::SHIFT);

    // Parâmetro de modificador do xterm: 1 + shift(1) + alt(2) + ctrl(4)
    let modif = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;

    let seta = |letra: char| -> Vec<u8> {
        if modif > 1 {
            format!("\x1b[1;{modif}{letra}").into_bytes()
        } else if cursor_de_aplicacao {
            format!("\x1bO{letra}").into_bytes()
        } else {
            format!("\x1b[{letra}").into_bytes()
        }
    };
    let til = |n: u8| -> Vec<u8> {
        if modif > 1 {
            format!("\x1b[{n};{modif}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    let mut bytes = match tecla.code {
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            c @ 'a'..='z' => vec![c as u8 - b'a' + 1],
            ' ' | '@' | '2' => vec![0],
            '[' | '3' => vec![0x1b],
            '\\' | '4' => vec![0x1c],
            ']' | '5' => vec![0x1d],
            '^' | '6' => vec![0x1e],
            '_' | '-' | '7' => vec![0x1f],
            c => c.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        // Shift+Enter: o claude entende ESC+Enter como "nova linha sem enviar"
        KeyCode::Enter if shift => return b"\x1b\r".to_vec(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace if ctrl => vec![0x08],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => return b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => return seta('A'),
        KeyCode::Down => return seta('B'),
        KeyCode::Right => return seta('C'),
        KeyCode::Left => return seta('D'),
        KeyCode::Home => return seta('H'),
        KeyCode::End => return seta('F'),
        KeyCode::Insert => return til(2),
        KeyCode::Delete => return til(3),
        KeyCode::PageUp => return til(5),
        KeyCode::PageDown => return til(6),
        KeyCode::F(n) => {
            return match n {
                1 => b"\x1bOP".to_vec(),
                2 => b"\x1bOQ".to_vec(),
                3 => b"\x1bOR".to_vec(),
                4 => b"\x1bOS".to_vec(),
                5 => til(15),
                6 => til(17),
                7 => til(18),
                8 => til(19),
                9 => til(20),
                10 => til(21),
                11 => til(23),
                12 => til(24),
                _ => vec![],
            }
        }
        _ => vec![],
    };

    // Alt+tecla = ESC na frente
    if alt && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}
