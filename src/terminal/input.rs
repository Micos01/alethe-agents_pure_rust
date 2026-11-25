//! egui key events -> xterm byte sequences.

use alacritty_terminal::term::TermMode;
use egui::{Key, Modifiers};

fn modifier_param(m: &Modifiers) -> u8 {
    1 + (m.shift as u8) + 2 * (m.alt as u8) + 4 * (m.ctrl as u8)
}

fn csi_with_mods(final_char: char, m: &Modifiers, app_cursor: bool) -> String {
    let param = modifier_param(m);
    if param > 1 {
        format!("\x1b[1;{param}{final_char}")
    } else if app_cursor {
        format!("\x1bO{final_char}")
    } else {
        format!("\x1b[{final_char}")
    }
}

fn tilde(code: u8, m: &Modifiers) -> String {
    let param = modifier_param(m);
    if param > 1 {
        format!("\x1b[{code};{param}~")
    } else {
        format!("\x1b[{code}~")
    }
}

fn letter_index(key: Key) -> Option<u8> {
    use Key::*;
    let keys = [
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    ];
    keys.iter().position(|k| *k == key).map(|i| i as u8)
}

/// Bytes for a key press, or `None` when the key produces text (handled by `Event::Text`).
pub fn key_to_bytes(key: Key, m: &Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let s: String = match key {
        Key::Enter if m.shift || m.alt => "\x1b\r".into(),
        Key::Enter => "\r".into(),
        Key::Tab if m.shift => "\x1b[Z".into(),
        Key::Tab => "\t".into(),
        Key::Backspace if m.ctrl => "\x17".into(),
        Key::Backspace if m.alt => "\x1b\x7f".into(),
        Key::Backspace => "\x7f".into(),
        Key::Escape => "\x1b".into(),
        Key::ArrowUp => csi_with_mods('A', m, app_cursor),
        Key::ArrowDown => csi_with_mods('B', m, app_cursor),
        Key::ArrowRight => csi_with_mods('C', m, app_cursor),
        Key::ArrowLeft => csi_with_mods('D', m, app_cursor),
        Key::Home => csi_with_mods('H', m, app_cursor),
        Key::End => csi_with_mods('F', m, app_cursor),
        Key::PageUp => tilde(5, m),
        Key::PageDown => tilde(6, m),
        Key::Insert => tilde(2, m),
        Key::Delete => tilde(3, m),
        Key::F1 => csi_with_mods('P', m, true),
        Key::F2 => csi_with_mods('Q', m, true),
        Key::F3 => csi_with_mods('R', m, true),
        Key::F4 => csi_with_mods('S', m, true),
        Key::F5 => tilde(15, m),
        Key::F6 => tilde(17, m),
        Key::F7 => tilde(18, m),
        Key::F8 => tilde(19, m),
        Key::F9 => tilde(20, m),
        Key::F10 => tilde(21, m),
        Key::F11 => tilde(23, m),
        Key::F12 => tilde(24, m),
        Key::Space if m.ctrl => "\0".into(),
        _ if m.ctrl && !m.alt => {
            if let Some(i) = letter_index(key) {
                return Some(vec![i + 1]);
            }
            match key {
                Key::OpenBracket => "\x1b".into(),
                Key::Backslash => "\x1c".into(),
                Key::CloseBracket => "\x1d".into(),
                Key::Num6 => "\x1e".into(),
                Key::Minus | Key::Slash => "\x1f".into(),
                _ => return None,
            }
        }
        _ if m.alt && !m.ctrl => {
            // Meta+letter: ESC prefix. The text event for it is suppressed by the caller.
            let i = letter_index(key)?;
            let c = if m.shift { b'A' + i } else { b'a' + i };
            return Some(vec![0x1b, c]);
        }
        _ => return None,
    };
    Some(s.into_bytes())
}

/// SGR / X10 mouse report for a button event at a 0-based cell.
pub fn mouse_report(
    mode: TermMode,
    button: u8,
    pressed: bool,
    col: usize,
    line: usize,
    m: &Modifiers,
) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    let mut code = button;
    if m.shift {
        code += 4;
    }
    if m.alt {
        code += 8;
    }
    if m.ctrl {
        code += 16;
    }
    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{};{}{suffix}", col + 1, line + 1).into_bytes());
    }
    let code = if pressed { code } else { 3 };
    if col > 222 || line > 222 {
        return None;
    }
    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code,
        33 + col as u8,
        33 + line as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_letters_become_control_codes() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(
            key_to_bytes(Key::C, &ctrl, TermMode::empty()),
            Some(vec![3])
        );
        assert_eq!(
            key_to_bytes(Key::D, &ctrl, TermMode::empty()),
            Some(vec![4])
        );
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        let none = Modifiers::default();
        assert_eq!(
            key_to_bytes(Key::ArrowUp, &none, TermMode::empty()).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            key_to_bytes(Key::ArrowUp, &none, TermMode::APP_CURSOR).unwrap(),
            b"\x1bOA"
        );
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            key_to_bytes(Key::ArrowUp, &shift, TermMode::APP_CURSOR).unwrap(),
            b"\x1b[1;2A"
        );
    }

    #[test]
    fn printable_keys_are_left_to_text_events() {
        assert_eq!(
            key_to_bytes(Key::A, &Modifiers::default(), TermMode::empty()),
            None
        );
    }
}
