//! Mermaid diagrams in markdown (`MermaidDiagram` upstream, which runs mermaid.js), drawn
//! natively: flowcharts (`graph`/`flowchart`), sequence diagrams and state diagrams, the kinds
//! agents write in docs and plans. Other kinds stay a code block.
//!
//! A diagram is parsed and laid out once per source text and cached in egui's memory; colors come
//! from the theme at paint time, so a theme switch needs no relayout.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{pos2, vec2, Color32, CornerRadius, Galley, Pos2, Rect, Shape as EShape, Stroke, Vec2};

use super::widgets::{mono, sans};
use crate::theme::Theme;

// ------------------------------------------------------------------ model

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dir {
    TopDown,
    BottomUp,
    LeftRight,
    RightLeft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeShape {
    Rect,
    Round,
    Stadium,
    Circle,
    Diamond,
    Hexagon,
    Subroutine,
    Cylinder,
    Flag,
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
struct Node {
    id: String,
    label: String,
    shape: NodeShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct LinkStyle {
    dashed: bool,
    thick: bool,
    head: bool,
    tail: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Edge {
    from: usize,
    to: usize,
    label: Option<String>,
    style: LinkStyle,
}

#[derive(Clone, Debug, PartialEq)]
struct Flow {
    dir: Dir,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq)]
enum SeqStep {
    Message {
        from: usize,
        to: usize,
        text: String,
        dashed: bool,
        head: bool,
    },
    Note {
        first: usize,
        last: usize,
        /// -1 left of, 0 over, 1 right of.
        side: i8,
        text: String,
    },
    BlockStart(String),
    BlockElse(String),
    BlockEnd,
}

#[derive(Clone, Debug, PartialEq)]
struct Sequence {
    participants: Vec<(String, String)>,
    steps: Vec<SeqStep>,
    autonumber: bool,
}

enum Parsed {
    Flow(Flow),
    Sequence(Sequence),
}

// ------------------------------------------------------------------ parsing

fn clean_label(raw: &str) -> String {
    let mut text = raw.trim();
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        text = &text[1..text.len() - 1];
    }
    let mut out = text.to_string();
    for br in ["<br/>", "<br />", "<br>", "<BR>", "<BR/>"] {
        out = out.replace(br, "\n");
    }
    out.replace("#quot;", "\"").replace("\\n", "\n")
}

/// Statements: one per line or `;`, comments and blank lines dropped.
fn statements(code: &str) -> Vec<String> {
    code.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"))
        .flat_map(|l| {
            // `;` ends a statement unless it sits inside a label.
            let mut parts = Vec::new();
            let mut depth = 0i32;
            let mut quoted = false;
            let mut start = 0;
            for (i, c) in l.char_indices() {
                match c {
                    '"' => quoted = !quoted,
                    '[' | '(' | '{' if !quoted => depth += 1,
                    ']' | ')' | '}' if !quoted => depth -= 1,
                    ';' if !quoted && depth <= 0 => {
                        parts.push(l[start..i].trim().to_string());
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            parts.push(l[start..].trim().to_string());
            parts.into_iter().filter(|p| !p.is_empty())
        })
        .collect()
}

fn parse(code: &str) -> Option<Parsed> {
    let lines = statements(code);
    let (head, body) = lines.split_first()?;
    let mut words = head.split_whitespace();
    let kind = words.next()?;
    match kind {
        "graph" | "flowchart" => {
            let dir = parse_dir(words.next().unwrap_or("TD"));
            parse_flow(dir, body).map(Parsed::Flow)
        }
        "stateDiagram" | "stateDiagram-v2" => parse_state(body).map(Parsed::Flow),
        "sequenceDiagram" => parse_sequence(body).map(Parsed::Sequence),
        _ => None,
    }
}

fn parse_dir(word: &str) -> Dir {
    match word {
        "LR" => Dir::LeftRight,
        "RL" => Dir::RightLeft,
        "BT" => Dir::BottomUp,
        _ => Dir::TopDown,
    }
}

struct FlowBuilder {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    index: HashMap<String, usize>,
}

impl FlowBuilder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn node(&mut self, id: &str, label: Option<String>, shape: Option<NodeShape>) -> usize {
        let i = *self.index.entry(id.to_string()).or_insert_with(|| {
            self.nodes.push(Node {
                id: id.to_string(),
                label: id.to_string(),
                shape: NodeShape::Rect,
            });
            self.nodes.len() - 1
        });
        if let Some(label) = label {
            self.nodes[i].label = label;
        }
        if let Some(shape) = shape {
            self.nodes[i].shape = shape;
        }
        i
    }

    fn finish(self, dir: Dir) -> Option<Flow> {
        (!self.nodes.is_empty()).then_some(Flow {
            dir,
            nodes: self.nodes,
            edges: self.edges,
        })
    }
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `id` plus an optional shaped label: `A`, `A[text]`, `B{"q?"}`, `C((round))`…
fn parse_node(s: &str) -> Option<(String, Option<String>, Option<NodeShape>, &str)> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut end = 0;
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k].1;
        let next = chars.get(k + 1).map(|x| x.1);
        let ok = is_id_char(c) || ((c == '-' || c == '.') && k > 0 && next.is_some_and(is_id_char));
        if !ok {
            break;
        }
        k += 1;
        end = chars.get(k).map_or(s.len(), |x| x.0);
    }
    if end == 0 {
        return None;
    }
    let id = s[..end].to_string();
    let rest = &s[end..];
    const SHAPES: [(&str, &str, NodeShape); 14] = [
        ("(((", ")))", NodeShape::Circle),
        ("((", "))", NodeShape::Circle),
        ("([", "])", NodeShape::Stadium),
        ("[[", "]]", NodeShape::Subroutine),
        ("[(", ")]", NodeShape::Cylinder),
        ("{{", "}}", NodeShape::Hexagon),
        ("[/", "/]", NodeShape::Rect),
        ("[/", "\\]", NodeShape::Rect),
        ("[\\", "\\]", NodeShape::Rect),
        ("[\\", "/]", NodeShape::Rect),
        ("[", "]", NodeShape::Rect),
        ("(", ")", NodeShape::Round),
        ("{", "}", NodeShape::Diamond),
        (">", "]", NodeShape::Flag),
    ];
    for (open, close, shape) in SHAPES {
        let Some(inner) = rest.strip_prefix(open) else {
            continue;
        };
        // A quoted label may hold the closing characters.
        let body_end = if inner.trim_start().starts_with('"') {
            let q = inner.find('"')?;
            let q2 = inner[q + 1..].find('"')? + q + 1;
            inner[q2..].find(close)? + q2
        } else {
            inner.find(close)?
        };
        let label = clean_label(&inner[..body_end]);
        let mut after = &inner[body_end + close.len()..];
        after = strip_class(after);
        return Some((id, Some(label), Some(shape), after));
    }
    Some((id, None, None, strip_class(rest)))
}

/// Drops a `:::className` suffix.
fn strip_class(s: &str) -> &str {
    match s.strip_prefix(":::") {
        Some(rest) => {
            let end = rest
                .find(|c: char| !is_id_char(c) && c != '-')
                .unwrap_or(rest.len());
            &rest[end..]
        }
        None => s,
    }
}

/// `-->`, `---`, `-.->`, `==>`, `--x`, `<-->`, `-- text -->`, `-->|text|`…
fn parse_link(s: &str) -> Option<(LinkStyle, Option<String>, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    let tail = b.first() == Some(&b'<');
    if tail {
        i += 1;
    }
    let start = i;
    while i < b.len() && matches!(b[i], b'-' | b'=' | b'.') {
        i += 1;
    }
    let body = &s[start..i];
    if body.len() < 2 && !(body == "-" && b.get(i) == Some(&b'>')) {
        return None;
    }
    let mut style = LinkStyle {
        dashed: body.contains('.'),
        thick: body.contains('='),
        head: false,
        tail,
    };
    let at_break = |j: usize| j >= b.len() || b[j].is_ascii_whitespace() || b[j] == b'|';
    let mut label = None;
    if i < b.len() && b[i] == b'>' {
        style.head = true;
        i += 1;
    } else if i < b.len() && matches!(b[i], b'x' | b'o') && at_break(i + 1) {
        style.head = true;
        i += 1;
    } else if matches!(body, "--" | "==" | "-.") && i < b.len() && b[i] == b' ' {
        // Text on the link: `-- text -->`.
        let rest = &s[i..];
        let (pos, term) = ["-->", "---", "==>", "===", ".->", ".-", "--x", "--o"]
            .iter()
            .filter_map(|t| rest.find(t).map(|p| (p, *t)))
            .min_by_key(|(p, t)| (*p, std::cmp::Reverse(t.len())))?;
        label = Some(clean_label(&rest[..pos]));
        let mut j = i + pos + term.len();
        while j < b.len() && matches!(b[j], b'-' | b'=' | b'.') {
            j += 1;
        }
        style.head = term.ends_with('>') || term.ends_with('x') || term.ends_with('o');
        if j < b.len() && b[j] == b'>' {
            style.head = true;
            j += 1;
        }
        i = j;
    }
    let mut rest = s[i..].trim_start();
    if let Some(inner) = rest.strip_prefix('|') {
        let close = inner.find('|')?;
        label = Some(clean_label(&inner[..close]));
        rest = &inner[close + 1..];
    }
    Some((style, label, rest))
}

fn parse_flow(dir: Dir, body: &[String]) -> Option<Flow> {
    let mut flow = FlowBuilder::new();
    let mut dir = dir;
    for line in body {
        let first = line.split_whitespace().next().unwrap_or_default();
        match first {
            "classDef" | "class" | "style" | "linkStyle" | "click" | "end" | "accTitle"
            | "accDescr" => continue,
            "direction" => {
                dir = parse_dir(line.split_whitespace().nth(1).unwrap_or("TD"));
                continue;
            }
            "subgraph" => continue,
            _ => {}
        }
        parse_chain(&mut flow, line);
    }
    flow.finish(dir)
}

/// `A & B --> C -->|x| D`.
fn parse_chain(flow: &mut FlowBuilder, line: &str) {
    let mut rest = line.trim();
    let mut prev: Vec<usize> = Vec::new();
    let mut link: Option<(LinkStyle, Option<String>)> = None;
    loop {
        let mut group = Vec::new();
        loop {
            let Some((id, label, shape, after)) = parse_node(rest) else {
                return;
            };
            group.push(flow.node(&id, label, shape));
            rest = after.trim_start();
            match rest.strip_prefix('&') {
                Some(r) => rest = r.trim_start(),
                None => break,
            }
        }
        if let Some((style, label)) = link.take() {
            for &a in &prev {
                for &b in &group {
                    flow.edges.push(Edge {
                        from: a,
                        to: b,
                        label: label.clone(),
                        style,
                    });
                }
            }
        }
        prev = group;
        if rest.is_empty() {
            return;
        }
        let Some((style, label, after)) = parse_link(rest) else {
            return;
        };
        link = Some((style, label));
        rest = after.trim_start();
    }
}

fn parse_state(body: &[String]) -> Option<Flow> {
    let mut flow = FlowBuilder::new();
    let mut dir = Dir::TopDown;
    let state_node = |flow: &mut FlowBuilder, raw: &str, source: bool| {
        let raw = raw.trim();
        if raw == "[*]" {
            let (id, shape) = if source {
                ("[*]start", NodeShape::Start)
            } else {
                ("[*]end", NodeShape::End)
            };
            flow.node(id, Some(String::new()), Some(shape))
        } else {
            let i = flow.node(raw, None, None);
            if flow.nodes[i].shape == NodeShape::Rect {
                flow.nodes[i].shape = NodeShape::Round;
            }
            i
        }
    };
    for line in body {
        let line = line.trim();
        if line.starts_with("direction ") {
            dir = parse_dir(line.split_whitespace().nth(1).unwrap_or("TB"));
            continue;
        }
        if line == "}" || line.starts_with("note ") || line.starts_with("classDef") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("state ") {
            let rest = rest.trim_end_matches('{').trim();
            if let Some((label, id)) = rest.split_once(" as ") {
                let i = state_node(&mut flow, id.split_whitespace().next().unwrap_or(id), false);
                flow.nodes[i].label = clean_label(label);
            } else if let Some(id) = rest.split_whitespace().next() {
                state_node(&mut flow, id, false);
            }
            continue;
        }
        let (main, label) = match line.split_once(" : ").or_else(|| line.split_once(':')) {
            Some((m, l)) => (m.trim(), Some(clean_label(l))),
            None => (line, None),
        };
        if let Some((a, b)) = main.split_once("-->") {
            let from = state_node(&mut flow, a, true);
            let to = state_node(&mut flow, b, false);
            flow.edges.push(Edge {
                from,
                to,
                label,
                style: LinkStyle {
                    head: true,
                    ..Default::default()
                },
            });
        } else if let Some(text) = label {
            // `Id : description`.
            let i = state_node(&mut flow, main, false);
            flow.nodes[i].label = text;
        } else if !main.contains(char::is_whitespace) {
            state_node(&mut flow, main, false);
        }
    }
    flow.finish(dir)
}

fn parse_sequence(body: &[String]) -> Option<Sequence> {
    let mut seq = Sequence {
        participants: Vec::new(),
        steps: Vec::new(),
        autonumber: false,
    };
    fn who(seq: &mut Sequence, id: &str) -> usize {
        let id = id.trim().trim_start_matches(['+', '-']).trim();
        if let Some(i) = seq.participants.iter().position(|(p, _)| p == id) {
            return i;
        }
        seq.participants.push((id.to_string(), id.to_string()));
        seq.participants.len() - 1
    }
    const ARROWS: [(&str, bool, bool); 8] = [
        ("-->>", true, true),
        ("->>", false, true),
        ("--x", true, true),
        ("-x", false, true),
        ("--)", true, true),
        ("-)", false, true),
        ("-->", true, false),
        ("->", false, false),
    ];
    for line in body {
        let line = line.trim();
        let lower = line.to_ascii_lowercase();
        let first = lower.split_whitespace().next().unwrap_or_default();
        match first {
            "participant" | "actor" => {
                let rest = line[first.len()..].trim();
                let (id, label) = match rest.split_once(" as ") {
                    Some((id, label)) => (id.trim(), clean_label(label)),
                    None => (rest, clean_label(rest)),
                };
                let i = who(&mut seq, id);
                seq.participants[i].1 = label;
                continue;
            }
            "autonumber" => {
                seq.autonumber = true;
                continue;
            }
            "activate" | "deactivate" | "title" | "box" | "create" | "destroy" | "link"
            | "links" => continue,
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                let label = line[first.len()..].trim();
                seq.steps.push(SeqStep::BlockStart(if first == "rect" {
                    String::new()
                } else {
                    format!("{first} {label}").trim().to_string()
                }));
                continue;
            }
            "else" | "and" | "option" => {
                seq.steps
                    .push(SeqStep::BlockElse(line[first.len()..].trim().to_string()));
                continue;
            }
            "end" => {
                seq.steps.push(SeqStep::BlockEnd);
                continue;
            }
            "note" => {
                let (place, text) = line.split_once(':').unwrap_or((line, ""));
                let place = place[4..].trim();
                let lower = place.to_ascii_lowercase();
                let (side, who_part) = if let Some(r) = lower.strip_prefix("left of") {
                    (-1, &place[place.len() - r.len()..])
                } else if let Some(r) = lower.strip_prefix("right of") {
                    (1, &place[place.len() - r.len()..])
                } else if let Some(r) = lower.strip_prefix("over") {
                    (0, &place[place.len() - r.len()..])
                } else {
                    continue;
                };
                let mut ids = who_part.split(',').map(str::trim).filter(|s| !s.is_empty());
                let Some(a) = ids.next() else { continue };
                let first_i = who(&mut seq, a);
                let last_i = ids.next().map_or(first_i, |b| who(&mut seq, b));
                seq.steps.push(SeqStep::Note {
                    first: first_i.min(last_i),
                    last: first_i.max(last_i),
                    side,
                    text: clean_label(text),
                });
                continue;
            }
            _ => {}
        }
        let (main, text) = line.split_once(':').unwrap_or((line, ""));
        let Some((pos, arrow, dashed, head)) = ARROWS
            .iter()
            .filter_map(|(a, d, h)| main.find(a).map(|p| (p, *a, *d, *h)))
            .min_by_key(|(p, a, ..)| (*p, std::cmp::Reverse(a.len())))
        else {
            continue;
        };
        let from_id = main[..pos].trim();
        let to_id = main[pos + arrow.len()..].trim();
        if from_id.is_empty() || to_id.is_empty() {
            continue;
        }
        let from = who(&mut seq, from_id);
        let to = who(&mut seq, to_id);
        seq.steps.push(SeqStep::Message {
            from,
            to,
            text: clean_label(text),
            dashed,
            head: head || arrow.ends_with('>'),
        });
    }
    (!seq.participants.is_empty()).then_some(seq)
}

// ------------------------------------------------------------------ layout

#[derive(Clone)]
enum Item {
    Node {
        rect: Rect,
        shape: NodeShape,
        text: Arc<Galley>,
    },
    Edge {
        points: Vec<Pos2>,
        style: LinkStyle,
        label: Option<(Pos2, Arc<Galley>)>,
    },
    Lifeline {
        from: Pos2,
        to: Pos2,
    },
    Note {
        rect: Rect,
        text: Arc<Galley>,
    },
    Frame {
        rect: Rect,
        title: Option<Arc<Galley>>,
    },
    Divider {
        from: Pos2,
        to: Pos2,
        text: Option<Arc<Galley>>,
    },
}

#[derive(Clone)]
struct Diagram {
    size: Vec2,
    items: Vec<Item>,
}

type Measure<'a> = dyn FnMut(&str, f32) -> Arc<Galley> + 'a;

const MARGIN: f32 = 12.0;

fn node_size(shape: NodeShape, text: Vec2) -> Vec2 {
    match shape {
        NodeShape::Start => vec2(16.0, 16.0),
        NodeShape::End => vec2(20.0, 20.0),
        NodeShape::Circle => Vec2::splat(text.x.max(text.y) + 26.0),
        NodeShape::Diamond => vec2(text.x * 1.35 + 30.0, text.y * 1.35 + 26.0),
        NodeShape::Hexagon => vec2(text.x + 44.0, text.y + 16.0),
        NodeShape::Subroutine => vec2(text.x + 36.0, text.y + 16.0),
        NodeShape::Cylinder => vec2(text.x + 24.0, text.y + 26.0),
        _ => vec2(text.x + 26.0, text.y + 16.0),
    }
}

/// Where the segment from `center` towards `toward` leaves `rect`.
fn clip(rect: Rect, shape: NodeShape, toward: Pos2) -> Pos2 {
    let c = rect.center();
    let d = toward - c;
    if d.length() < 0.5 {
        return c;
    }
    let half = rect.size() / 2.0;
    match shape {
        NodeShape::Circle | NodeShape::Start | NodeShape::End => c + d.normalized() * half.x,
        NodeShape::Diamond => {
            let t = 1.0 / (d.x.abs() / half.x + d.y.abs() / half.y);
            c + d * t
        }
        _ => {
            let tx = if d.x.abs() > 1e-3 {
                half.x / d.x.abs()
            } else {
                f32::INFINITY
            };
            let ty = if d.y.abs() > 1e-3 {
                half.y / d.y.abs()
            } else {
                f32::INFINITY
            };
            c + d * tx.min(ty)
        }
    }
}

fn layout_flow(flow: &Flow, measure: &mut Measure) -> Diagram {
    let n = flow.nodes.len();
    let texts: Vec<Arc<Galley>> = flow
        .nodes
        .iter()
        .map(|nd| measure(&nd.label, 180.0))
        .collect();
    let sizes: Vec<Vec2> = flow
        .nodes
        .iter()
        .zip(&texts)
        .map(|(nd, t)| node_size(nd.shape, t.size()))
        .collect();

    // Break cycles: edges that close a DFS cycle don't constrain ranks.
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, e) in flow.edges.iter().enumerate() {
        if e.from != e.to {
            out[e.from].push(i);
        }
    }
    let mut state = vec![0u8; n];
    let mut back = vec![false; flow.edges.len()];
    fn dfs(v: usize, flow: &Flow, out: &[Vec<usize>], state: &mut [u8], back: &mut [bool]) {
        state[v] = 1;
        for &ei in &out[v] {
            let w = flow.edges[ei].to;
            if state[w] == 1 {
                back[ei] = true;
            } else if state[w] == 0 {
                dfs(w, flow, out, state, back);
            }
        }
        state[v] = 2;
    }
    for v in 0..n {
        if state[v] == 0 {
            dfs(v, flow, &out, &mut state, &mut back);
        }
    }
    let forward: Vec<(usize, usize)> = flow
        .edges
        .iter()
        .enumerate()
        .filter(|(i, e)| !back[*i] && e.from != e.to)
        .map(|(_, e)| (e.from, e.to))
        .collect();
    // Longest path from the sources (Kahn order).
    let mut indeg = vec![0usize; n];
    for &(_, b) in &forward {
        indeg[b] += 1;
    }
    let mut rank = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let v = queue[qi];
        qi += 1;
        for &(a, b) in &forward {
            if a == v {
                rank[b] = rank[b].max(rank[v] + 1);
                indeg[b] -= 1;
                if indeg[b] == 0 {
                    queue.push(b);
                }
            }
        }
    }
    let layers_n = rank.iter().max().map_or(0, |m| m + 1);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); layers_n];
    for v in 0..n {
        layers[rank[v]].push(v);
    }
    // Barycenter sweeps to untangle crossings.
    let mut pos = vec![0.0f32; n];
    let set_pos = |layers: &Vec<Vec<usize>>, pos: &mut Vec<f32>| {
        for layer in layers {
            for (i, &v) in layer.iter().enumerate() {
                pos[v] = i as f32;
            }
        }
    };
    set_pos(&layers, &mut pos);
    for _ in 0..4 {
        for down in [true, false] {
            let order: Vec<usize> = if down {
                (1..layers_n).collect()
            } else {
                (0..layers_n.saturating_sub(1)).rev().collect()
            };
            for r in order {
                let mut keyed: Vec<(f32, usize)> = layers[r]
                    .iter()
                    .map(|&v| {
                        let neighbors: Vec<f32> = forward
                            .iter()
                            .filter_map(|&(a, b)| {
                                if down && b == v && rank[a] + 1 == r {
                                    Some(pos[a])
                                } else if !down && a == v && rank[b] == r + 1 {
                                    Some(pos[b])
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let key = if neighbors.is_empty() {
                            pos[v]
                        } else {
                            neighbors.iter().sum::<f32>() / neighbors.len() as f32
                        };
                        (key, v)
                    })
                    .collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
                layers[r] = keyed.into_iter().map(|(_, v)| v).collect();
                set_pos(&layers, &mut pos);
            }
        }
    }

    // Coordinates: `main` runs along the ranks, `cross` across them.
    let horizontal = matches!(flow.dir, Dir::LeftRight | Dir::RightLeft);
    let main_of = |s: Vec2| if horizontal { s.x } else { s.y };
    let cross_of = |s: Vec2| if horizontal { s.y } else { s.x };
    let labelled_gap: Vec<bool> = (0..layers_n)
        .map(|r| {
            flow.edges
                .iter()
                .any(|e| e.label.is_some() && rank[e.from] == r && rank[e.to] > r)
        })
        .collect();
    let (main_gap, cross_gap) = if horizontal {
        (60.0, 22.0)
    } else {
        (46.0, 28.0)
    };
    let mut layer_main = Vec::with_capacity(layers_n);
    let mut cursor = 0.0;
    for (r, layer) in layers.iter().enumerate() {
        let thick = layer.iter().map(|&v| main_of(sizes[v])).fold(0.0, f32::max);
        layer_main.push((cursor, thick));
        cursor += thick + main_gap + if labelled_gap[r] { 24.0 } else { 0.0 };
    }
    let total_main = (cursor - main_gap).max(0.0);
    let widths: Vec<f32> = layers
        .iter()
        .map(|l| {
            l.iter().map(|&v| cross_of(sizes[v])).sum::<f32>()
                + cross_gap * l.len().saturating_sub(1) as f32
        })
        .collect();
    let total_cross = widths.iter().copied().fold(0.0, f32::max);
    let mut rects = vec![Rect::NOTHING; n];
    for (r, layer) in layers.iter().enumerate() {
        let (m0, thick) = layer_main[r];
        let mut c = (total_cross - widths[r]) / 2.0;
        for &v in layer {
            let s = sizes[v];
            let mut m = m0 + (thick - main_of(s)) / 2.0;
            if matches!(flow.dir, Dir::BottomUp | Dir::RightLeft) {
                m = total_main - m - main_of(s);
            }
            let min = if horizontal { pos2(m, c) } else { pos2(c, m) };
            rects[v] = Rect::from_min_size(min + Vec2::splat(MARGIN), s);
            c += cross_of(s) + cross_gap;
        }
    }

    let mut items = Vec::new();
    let mut lanes = 0;
    // How far side lanes and their labels reach across the ranks.
    let mut lane_extent: f32 = 0.0;
    for e in &flow.edges {
        let (a, b) = (rects[e.from], rects[e.to]);
        let points = if e.from == e.to {
            let r = a.right_center();
            vec![
                r + vec2(0.0, -6.0),
                r + vec2(18.0, -12.0),
                r + vec2(18.0, 12.0),
                r + vec2(0.0, 6.0),
            ]
        } else if rank[e.to] != rank[e.from] + 1 && rank[e.to] != rank[e.from] {
            // Back edges and edges skipping a rank go round the side, clear of the nodes between.
            let (lo, hi) = (rank[e.from].min(rank[e.to]), rank[e.from].max(rank[e.to]));
            let spanned = (0..n).filter(|&v| rank[v] >= lo && rank[v] <= hi);
            let lane_at = spanned
                .map(|v| {
                    if horizontal {
                        rects[v].bottom()
                    } else {
                        rects[v].right()
                    }
                })
                .fold(0.0, f32::max)
                + 18.0
                + lanes as f32 * 14.0;
            lanes += 1;
            if horizontal {
                lane_extent = lane_extent.max(lane_at);
                vec![
                    a.center_bottom(),
                    pos2(a.center().x, lane_at),
                    pos2(b.center().x, lane_at),
                    b.center_bottom(),
                ]
            } else {
                lane_extent = lane_extent.max(lane_at);
                vec![
                    a.right_center(),
                    pos2(lane_at, a.center().y),
                    pos2(lane_at, b.center().y),
                    b.right_center(),
                ]
            }
        } else {
            vec![
                clip(a, flow.nodes[e.from].shape, b.center()),
                clip(b, flow.nodes[e.to].shape, a.center()),
            ]
        };
        let label = e.label.as_ref().filter(|l| !l.is_empty()).map(|l| {
            let galley = measure(l, 160.0);
            let mid = match points.len() {
                2 => points[0] + (points[1] - points[0]) / 2.0,
                // A side lane: the label sits beside its middle run.
                _ if e.from != e.to => {
                    let m = points[1] + (points[2] - points[1]) / 2.0;
                    if horizontal {
                        m + vec2(0.0, galley.size().y / 2.0 + 4.0)
                    } else {
                        m + vec2(galley.size().x / 2.0 + 6.0, 0.0)
                    }
                }
                _ => points[1] + vec2(4.0, 12.0),
            };
            if horizontal {
                lane_extent = lane_extent.max(mid.y + galley.size().y / 2.0);
            } else {
                lane_extent = lane_extent.max(mid.x + galley.size().x / 2.0);
            }
            (mid, galley)
        });
        items.push(Item::Edge {
            points,
            style: e.style,
            label,
        });
    }
    for v in 0..n {
        items.push(Item::Node {
            rect: rects[v],
            shape: flow.nodes[v].shape,
            text: texts[v].clone(),
        });
    }
    let cross = total_cross.max(lane_extent - MARGIN);
    let size = if horizontal {
        vec2(total_main, cross)
    } else {
        vec2(cross, total_main)
    } + Vec2::splat(MARGIN * 2.0)
        + vec2(24.0, 0.0);
    Diagram { size, items }
}

fn layout_sequence(seq: &Sequence, measure: &mut Measure) -> Diagram {
    let count = seq.participants.len();
    let heads: Vec<Arc<Galley>> = seq
        .participants
        .iter()
        .map(|(_, label)| measure(label, 160.0))
        .collect();
    let box_w: Vec<f32> = heads.iter().map(|g| g.size().x + 24.0).collect();
    let box_h = heads.iter().map(|g| g.size().y).fold(0.0, f32::max) + 16.0;
    // Room between neighbours for the messages that cross that gap.
    let mut gaps: Vec<f32> = (0..count.saturating_sub(1))
        .map(|i| ((box_w[i] + box_w[i + 1]) / 2.0 + 30.0).max(130.0))
        .collect();
    let mut texts: Vec<Option<Arc<Galley>>> = Vec::new();
    let mut number = 0;
    for step in &seq.steps {
        let galley = match step {
            SeqStep::Message { from, to, text, .. } => {
                number += 1;
                let text = if seq.autonumber {
                    format!("{number}. {text}")
                } else {
                    text.clone()
                };
                let g = (!text.is_empty()).then(|| measure(&text, 260.0));
                if let Some(g) = &g {
                    let (a, b) = ((*from).min(*to), (*from).max(*to));
                    if a == b {
                        if a < gaps.len() {
                            gaps[a] = gaps[a].max(g.size().x + 50.0);
                        }
                    } else {
                        let span: f32 = gaps[a..b].iter().sum();
                        let need = g.size().x + 30.0;
                        if span < need {
                            let extra = (need - span) / (b - a) as f32;
                            for gap in &mut gaps[a..b] {
                                *gap += extra;
                            }
                        }
                    }
                }
                g
            }
            SeqStep::Note { text, .. } => Some(measure(text, 200.0)),
            SeqStep::BlockStart(label) | SeqStep::BlockElse(label) => {
                (!label.is_empty()).then(|| measure(label, 300.0))
            }
            SeqStep::BlockEnd => None,
        };
        texts.push(galley);
    }
    let mut xs = Vec::with_capacity(count);
    let mut x = MARGIN + box_w.first().copied().unwrap_or(0.0) / 2.0;
    for i in 0..count {
        xs.push(x);
        if i < gaps.len() {
            x += gaps[i];
        }
    }
    let left = MARGIN;
    let right = xs.last().copied().unwrap_or(0.0) + box_w.last().copied().unwrap_or(0.0) / 2.0;
    let mut items = Vec::new();
    let mut y = MARGIN + box_h + 18.0;
    // Open blocks: top, title, and the first item inside.
    let mut open: Vec<(f32, Option<Arc<Galley>>, usize)> = Vec::new();
    // A frame reaches past the participants when a message label inside it does.
    let reach = |items: &[Item]| {
        items
            .iter()
            .filter_map(|it| match it {
                Item::Edge { points, label, .. } => Some(
                    points.iter().map(|p| p.x).fold(
                        label
                            .as_ref()
                            .map_or(0.0, |(p, g)| p.x + g.size().x / 2.0 + 4.0),
                        f32::max,
                    ),
                ),
                Item::Note { rect, .. } | Item::Frame { rect, .. } => Some(rect.right()),
                _ => None,
            })
            .fold(0.0, f32::max)
    };
    for (step, text) in seq.steps.iter().zip(texts) {
        match step {
            SeqStep::Message {
                from,
                to,
                dashed,
                head,
                ..
            } => {
                let text_h = text.as_ref().map_or(0.0, |g| g.size().y);
                let style = LinkStyle {
                    dashed: *dashed,
                    head: *head,
                    ..Default::default()
                };
                if from == to {
                    let x0 = xs[*from];
                    let top = y + text_h + 4.0;
                    items.push(Item::Edge {
                        points: vec![
                            pos2(x0, top),
                            pos2(x0 + 34.0, top),
                            pos2(x0 + 34.0, top + 16.0),
                            pos2(x0, top + 16.0),
                        ],
                        style,
                        label: text
                            .map(|g| (pos2(x0 + 40.0 + g.size().x / 2.0, y + g.size().y / 2.0), g)),
                    });
                    y += text_h + 34.0;
                } else {
                    let line_y = y + text_h + 6.0;
                    let (xa, xb) = (xs[*from], xs[*to]);
                    items.push(Item::Edge {
                        points: vec![pos2(xa, line_y), pos2(xb, line_y)],
                        style,
                        label: text.map(|g| (pos2((xa + xb) / 2.0, y + g.size().y / 2.0 + 2.0), g)),
                    });
                    y += text_h + 22.0;
                }
            }
            SeqStep::Note {
                first, last, side, ..
            } => {
                let g = text.expect("note text");
                let w = g.size().x + 16.0;
                let h = g.size().y + 10.0;
                let x0 = match side {
                    -1 => xs[*first] - 12.0 - w,
                    1 => xs[*first] + 12.0,
                    _ => {
                        let span = xs[*last] - xs[*first];
                        let w = w.max(span + 40.0);
                        xs[*first] + span / 2.0 - w / 2.0
                    }
                };
                let w = if *side == 0 {
                    w.max(xs[*last] - xs[*first] + 40.0)
                } else {
                    w
                };
                items.push(Item::Note {
                    rect: Rect::from_min_size(pos2(x0, y), vec2(w, h)),
                    text: g,
                });
                y += h + 12.0;
            }
            SeqStep::BlockStart(_) => {
                open.push((y, text, items.len()));
                y += 24.0;
            }
            SeqStep::BlockElse(_) => {
                items.push(Item::Divider {
                    from: pos2(left - 4.0, y),
                    to: pos2(right + 4.0, y),
                    text,
                });
                y += 24.0;
            }
            SeqStep::BlockEnd => {
                if let Some((top, title, first)) = open.pop() {
                    let inset = open.len() as f32 * 6.0;
                    let edge = (right + 8.0 - inset).max(reach(&items[first..]) + 8.0);
                    items.push(Item::Frame {
                        rect: Rect::from_min_max(
                            pos2(left - 8.0 + inset, top),
                            pos2(edge, y + 2.0),
                        ),
                        title,
                    });
                    y += 12.0;
                }
            }
        }
    }
    // Blocks left open close at the end.
    while let Some((top, title, first)) = open.pop() {
        let edge = (right + 8.0).max(reach(&items[first..]) + 8.0);
        items.push(Item::Frame {
            rect: Rect::from_min_max(pos2(left - 8.0, top), pos2(edge, y)),
            title,
        });
    }
    let bottom = y + 10.0;
    for i in 0..count {
        items.insert(
            0,
            Item::Lifeline {
                from: pos2(xs[i], MARGIN + box_h),
                to: pos2(xs[i], bottom),
            },
        );
        for top in [MARGIN, bottom] {
            items.push(Item::Node {
                rect: Rect::from_center_size(pos2(xs[i], top + box_h / 2.0), vec2(box_w[i], box_h)),
                shape: NodeShape::Rect,
                text: heads[i].clone(),
            });
        }
    }
    // Notes left of the first participant can reach past the margin: shift everything right.
    let min_x = items
        .iter()
        .filter_map(|it| match it {
            Item::Note { rect, .. } => Some(rect.left()),
            _ => None,
        })
        .fold(MARGIN, f32::min);
    let shift = (MARGIN - min_x).max(0.0);
    let max_x = items
        .iter()
        .filter_map(|it| match it {
            Item::Note { rect, .. } | Item::Frame { rect, .. } => Some(rect.right()),
            Item::Edge { points, label, .. } => Some(points.iter().map(|p| p.x).fold(
                label.as_ref().map_or(0.0, |(p, g)| p.x + g.size().x / 2.0),
                f32::max,
            )),
            _ => None,
        })
        .fold(right, f32::max);
    if shift > 0.0 {
        let d = vec2(shift, 0.0);
        for item in &mut items {
            match item {
                Item::Node { rect, .. } | Item::Note { rect, .. } | Item::Frame { rect, .. } => {
                    *rect = rect.translate(d)
                }
                Item::Edge { points, label, .. } => {
                    for p in points.iter_mut() {
                        *p += d;
                    }
                    if let Some((p, _)) = label {
                        *p += d;
                    }
                }
                Item::Lifeline { from, to } | Item::Divider { from, to, .. } => {
                    *from += d;
                    *to += d;
                }
            }
        }
    }
    Diagram {
        size: vec2(max_x + shift + MARGIN + 8.0, bottom + box_h + MARGIN),
        items,
    }
}

// ------------------------------------------------------------------ painting

fn arrow_head(painter: &egui::Painter, tip: Pos2, from: Pos2, color: Color32) {
    let dir = (tip - from).normalized();
    if !dir.x.is_finite() {
        return;
    }
    let side = vec2(-dir.y, dir.x);
    let base = tip - dir * 8.0;
    painter.add(EShape::convex_polygon(
        vec![tip, base + side * 4.0, base - side * 4.0],
        color,
        Stroke::NONE,
    ));
}

fn paint(painter: &egui::Painter, origin: Pos2, diagram: &Diagram, theme: &Theme) {
    let o = origin.to_vec2();
    let line = theme.fg_muted;
    for item in &diagram.items {
        match item {
            Item::Lifeline { from, to } => {
                painter.extend(EShape::dashed_line(
                    &[*from + o, *to + o],
                    Stroke::new(1.0, theme.border_strong),
                    4.0,
                    4.0,
                ));
            }
            Item::Frame { rect, title } => {
                let rect = rect.translate(o);
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, theme.border_strong),
                    egui::StrokeKind::Inside,
                );
                if let Some(title) = title {
                    let tab = Rect::from_min_size(rect.min, title.size() + vec2(12.0, 6.0));
                    painter.rect_filled(
                        tab,
                        CornerRadius {
                            nw: 4,
                            se: 4,
                            ..Default::default()
                        },
                        theme.panel_hover,
                    );
                    painter.galley(tab.min + vec2(6.0, 3.0), title.clone(), theme.fg_muted);
                }
            }
            Item::Divider { from, to, text } => {
                painter.extend(EShape::dashed_line(
                    &[*from + o, *to + o],
                    Stroke::new(1.0, theme.border_strong),
                    5.0,
                    3.0,
                ));
                if let Some(text) = text {
                    painter.galley(*from + o + vec2(10.0, 4.0), text.clone(), theme.fg_faint);
                }
            }
            Item::Edge {
                points,
                style,
                label,
            } => {
                let pts: Vec<Pos2> = points.iter().map(|p| *p + o).collect();
                let stroke = Stroke::new(if style.thick { 2.4 } else { 1.2 }, line);
                if style.dashed {
                    painter.extend(EShape::dashed_line(&pts, stroke, 4.0, 3.0));
                } else {
                    painter.add(EShape::line(pts.clone(), stroke));
                }
                let n = pts.len();
                if style.head && n >= 2 {
                    arrow_head(painter, pts[n - 1], pts[n - 2], line);
                }
                if style.tail && n >= 2 {
                    arrow_head(painter, pts[0], pts[1], line);
                }
                if let Some((at, galley)) = label {
                    let rect = Rect::from_center_size(*at + o, galley.size() + vec2(8.0, 4.0));
                    painter.rect_filled(rect, CornerRadius::same(3), theme.bg_elevated);
                    painter.galley(rect.min + vec2(4.0, 2.0), galley.clone(), theme.fg_muted);
                }
            }
            Item::Note { rect, text } => {
                let rect = rect.translate(o);
                painter.rect(
                    rect,
                    CornerRadius::same(3),
                    theme.accent_soft,
                    Stroke::new(1.0, theme.accent_border),
                    egui::StrokeKind::Inside,
                );
                painter.galley(rect.min + vec2(8.0, 5.0), text.clone(), theme.fg);
            }
            Item::Node { rect, shape, text } => {
                let rect = rect.translate(o);
                let fill = theme.panel;
                let stroke = Stroke::new(1.0, theme.border_strong);
                let c = rect.center();
                match shape {
                    NodeShape::Start => {
                        painter.circle_filled(c, rect.width() / 2.0, theme.fg_muted);
                        continue;
                    }
                    NodeShape::End => {
                        painter.circle_stroke(
                            c,
                            rect.width() / 2.0,
                            Stroke::new(1.5, theme.fg_muted),
                        );
                        painter.circle_filled(c, rect.width() / 2.0 - 4.0, theme.fg_muted);
                        continue;
                    }
                    NodeShape::Circle => {
                        painter.circle(c, rect.width() / 2.0, fill, stroke);
                    }
                    NodeShape::Diamond => {
                        painter.add(EShape::convex_polygon(
                            vec![
                                rect.center_top(),
                                rect.right_center(),
                                rect.center_bottom(),
                                rect.left_center(),
                            ],
                            fill,
                            stroke,
                        ));
                    }
                    NodeShape::Hexagon => {
                        let k = 14.0;
                        painter.add(EShape::convex_polygon(
                            vec![
                                rect.left_center(),
                                rect.left_top() + vec2(k, 0.0),
                                rect.right_top() - vec2(k, 0.0),
                                rect.right_center(),
                                rect.right_bottom() - vec2(k, 0.0),
                                rect.left_bottom() + vec2(k, 0.0),
                            ],
                            fill,
                            stroke,
                        ));
                    }
                    NodeShape::Flag => {
                        // Concave: a flag is a rectangle with a notch on the left.
                        painter.add(EShape::convex_polygon(
                            vec![
                                rect.left_top(),
                                rect.right_top(),
                                rect.right_bottom(),
                                rect.left_bottom(),
                            ],
                            fill,
                            stroke,
                        ));
                    }
                    other => {
                        let radius = match other {
                            NodeShape::Round => 8.0,
                            NodeShape::Stadium => rect.height() / 2.0,
                            NodeShape::Cylinder => 10.0,
                            _ => 3.0,
                        };
                        painter.rect(
                            rect,
                            CornerRadius::same(radius.min(120.0) as u8),
                            fill,
                            stroke,
                            egui::StrokeKind::Inside,
                        );
                        if *other == NodeShape::Subroutine {
                            for x in [rect.left() + 7.0, rect.right() - 7.0] {
                                painter.line_segment(
                                    [pos2(x, rect.top()), pos2(x, rect.bottom())],
                                    stroke,
                                );
                            }
                        }
                        if *other == NodeShape::Cylinder {
                            painter.line_segment(
                                [
                                    pos2(rect.left() + 2.0, rect.top() + 8.0),
                                    pos2(rect.right() - 2.0, rect.top() + 8.0),
                                ],
                                stroke,
                            );
                        }
                    }
                }
                painter.galley(c - text.size() / 2.0, text.clone(), theme.fg);
            }
        }
    }
}

// ------------------------------------------------------------------ entry point

/// Draws `code` as a diagram; false when its kind isn't supported (the caller shows the code).
pub fn show(ui: &mut egui::Ui, theme: &Theme, code: &str) -> bool {
    let mut hasher = std::hash::DefaultHasher::new();
    code.hash(&mut hasher);
    let id = egui::Id::unique(("mermaid", hasher.finish()));
    let cached = ui
        .ctx()
        .memory_mut(|m| m.data.get_temp::<Option<Arc<Diagram>>>(id));
    let diagram = match cached {
        Some(d) => d,
        None => {
            let parsed = parse(code);
            let built = parsed.map(|parsed| {
                let mut measure = |text: &str, wrap: f32| {
                    let job =
                        LayoutJob::simple(text.to_string(), sans(12.5), Color32::PLACEHOLDER, wrap);
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                Arc::new(match parsed {
                    Parsed::Flow(flow) => layout_flow(&flow, &mut measure),
                    Parsed::Sequence(seq) => layout_sequence(&seq, &mut measure),
                })
            });
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(id, built.clone()));
            built
        }
    };
    let Some(diagram) = diagram else {
        return false;
    };
    egui::Frame::new()
        .fill(theme.bg_elevated)
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new("mermaid")
                    .font(mono(10.0))
                    .color(theme.fg_faint),
            );
            egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
                let width = diagram.size.x.max(ui.available_width());
                let (rect, _) =
                    ui.allocate_exact_size(vec2(width, diagram.size.y), egui::Sense::hover());
                // Centered when narrower than the pane.
                let origin = rect.min + vec2((width - diagram.size.x) / 2.0, 0.0);
                paint(ui.painter(), origin, &diagram, theme);
            });
        });
    ui.add_space(4.0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(code: &str) -> Flow {
        match parse(code) {
            Some(Parsed::Flow(f)) => f,
            _ => panic!("not a flow: {code}"),
        }
    }

    fn labels(f: &Flow) -> Vec<(&str, &str)> {
        f.nodes
            .iter()
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect()
    }

    #[test]
    fn flowchart_nodes_shapes_and_chains() {
        let f = flow(
            "flowchart LR\n  A[Início] -->|sim| B{\"Pronto?\"}\n  B -- não --> C((fim))\n  B -.-> D([pílula]) ==> A\n  %% comentário\n  E & F --- G:::destaque",
        );
        assert_eq!(f.dir, Dir::LeftRight);
        assert_eq!(
            labels(&f),
            vec![
                ("A", "Início"),
                ("B", "Pronto?"),
                ("C", "fim"),
                ("D", "pílula"),
                ("E", "E"),
                ("F", "F"),
                ("G", "G"),
            ]
        );
        assert_eq!(f.nodes[1].shape, NodeShape::Diamond);
        assert_eq!(f.nodes[2].shape, NodeShape::Circle);
        assert_eq!(f.nodes[3].shape, NodeShape::Stadium);
        let edges: Vec<(usize, usize, Option<&str>)> = f
            .edges
            .iter()
            .map(|e| (e.from, e.to, e.label.as_deref()))
            .collect();
        assert_eq!(
            edges,
            vec![
                (0, 1, Some("sim")),
                (1, 2, Some("não")),
                (1, 3, None),
                (3, 0, None),
                (4, 6, None),
                (5, 6, None),
            ]
        );
        assert!(f.edges[0].style.head && !f.edges[0].style.dashed);
        assert!(f.edges[2].style.dashed && f.edges[2].style.head);
        assert!(f.edges[3].style.thick);
        assert!(!f.edges[4].style.head);
    }

    #[test]
    fn compact_and_semicolon_syntax() {
        let f = flow("graph TD;A-->B;B-->C[\"x; y\"];");
        assert_eq!(f.dir, Dir::TopDown);
        assert_eq!(f.nodes.len(), 3);
        assert_eq!(f.nodes[2].label, "x; y");
        assert_eq!(f.edges.len(), 2);
    }

    #[test]
    fn state_diagram_maps_to_a_flow() {
        let f = flow(
            "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Running : start\n  state \"Em execução\" as Running\n  Running --> [*]",
        );
        assert_eq!(f.nodes[0].shape, NodeShape::Start);
        assert_eq!(f.nodes[2].label, "Em execução");
        assert_eq!(f.nodes[3].shape, NodeShape::End);
        assert_eq!(f.edges[1].label.as_deref(), Some("start"));
    }

    #[test]
    fn sequence_messages_notes_and_blocks() {
        let Some(Parsed::Sequence(s)) = parse(
            "sequenceDiagram\n  autonumber\n  participant U as Usuário\n  U->>+API: POST /jobs\n  API-->>U: 202\n  Note over U,API: assíncrono\n  loop a cada 5s\n    U->>API: GET\n  end",
        ) else {
            panic!("not a sequence");
        };
        assert!(s.autonumber);
        assert_eq!(
            s.participants,
            vec![("U".into(), "Usuário".into()), ("API".into(), "API".into())]
        );
        assert_eq!(
            s.steps[0],
            SeqStep::Message {
                from: 0,
                to: 1,
                text: "POST /jobs".into(),
                dashed: false,
                head: true
            }
        );
        assert!(matches!(s.steps[1], SeqStep::Message { dashed: true, .. }));
        assert!(matches!(
            s.steps[2],
            SeqStep::Note {
                first: 0,
                last: 1,
                side: 0,
                ..
            }
        ));
        assert_eq!(s.steps[3], SeqStep::BlockStart("loop a cada 5s".into()));
        assert_eq!(s.steps[5], SeqStep::BlockEnd);
    }

    #[test]
    fn unsupported_kinds_fall_back() {
        assert!(parse("pie title Pets\n \"Dogs\" : 386").is_none());
        assert!(parse("gantt\n title x").is_none());
        assert!(parse("").is_none());
    }
}
