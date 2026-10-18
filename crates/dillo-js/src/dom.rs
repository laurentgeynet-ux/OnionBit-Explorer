// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Mini-DOM autonome — la structure d'arbre derrière la façade
//! `document`/`Element` (JS-2).
//!
//! Ce n'est **pas** le doctree du navigateur : c'est un DOM miroir
//! autonome, sans dépendance sur `dillo-html`/`dillo-dw`, pensé pour
//! les scripts « old school » (lire/écrire `innerHTML`, toggles de
//! classes, formulaires). Le bridge vers le vrai `Doctree` (JS-5)
//! remplacera l'alimentation de cette structure, pas l'API.
//!
//! Le parseur de fragments est volontairement *tolérant-simple* : il
//! n'a pas vocation à reproduire la récupération d'erreurs de
//! `html.cc` — c'est un DOM de script, pas le moteur de rendu.

use crate::value::Value;
use std::cell::Cell;

/// Arena handle.
pub type NodeId = u32;

/// Kind of node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// Element (`<div>`…). `tag` is lowercased; `"#document"` is the
    /// synthetic root.
    Element,
    /// Text run — payload in `text`, `tag` is `"#text"`.
    Text,
}

/// One node in the arena.
#[derive(Debug)]
pub struct Node {
    /// Element or text.
    pub kind: NodeKind,
    /// Lowercased tag name (`"#document"`, `"#text"` for non-elements).
    pub tag: String,
    /// Attributes in source order.
    pub attrs: Vec<(String, String)>,
    /// Child node ids, in order.
    pub children: Vec<NodeId>,
    /// Parent (none for the root / detached nodes).
    pub parent: Option<NodeId>,
    /// Text payload (kind == Text).
    pub text: String,
    /// `addEventListener` handlers: `(type, function, capture)` —
    /// dispatched by the host layer / platform events (JS-3).
    pub listeners: Vec<(String, Value, bool)>,
}

/// A standalone DOM tree: node arena + document root.
#[derive(Debug)]
pub struct Dom {
    /// Node arena (index = `NodeId`).
    pub nodes: Vec<Node>,
    /// The `#document` node.
    pub root: NodeId,
    /// Marqué par toute mutation structurelle/attributaire — l'embedder
    /// lit (et efface) le flag pour savoir si le rendu doit être
    /// resynchronisé avec le miroir (JS-5b).
    dirty: Cell<bool>,
}

impl Default for Dom {
    fn default() -> Self {
        Self::new()
    }
}

impl Dom {
    /// Empty document (`#document` root only).
    #[must_use]
    pub fn new() -> Self {
        let mut d = Dom {
            nodes: Vec::new(),
            root: 0,
            dirty: Cell::new(false),
        };
        d.root = d.alloc(NodeKind::Element, "#document");
        d
    }

    /// Parse a full document fragment under the root.
    #[must_use]
    pub fn from_html(html: &str) -> Self {
        let mut d = Self::new();
        d.parse_fragment_into(d.root, html);
        d
    }

    fn alloc(&mut self, kind: NodeKind, tag: &str) -> NodeId {
        self.nodes.push(Node {
            kind,
            tag: tag.to_lowercase(),
            attrs: Vec::new(),
            children: Vec::new(),
            parent: None,
            text: String::new(),
            listeners: Vec::new(),
        });
        (self.nodes.len() - 1) as NodeId
    }

    /// Detached element (`document.createElement`).
    pub fn create_element(&mut self, tag: &str) -> NodeId {
        self.alloc(NodeKind::Element, tag)
    }

    /// Detached text node (`document.createTextNode`).
    pub fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.alloc(NodeKind::Text, "#text");
        self.nodes[id as usize].text = text.to_string();
        id
    }

    /// Dirty = mutation depuis la dernière synchro de rendu.
    pub fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    /// Lit et efface le flag de mutation (≙ « dirty bit » des toolkits).
    pub fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }

    /// Sérialisation du document entier (source de re-rendu, JS-5b).
    #[must_use]
    pub fn serialize_document(&self) -> String {
        self.inner_html(self.root)
    }

    /// Attach `child` under `parent` (re-parents if attached).
    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        self.nodes[child as usize].parent = Some(parent);
        self.nodes[parent as usize].children.push(child);
        self.dirty.set(true);
    }

    /// Detach `node` from its parent (keeps the node alive).
    pub fn detach(&mut self, node: NodeId) {
        if let Some(p) = self.nodes[node as usize].parent {
            self.nodes[p as usize].children.retain(|&c| c != node);
            self.dirty.set(true);
            self.nodes[node as usize].parent = None;
        }
    }

    /// Replace all children of `node` with `kids` (innerHTML setter).
    pub fn replace_children(&mut self, node: NodeId, kids: &[NodeId]) {
        let old = std::mem::take(&mut self.nodes[node as usize].children);
        for c in old {
            self.nodes[c as usize].parent = None;
        }
        for &k in kids {
            self.append(node, k);
        }
        self.dirty.set(true);
    }

    /// All children — elements *and* text (`childNodes`; `children`
    /// is elements-only, see [`Dom::element_children`]).
    #[must_use]
    pub fn child_nodes(&self, node: NodeId) -> Vec<NodeId> {
        self.nodes[node as usize].children.clone()
    }

    /// `parentNode` — `None` for the root or detached nodes.
    #[must_use]
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.nodes[node as usize].parent
    }

    /// Next (`true`) or previous (`false`) sibling, any kind —
    /// `None` when detached or at the edge.
    #[must_use]
    pub fn sibling(&self, node: NodeId, next: bool) -> Option<NodeId> {
        let p = self.nodes[node as usize].parent?;
        let kids = &self.nodes[p as usize].children;
        let i = kids.iter().position(|&c| c == node)?;
        let j = if next { i + 1 } else { i.checked_sub(1)? };
        kids.get(j).copied()
    }

    /// `insertBefore(child, ref)` — `None` ref ≙ append. If `ref`
    /// isn't a child of `parent` the insert degrades to append
    /// (tolerant — Dillo never throws on malformed trees either).
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, ref_node: Option<NodeId>) {
        self.detach(child);
        let idx = ref_node.and_then(|r| {
            self.nodes[parent as usize]
                .children
                .iter()
                .position(|&c| c == r)
        });
        match idx {
            Some(i) => self.nodes[parent as usize].children.insert(i, child),
            None => self.nodes[parent as usize].children.push(child),
        }
        self.nodes[child as usize].parent = Some(parent);
        self.dirty.set(true);
    }

    /// `replaceChild(new, old)` — `old` is detached, `new` takes
    /// its slot. Returns `old` (spec) — caller wraps it.
    pub fn replace_child(&mut self, parent: NodeId, new: NodeId, old: NodeId) {
        let idx = self.nodes[parent as usize]
            .children
            .iter()
            .position(|&c| c == old);
        if let Some(i) = idx {
            self.detach(new);
            self.nodes[parent as usize].children[i] = new;
            self.nodes[new as usize].parent = Some(parent);
            self.nodes[old as usize].parent = None;
            self.dirty.set(true);
        }
    }

    /// `cloneNode(deep)` — fresh ids, listeners are *not* cloned
    /// (spec: `addEventListener` handlers don't survive cloning).
    pub fn clone_node(&mut self, node: NodeId, deep: bool) -> NodeId {
        let (kind, tag, attrs, text, kids) = {
            let n = &self.nodes[node as usize];
            (
                n.kind,
                n.tag.clone(),
                n.attrs.clone(),
                n.text.clone(),
                n.children.clone(),
            )
        };
        let id = self.alloc(kind, &tag);
        self.nodes[id as usize].attrs = attrs;
        self.nodes[id as usize].text = text;
        if deep {
            for c in kids {
                let cc = self.clone_node(c, true);
                self.append(id, cc);
            }
        }
        id
    }

    /// `a.contains(b)` — `b` is `a` itself or a descendant.
    #[must_use]
    pub fn contains(&self, a: NodeId, b: NodeId) -> bool {
        a == b || self.ancestors(b).contains(&a)
    }

    /// `el.matches(sel)` — single-node selector test (same subset
    /// as [`Dom::query`]: simple selectors + descendant combinator).
    #[must_use]
    pub fn matches(&self, node: NodeId, sel: &str) -> bool {
        let parts: Vec<&str> = sel.split_whitespace().collect();
        let Some((&last, prefix)) = parts.split_last() else {
            return false;
        };
        let Some((tag, id, class)) = parse_simple_selector(last) else {
            return false;
        };
        self.node_matches(node, tag.as_deref(), id.as_deref(), class.as_deref())
            && (prefix.is_empty() || self.ancestor_matches(self.root, node, prefix))
    }

    /// `el.closest(sel)` — nearest ancestor-or-self matching `sel`.
    #[must_use]
    pub fn closest(&self, node: NodeId, sel: &str) -> Option<NodeId> {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if self.matches(n, sel) {
                return Some(n);
            }
            cur = self.parent(n);
        }
        None
    }

    /// `outerHTML` — serialize the node itself (inner + its own tag).
    #[must_use]
    pub fn outer_html(&self, node: NodeId) -> String {
        let mut out = String::new();
        self.serialize(node, &mut out);
        out
    }

    // -- attributes -----------------------------------------------------

    /// `getAttribute` (first occurrence, case-insensitive name).
    #[must_use]
    pub fn attr(&self, node: NodeId, name: &str) -> Option<&str> {
        self.nodes[node as usize]
            .attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `setAttribute`.
    pub fn set_attr(&mut self, node: NodeId, name: &str, value: &str) {
        let attrs = &mut self.nodes[node as usize].attrs;
        if let Some((_, v)) = attrs.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            *v = value.to_string();
        } else {
            attrs.push((name.to_lowercase(), value.to_string()));
        }
        self.dirty.set(true);
    }

    /// `removeAttribute`. Returns whether it existed.
    pub fn remove_attr(&mut self, node: NodeId, name: &str) -> bool {
        let attrs = &mut self.nodes[node as usize].attrs;
        let before = attrs.len();
        attrs.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        if attrs.len() != before {
            self.dirty.set(true);
        }
        attrs.len() != before
    }

    /// `hasAttribute`.
    #[must_use]
    pub fn has_attr(&self, node: NodeId, name: &str) -> bool {
        self.attr(node, name).is_some()
    }

    // -- queries ----------------------------------------------------------

    /// `document.body` — first `<body>` descendant, else the root.
    #[must_use]
    pub fn body(&self) -> NodeId {
        self.by_tag(self.root, "body")
            .into_iter()
            .next()
            .unwrap_or(self.root)
    }

    /// `document.documentElement` — first element child of the root.
    #[must_use]
    pub fn document_element(&self) -> Option<NodeId> {
        self.element_children(self.root).into_iter().next()
    }

    /// Ancestor chain of `node`, parent first, `#document` root last
    /// (empty for the root itself).
    #[must_use]
    pub fn ancestors(&self, node: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.nodes[node as usize].parent;
        while let Some(p) = cur {
            out.push(p);
            cur = self.nodes[p as usize].parent;
        }
        out
    }

    /// Element children of `node` (text nodes filtered out).
    #[must_use]
    pub fn element_children(&self, node: NodeId) -> Vec<NodeId> {
        self.nodes[node as usize]
            .children
            .iter()
            .copied()
            .filter(|&c| self.nodes[c as usize].kind == NodeKind::Element)
            .collect()
    }

    /// `getElementById` — first match, document order.
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .position(|n| n.kind == NodeKind::Element && Self::attr_val(n, "id") == Some(id))
            .map(|i| i as NodeId)
    }

    fn attr_val<'a>(n: &'a Node, name: &str) -> Option<&'a str> {
        n.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `getElementsByTagName` — deep, document order, `"*"` matches
    /// every element.
    #[must_use]
    pub fn by_tag(&self, from: NodeId, tag: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_tag(from, &tag.to_lowercase(), &mut out);
        out
    }

    fn collect_tag(&self, node: NodeId, tag: &str, out: &mut Vec<NodeId>) {
        for &c in &self.nodes[node as usize].children {
            let n = &self.nodes[c as usize];
            if n.kind == NodeKind::Element && (tag == "*" || n.tag == tag) {
                out.push(c);
            }
            self.collect_tag(c, tag, out);
        }
    }

    /// `getElementsByClassName` — token-set membership.
    #[must_use]
    pub fn by_class(&self, from: NodeId, class: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_class(from, class, &mut out);
        out
    }

    fn collect_class(&self, node: NodeId, class: &str, out: &mut Vec<NodeId>) {
        for &c in &self.nodes[node as usize].children {
            let n = &self.nodes[c as usize];
            if n.kind == NodeKind::Element
                && Self::attr_val(n, "class")
                    .is_some_and(|v| v.split_whitespace().any(|t| t == class))
            {
                out.push(c);
            }
            self.collect_class(c, class, out);
        }
    }

    /// Minimal `querySelector(All)`: single-token selectors —
    /// `tag`, `#id`, `.class`, `tag.class`, `tag#id`, `*`, plus the
    /// descendant combinator (`a b`). `None`/`empty` for anything
    /// more complex (documented subset).
    #[must_use]
    pub fn query(&self, from: NodeId, sel: &str) -> Vec<NodeId> {
        let parts: Vec<&str> = sel.split_whitespace().collect();
        let Some(last) = parts.last() else {
            return Vec::new();
        };
        let Some((tag, id, class)) = parse_simple_selector(last) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.collect_query(
            from,
            tag.as_deref(),
            id.as_deref(),
            class.as_deref(),
            &mut out,
        );
        // Descendant combinator: keep only nodes whose ancestors match
        // the left-to-right selector chain (bounded by `from`).
        if parts.len() > 1 {
            let prefix = &parts[..parts.len() - 1];
            out.retain(|&n| self.ancestor_matches(from, n, prefix));
        }
        out
    }

    /// Does some ancestor chain of `node` (within `from`) match the
    /// selector `parts` right-to-left? Used by the descendant
    /// combinator and by `closest` on compound selectors.
    fn ancestor_matches(&self, bound: NodeId, node: NodeId, parts: &[&str]) -> bool {
        let Some((&last, rest)) = parts.split_last() else {
            return true;
        };
        let Some((tag, id, class)) = parse_simple_selector(last) else {
            return false;
        };
        let mut cur = self.parent(node);
        while let Some(a) = cur {
            if a == bound {
                break;
            }
            if self.node_matches(a, tag.as_deref(), id.as_deref(), class.as_deref())
                && self.ancestor_matches(bound, a, rest)
            {
                return true;
            }
            cur = self.parent(a);
        }
        false
    }

    fn collect_query(
        &self,
        node: NodeId,
        tag: Option<&str>,
        id: Option<&str>,
        class: Option<&str>,
        out: &mut Vec<NodeId>,
    ) {
        for &c in &self.nodes[node as usize].children {
            if self.node_matches(c, tag, id, class) {
                out.push(c);
            }
            self.collect_query(c, tag, id, class, out);
        }
    }

    /// Single-node selector test — `matches()` and `collect_query`
    /// share the same simple-selector subset.
    fn node_matches(
        &self,
        node: NodeId,
        tag: Option<&str>,
        id: Option<&str>,
        class: Option<&str>,
    ) -> bool {
        let n = &self.nodes[node as usize];
        if n.kind != NodeKind::Element {
            return false;
        }
        let tag_ok = tag.is_none_or(|t| n.tag == t);
        let id_ok = id.is_none_or(|i| Self::attr_val(n, "id") == Some(i));
        let class_ok = class.is_none_or(|cls| {
            Self::attr_val(n, "class").is_some_and(|v| v.split_whitespace().any(|t| t == cls))
        });
        tag_ok && id_ok && class_ok
    }

    // -- text & serialization ---------------------------------------------

    /// `textContent` — concatenated descendant text.
    #[must_use]
    pub fn text_content(&self, node: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(node, &mut out);
        out
    }

    fn collect_text(&self, node: NodeId, out: &mut String) {
        let n = &self.nodes[node as usize];
        if n.kind == NodeKind::Text {
            out.push_str(&n.text);
        }
        for &c in &n.children {
            self.collect_text(c, out);
        }
    }

    /// `innerHTML` getter — serialize children.
    #[must_use]
    pub fn inner_html(&self, node: NodeId) -> String {
        let mut out = String::new();
        for &c in &self.nodes[node as usize].children {
            self.serialize(c, &mut out);
        }
        out
    }

    fn serialize(&self, node: NodeId, out: &mut String) {
        let n = &self.nodes[node as usize];
        if n.kind == NodeKind::Text {
            out.push_str(&escape_text(&n.text));
            return;
        }
        out.push('<');
        out.push_str(&n.tag);
        for (k, v) in &n.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(&escape_attr(v));
            out.push('"');
        }
        out.push('>');
        if VOID_TAGS.contains(&n.tag.as_str()) {
            return;
        }
        for &c in &n.children {
            self.serialize(c, out);
        }
        out.push_str("</");
        out.push_str(&n.tag);
        out.push('>');
    }

    // -- fragment parsing ---------------------------------------------------

    /// Parse `html` as a fragment and attach it under `parent`.
    /// Tolerant by design: unknown entities pass through, unclosed
    /// tags simply stay open, `</x>` pops to the nearest open `x`.
    pub fn parse_fragment_into(&mut self, parent: NodeId, html: &str) {
        let mut stack = vec![parent];
        let cs: Vec<char> = html.chars().collect();
        let mut i = 0;
        let mut text = String::new();
        macro_rules! flush_text {
            () => {
                if !text.is_empty() {
                    let t = self.create_text(&text);
                    let top = *stack.last().unwrap_or(&parent);
                    self.append(top, t);
                    text.clear();
                }
            };
        }
        while i < cs.len() {
            if cs[i] != '<' {
                text.push(cs[i]);
                i += 1;
                continue;
            }
            // Comments / doctype / PIs.
            if cs[i + 1..].starts_with(&['!', '-', '-']) {
                i += 4;
                while i + 2 < cs.len() && !(cs[i] == '-' && cs[i + 1] == '-' && cs[i + 2] == '>') {
                    i += 1;
                }
                i = (i + 3).min(cs.len());
                continue;
            }
            if matches!(cs.get(i + 1), Some('!' | '?')) {
                while i < cs.len() && cs[i] != '>' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            if cs.get(i + 1) == Some(&'/') {
                // Close tag: pop to matching element if open.
                flush_text!();
                i += 2;
                let name_start = i;
                while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '-') {
                    i += 1;
                }
                let name: String = cs[name_start..i].iter().collect::<String>().to_lowercase();
                while i < cs.len() && cs[i] != '>' {
                    i += 1;
                }
                i += 1;
                if name.is_empty() {
                    continue;
                }
                if let Some(pos) = stack
                    .iter()
                    .rposition(|&id| self.nodes[id as usize].tag == name)
                {
                    // Pop through the match, but never the fragment parent.
                    if pos > 0 {
                        stack.truncate(pos);
                    }
                }
                continue;
            }
            // Start tag.
            flush_text!();
            i += 1;
            let name_start = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '-' || cs[i] == ':') {
                i += 1;
            }
            if i == name_start {
                text.push('<'); // stray '<' — literal text
                continue;
            }
            let tag: String = cs[name_start..i].iter().collect::<String>().to_lowercase();
            // Attributes.
            let mut attrs: Vec<(String, String)> = Vec::new();
            let mut self_close = false;
            loop {
                while i < cs.len() && cs[i].is_whitespace() {
                    i += 1;
                }
                match cs.get(i) {
                    None | Some('>') => break,
                    Some('/') => {
                        i += 1;
                        if cs.get(i) == Some(&'>') {
                            self_close = true;
                            i += 1;
                            break;
                        }
                    }
                    _ => {
                        let astart = i;
                        while i < cs.len()
                            && !matches!(cs[i], '=' | '>' | '/')
                            && !cs[i].is_whitespace()
                        {
                            i += 1;
                        }
                        let aname: String = cs[astart..i].iter().collect::<String>().to_lowercase();
                        while i < cs.len() && cs[i].is_whitespace() {
                            i += 1;
                        }
                        let mut aval = String::new();
                        if cs.get(i) == Some(&'=') {
                            i += 1;
                            while i < cs.len() && cs[i].is_whitespace() {
                                i += 1;
                            }
                            if let Some(&q @ ('"' | '\'')) = cs.get(i) {
                                i += 1;
                                let vstart = i;
                                while i < cs.len() && cs[i] != q {
                                    i += 1;
                                }
                                aval = cs[vstart..i].iter().collect();
                                i += 1;
                            } else {
                                let vstart = i;
                                while i < cs.len()
                                    && !matches!(cs[i], '>')
                                    && !cs[i].is_whitespace()
                                {
                                    i += 1;
                                }
                                aval = cs[vstart..i].iter().collect();
                            }
                        }
                        if !aname.is_empty() {
                            attrs.push((aname, decode_entities(&aval)));
                        }
                    }
                }
            }
            if i < cs.len() && cs[i] == '>' {
                i += 1;
            }
            if tag.is_empty() {
                continue;
            }
            // Minimal implicit-close tolerance (li/p/tr/td/option).
            if IMPLICIT_CLOSE.contains(&tag.as_str()) {
                if let Some(&top) = stack.last() {
                    if self.nodes[top as usize].tag == tag {
                        stack.pop();
                    }
                }
            }
            let id = self.alloc(NodeKind::Element, &tag);
            self.nodes[id as usize].attrs = attrs;
            let top = *stack.last().unwrap_or(&parent);
            self.append(top, id);
            if !self_close && !VOID_TAGS.contains(&tag.as_str()) {
                if RAW_TEXT.contains(&tag.as_str()) {
                    // script/style/textarea: raw text until `</tag`.
                    let close = format!("</{tag}");
                    let rest: String = cs[i..].iter().collect();
                    let pos = rest.to_lowercase().find(&close);
                    let (inner, consumed) = match pos {
                        Some(p) => (&rest[..p], p + close.len()),
                        None => (&rest[..], rest.len()),
                    };
                    if !inner.is_empty() {
                        let t = self.create_text(inner);
                        self.append(id, t);
                    }
                    i += consumed;
                    // Skip to `>` past the close tag.
                    while i < cs.len() && cs[i] != '>' {
                        i += 1;
                    }
                    i += 1;
                    continue; // don't push — already closed
                }
                stack.push(id);
            }
        }
        flush_text!();
    }
}

/// HTML void elements.
const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Raw-text elements (content is text until matching close tag).
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];

/// Tags that implicitly close a same-tag sibling.
const IMPLICIT_CLOSE: &[&str] = &["li", "p", "tr", "td", "th", "option", "dt", "dd"];

/// `tag` / `#id` / `.class` / `tag.class` / `tag#id` / `*` → parts.
fn parse_simple_selector(sel: &str) -> Option<(Option<String>, Option<String>, Option<String>)> {
    if sel.is_empty() || sel.contains(' ') || sel.contains(',') || sel.contains('>') {
        return None;
    }
    let (mut tag, mut id, mut class) = (None, None, None);
    let mut rest = sel;
    if let Some((h, r)) = rest.split_once('#') {
        if !h.is_empty() {
            tag = Some(h.to_lowercase());
        }
        rest = r;
        let (i, r2) = match rest.split_once('.') {
            Some((a, b)) => (a, b),
            None => (rest, ""),
        };
        if !i.is_empty() {
            id = Some(i.to_string());
        }
        rest = r2;
        if !rest.is_empty() {
            class = Some(rest.to_string());
            rest = "";
        }
    } else if let Some((h, r)) = rest.split_once('.') {
        if !h.is_empty() {
            tag = Some(h.to_lowercase());
        }
        if !r.is_empty() {
            class = Some(r.to_string());
        }
        rest = "";
    }
    if !rest.is_empty() {
        tag = Some(rest.to_lowercase());
    }
    if tag.is_none() && id.is_none() && class.is_none() {
        return None;
    }
    if tag.as_deref() == Some("*") {
        tag = None;
    }
    Some((tag, id, class))
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

/// Decode the common entities (mini-parser, not the full table —
/// unknown entities pass through verbatim).
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').map_or(0, |e| e + 1);
        if end == 0 || end > 12 {
            out.push('&');
            rest = &rest[1..];
            continue;
        }
        let (entity, tail) = rest.split_at(end);
        rest = tail;
        out.push_str(match entity {
            "&amp;" => "&",
            "&lt;" => "<",
            "&gt;" => ">",
            "&quot;" => "\"",
            "&apos;" => "'",
            "&nbsp;" => "\u{a0}",
            _ => {
                if let Some(num) = entity.strip_prefix("&#").and_then(|n| n.strip_suffix(';')) {
                    match num.parse::<u32>().ok().and_then(char::from_u32) {
                        Some(c) => {
                            out.push(c);
                            continue;
                        }
                        None => entity,
                    }
                } else {
                    entity
                }
            }
        });
    }
    out.push_str(rest);
    out
}
