//! Port of `agentCompletionMonitor.ts`: tells when an agent finished answering a prompt.
//!
//! A line submitted from the keyboard arms the monitor; real output after it (not the echo of the
//! prompt) marks the agent as working; once the output has been quiet for a while the answer is
//! considered complete, which is when the app notifies.

use std::time::{Duration, Instant};

const RESPONSE_IDLE: Duration = Duration::from_millis(4500);
const MIN_RESPONSE: Duration = Duration::from_millis(700);
const ECHO_GRACE: Duration = Duration::from_millis(350);
const MIN_OUTPUT_CHARS_AFTER_ECHO: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Idle,
    Armed,
    Working,
}

pub struct CompletionMonitor {
    state: State,
    line: String,
    prompt: String,
    submitted_at: Instant,
    output_chars: usize,
}

impl Default for CompletionMonitor {
    fn default() -> Self {
        Self {
            state: State::Idle,
            line: String::new(),
            prompt: String::new(),
            submitted_at: Instant::now(),
            output_chars: 0,
        }
    }
}

impl CompletionMonitor {
    /// Keystrokes and pastes sent to the PTY.
    pub fn handle_input(&mut self, bytes: &[u8]) {
        for ch in String::from_utf8_lossy(bytes).chars() {
            match ch {
                '\r' | '\n' => {
                    let prompt = std::mem::take(&mut self.line).trim().to_string();
                    if !prompt.is_empty() {
                        self.arm(prompt);
                    }
                }
                '\u{8}' | '\u{7f}' => {
                    self.line.pop();
                }
                c if c >= ' ' => self.line.push(c),
                _ => {}
            }
        }
    }

    /// Bytes read from the PTY.
    pub fn handle_output(&mut self, chunk: &[u8]) {
        if self.state == State::Idle {
            return;
        }
        let text = strip_terminal_controls(chunk);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if self.submitted_at.elapsed() <= ECHO_GRACE
            && !self.prompt.is_empty()
            && text.contains(self.prompt.as_str())
        {
            return;
        }
        self.output_chars += text.chars().count();
        if self.state == State::Armed && self.output_chars >= MIN_OUTPUT_CHARS_AFTER_ECHO {
            self.state = State::Working;
        }
    }

    /// True once, when a working agent has been quiet long enough to call its answer done.
    pub fn poll(&mut self, since_output: Duration) -> bool {
        if self.state != State::Working
            || since_output < RESPONSE_IDLE
            || self.submitted_at.elapsed() < MIN_RESPONSE
        {
            return false;
        }
        self.state = State::Idle;
        true
    }

    /// Whether a completion may still come, so the app keeps polling.
    pub fn pending(&self) -> bool {
        self.state != State::Idle
    }

    fn arm(&mut self, prompt: String) {
        self.state = State::Armed;
        self.prompt = prompt;
        self.submitted_at = Instant::now();
        self.output_chars = 0;
    }
}

/// Drops CSI/OSC/DCS sequences and C0 controls (tab and newlines stay), like
/// `stripTerminalControls`.
fn strip_terminal_controls(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                // CSI: parameters and intermediates, then one final byte in @..~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC, DCS, SOS, PM, APC: until BEL or ST (ESC \).
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if c == '\t' || c == '\n' || c == '\r' || !(c < ' ' || c == '\u{7f}') {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sequences_and_controls() {
        let raw = b"\x1b[31mred\x1b[0m \x1b]0;title\x07ok\x1bP1$r\x1b\\\x08!";
        assert_eq!(strip_terminal_controls(raw), "red ok!");
    }

    #[test]
    fn echo_alone_never_completes() {
        let mut m = CompletionMonitor::default();
        m.handle_input(b"fix the parser\r");
        m.handle_output(b"\x1b[2K> fix the parser");
        assert!(!m.poll(Duration::from_secs(10)));
        assert!(m.pending());
    }

    #[test]
    fn answer_then_quiet_completes_once() {
        let mut m = CompletionMonitor::default();
        m.handle_input(b"hello\x7f\x7fp\r");
        assert_eq!(m.prompt, "help");
        m.submitted_at -= Duration::from_secs(1);
        m.handle_output(b"Here is what I found in the code base.");
        assert!(!m.poll(Duration::from_millis(500)));
        assert!(m.poll(Duration::from_secs(5)));
        assert!(!m.poll(Duration::from_secs(9)));
        assert!(!m.pending());
    }

    #[test]
    fn output_without_a_prompt_is_ignored() {
        let mut m = CompletionMonitor::default();
        m.handle_output(b"startup banner and plenty of text");
        assert!(!m.pending());
        assert!(!m.poll(Duration::from_secs(10)));
    }
}
