// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du tree-builder HTML5 : structure de document, fermetures
//! implicites, éléments de format, tables/foster parenting,
//! rawtext/RCDATA, quirks — dump texte façon html5lib-tests.

use std::fmt::Write as _;

use explorer_dom::{Dom, NodeData, NodeId};
use explorer_html::{QuirksMode, TreeBuilder};

/// Dump indenté de l'arbre — format inspiré de html5lib-tests
/// (`#document`, `|   <tag>`, `|   "text"`, `|   <!-- c -->`).
fn dump(dom: &Dom) -> String {
    let mut out = String::from("#document\n");
    dump_children(dom, dom.document(), 1, &mut out);
    out
}

fn dump_children(dom: &Dom, parent: NodeId, depth: usize, out: &mut String) {
    for child in dom.children(parent).expect("children") {
        out.push_str("| ");
        for _ in 1..depth {
            out.push_str("  ");
        }
        match dom.data(child).expect("data") {
            NodeData::Document => out.push_str("#document-nested"),
            NodeData::Doctype(d) => {
                let _ = write!(out, "<!DOCTYPE {}>", d.name);
            }
            NodeData::Element(e) => {
                let _ = write!(out, "<{}>", e.tag_name);
                for a in &e.attributes {
                    out.push('\n');
                    out.push_str("| ");
                    for _ in 0..depth {
                        out.push_str("  ");
                    }
                    let _ = write!(out, "{}=\"{}\"", a.name, a.value);
                }
            }
            NodeData::Text(t) => {
                let _ = write!(out, "\"{t}\"");
            }
            NodeData::Comment(c) => {
                let _ = write!(out, "<!-- {c} -->");
            }
        }
        out.push('\n');
        dump_children(dom, child, depth + 1, out);
    }
}

fn parse(input: &str) -> Dom {
    TreeBuilder::parse(input)
}

// -- Structure de base ----------------------------------------------------

#[test]
fn empty_document_synthesizes_html_head_body() {
    let d = parse("");
    assert_eq!(
        dump(&d),
        "#document\n\
         | <html>\n\
         |   <head>\n\
         |   <body>\n"
    );
}

#[test]
fn simple_paragraph() {
    let d = parse("<p>hi");
    assert_eq!(
        dump(&d),
        "#document\n\
         | <html>\n\
         |   <head>\n\
         |   <body>\n\
         |     <p>\n\
         |       \"hi\"\n"
    );
}

#[test]
fn full_document_with_attrs() {
    let d = parse(
        "<!DOCTYPE html><html><head><title>T</title></head>\
                  <body class=x><p id=p1>yo</p></body></html>",
    );
    assert_eq!(
        dump(&d),
        "#document\n\
         | <!DOCTYPE html>\n\
         | <html>\n\
         |   <head>\n\
         |     <title>\n\
         |       \"T\"\n\
         |   <body>\n\
         |     class=\"x\"\n\
         |     <p>\n\
         |       id=\"p1\"\n\
         |       \"yo\"\n"
    );
}

// -- Fermetures implicites --------------------------------------------------

#[test]
fn implicit_p_close() {
    let d = parse("<p>un<p>deux");
    assert_eq!(
        dump(&d),
        "#document\n\
         | <html>\n\
         |   <head>\n\
         |   <body>\n\
         |     <p>\n\
         |       \"un\"\n\
         |     <p>\n\
         |       \"deux\"\n"
    );
}

#[test]
fn implicit_li_close() {
    let d = parse("<ul><li>a<li>b<li>c</ul>");
    assert_eq!(
        dump(&d),
        "#document\n\
         | <html>\n\
         |   <head>\n\
         |   <body>\n\
         |     <ul>\n\
         |       <li>\n\
         |         \"a\"\n\
         |       <li>\n\
         |         \"b\"\n\
         |       <li>\n\
         |         \"c\"\n"
    );
}

#[test]
fn implicit_dt_dd_close() {
    let d = parse("<dl><dt>t<dd>d</dl>");
    assert_eq!(
        dump(&d),
        "#document\n\
         | <html>\n\
         |   <head>\n\
         |   <body>\n\
         |     <dl>\n\
         |       <dt>\n\
         |         \"t\"\n\
         |       <dd>\n\
         |         \"d\"\n"
    );
}

#[test]
fn heading_closes_p() {
    let d = parse("<p>text<h1>title</h1>");
    let body = body_of(&d);
    let kids = d.children(body).unwrap();
    let tags: Vec<_> = kids
        .iter()
        .map(|&k| d.tag_name(k).unwrap().to_string())
        .collect();
    assert_eq!(tags, ["p", "h1"]);
}

// -- Éléments de format ------------------------------------------------------

#[test]
fn formatting_nests_under_open_formatting() {
    // Spec : <p> s'insère dans le <b> encore ouvert — les éléments
    // de format ne sont PAS clos par un bloc.
    let d = parse("<b>gras<p>encore gras");
    let body = body_of(&d);
    let b = d.children(body).unwrap()[0];
    assert_eq!(d.tag_name(b).unwrap(), "b");
    let kids = d.children(b).unwrap();
    assert_eq!(d.text_content(kids[0]).unwrap(), "gras");
    let p = kids[1];
    assert_eq!(d.tag_name(p).unwrap(), "p");
    assert_eq!(d.text_content(p).unwrap(), "encore gras");
}

#[test]
fn afe_reconstruction_after_block_close() {
    // </div> dépile <b> (implied end tags n'inclut pas le format) ;
    // le caractère « y » déclenche la reconstruction AFE.
    let d = parse("<div><b>x</div>y");
    let body = body_of(&d);
    let kids = d.children(body).unwrap();
    assert_eq!(d.tag_name(kids[0]).unwrap(), "div");
    let inner_b = d.children(kids[0]).unwrap()[0];
    assert_eq!(d.tag_name(inner_b).unwrap(), "b");
    assert_eq!(d.text_content(inner_b).unwrap(), "x");
    // <b> reconstruit en frère du div pour « y ».
    let reopened = kids[1];
    assert_eq!(d.tag_name(reopened).unwrap(), "b");
    assert_eq!(d.text_content(reopened).unwrap(), "y");
}

#[test]
fn misnested_formatting_adoption() {
    // <b><i>x</b>y</i> — la spec ferme <b>, réinsère <i> pour y.
    let d = parse("<b><i>x</b>y</i>");
    let body = body_of(&d);
    let b = d.children(body).unwrap()[0];
    assert_eq!(d.tag_name(b).unwrap(), "b");
    let i = d.children(b).unwrap()[0];
    assert_eq!(d.tag_name(i).unwrap(), "i");
    assert_eq!(d.text_content(i).unwrap(), "x");
    // "y" se retrouve dans un <i> frère (comportement navigateur).
    let i2 = d.children(body).unwrap()[1];
    assert_eq!(d.tag_name(i2).unwrap(), "i");
    assert_eq!(d.text_content(i2).unwrap(), "y");
}

// -- Tables / foster parenting ------------------------------------------------

#[test]
fn table_structure() {
    let d = parse("<table><tr><td>c1<td>c2</table>");
    assert_eq!(
        dump(&d),
        "#document\n\
         | <html>\n\
         |   <head>\n\
         |   <body>\n\
         |     <table>\n\
         |       <tbody>\n\
         |         <tr>\n\
         |           <td>\n\
         |             \"c1\"\n\
         |           <td>\n\
         |             \"c2\"\n"
    );
}

#[test]
fn foster_parenting_diverts_text() {
    // Du texte directement dans <table> est déplacé AVANT la table.
    let d = parse("<table>avant<tr><td>in</table>");
    let body = body_of(&d);
    let kids = d.children(body).unwrap();
    // Le texte foster-parented précède le <table>.
    assert!(matches!(d.data(kids[0]).unwrap(), NodeData::Text(_)));
    assert_eq!(d.tag_name(kids[1]).unwrap(), "table");
}

#[test]
fn implicit_table_sections() {
    // <td> orphelin → table+tbody+tr synthétisés par le tree-builder.
    let d = parse("<table><td>x</table>");
    let body = body_of(&d);
    let table = d.children(body).unwrap()[0];
    let tbody = d.children(table).unwrap()[0];
    assert_eq!(d.tag_name(tbody).unwrap(), "tbody");
    let tr = d.children(tbody).unwrap()[0];
    assert_eq!(d.tag_name(tr).unwrap(), "tr");
    let td = d.children(tr).unwrap()[0];
    assert_eq!(d.tag_name(td).unwrap(), "td");
}

// -- Rawtext / RCDATA ----------------------------------------------------------

#[test]
fn script_in_head_is_rawtext() {
    // Premier élément → script va dans head (spec).
    let d = parse("<script>if (a<b) x()</script><p>after");
    let head = head_of(&d);
    let script = d.children(head).unwrap()[0];
    assert_eq!(d.tag_name(script).unwrap(), "script");
    assert_eq!(d.text_content(script).unwrap(), "if (a<b) x()");
}

#[test]
fn script_in_body_is_rawtext() {
    // Après ouverture du body → script au nœud courant (spec
    // « in-head rules » mais insertion sur place).
    let d = parse("<p>avant<script>if (a<b) x()</script>");
    let body = body_of(&d);
    let p = d.children(body).unwrap()[0];
    // <p> reste ouvert : script au nœud courant (spec).
    let script = d.children(p).unwrap()[1];
    assert_eq!(d.tag_name(script).unwrap(), "script");
    assert_eq!(d.text_content(script).unwrap(), "if (a<b) x()");
}

#[test]
fn style_is_rawtext() {
    let d = parse("<style>a > b { color: red }</style><p>x");
    let head = head_of(&d);
    let style = d.children(head).unwrap()[0];
    assert_eq!(d.text_content(style).unwrap(), "a > b { color: red }");
}

#[test]
fn title_is_rcdata() {
    // RCDATA : les balises sont du texte, mais &entité est décodée.
    let d = parse("<title>a<b>&amp;c</title>");
    let head = head_of(&d);
    let title = d.children(head).unwrap()[0];
    assert_eq!(d.text_content(title).unwrap(), "a<b>&c");
}

#[test]
fn textarea_skips_leading_newline() {
    let d = parse("<textarea>\nABC</textarea>");
    let body = body_of(&d);
    let ta = d.children(body).unwrap()[0];
    assert_eq!(d.text_content(ta).unwrap(), "ABC");
}

#[test]
fn pre_skips_leading_newline() {
    let d = parse("<pre>\nline</pre>");
    let body = body_of(&d);
    let pre = d.children(body).unwrap()[0];
    assert_eq!(d.text_content(pre).unwrap(), "line");
}

// -- Doctype / quirks --------------------------------------------------------

#[test]
fn quirks_modes() {
    let mut tb = TreeBuilder::new();
    feed(&mut tb, "<!DOCTYPE html><p>x");
    assert_eq!(tb.quirks_mode(), QuirksMode::NoQuirks);

    // Strict 4.01 sans system → NoQuirks (spec : seuls
    // Frameset/Transitional basculent sans system).
    let mut tb = TreeBuilder::new();
    feed(
        &mut tb,
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01//EN\"><p>x",
    );
    assert_eq!(tb.quirks_mode(), QuirksMode::NoQuirks);

    // 4.01 Transitional avec system → limited-quirks ; sans → quirks.
    let mut tb = TreeBuilder::new();
    feed(
        &mut tb,
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01 Transitional//EN\" \
         \"http://www.w3.org/TR/html4/loose.dtd\"><p>x",
    );
    assert_eq!(tb.quirks_mode(), QuirksMode::LimitedQuirks);
    let mut tb = TreeBuilder::new();
    feed(
        &mut tb,
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01 Transitional//EN\"><p>x",
    );
    assert_eq!(tb.quirks_mode(), QuirksMode::Quirks);

    // Sans doctype → quirks.
    let mut tb = TreeBuilder::new();
    feed(&mut tb, "<p>x");
    assert_eq!(tb.quirks_mode(), QuirksMode::Quirks);
}

// -- Commentaires / contenu parasite ------------------------------------------

#[test]
fn comment_before_html_is_document_child() {
    let d = parse("<!-- c --><p>x");
    let root = d.children(d.document()).unwrap();
    assert!(matches!(d.data(root[0]).unwrap(), NodeData::Comment(_)));
}

#[test]
fn content_after_body_still_lands_in_body() {
    let d = parse("<p>a</body><p>apres");
    let body = body_of(&d);
    let kids = d.children(body).unwrap();
    assert_eq!(kids.len(), 2);
    assert_eq!(d.text_content(kids[1]).unwrap(), "apres");
}

#[test]
fn unclosed_end_tag_ignored() {
    // </br> orphelin → traité comme <br> par la spec (ou ignoré).
    let d = parse("</div><p>x");
    let body = body_of(&d);
    assert_eq!(d.children(body).unwrap().len(), 1);
}

// -- Divers --------------------------------------------------------------------

#[test]
fn nested_divs() {
    let d = parse("<div><div><div>deep</div></div></div>");
    let body = body_of(&d);
    let d1 = d.children(body).unwrap()[0];
    let d2 = d.children(d1).unwrap()[0];
    let d3 = d.children(d2).unwrap()[0];
    assert_eq!(d.text_content(d3).unwrap(), "deep");
}

#[test]
fn select_option_and_optgroup() {
    let d = parse("<select><option>a<option>b</select>");
    let body = body_of(&d);
    let select = d.children(body).unwrap()[0];
    assert_eq!(d.tag_name(select).unwrap(), "select");
    let opts = d.children(select).unwrap();
    assert_eq!(opts.len(), 2);
    assert_eq!(d.text_content(opts[0]).unwrap(), "a");
    assert_eq!(d.text_content(opts[1]).unwrap(), "b");
}

#[test]
fn incremental_feed_matches_one_shot() {
    use explorer_html::Tokenizer;
    let html = "<!DOCTYPE html><p>un<b>deux</b>&amp;<table><tr><td>x</table>";
    let one_shot = dump(&parse(html));
    let mut tb = TreeBuilder::new();
    let mut t = Tokenizer::new();
    // Chunks coupés au milieu de balises et d'entités.
    for chunk in [
        "<!DOCTYPE h",
        "tml><p>un<b>deux</b>&a",
        "mp;<table><tr><td>x</table>",
    ] {
        t.feed(chunk);
        while let Some(tok) = t.next_token() {
            tb.process(&mut t, tok);
        }
    }
    t.finish();
    while let Some(tok) = t.next_token() {
        tb.process(&mut t, tok);
    }
    assert_eq!(dump(&tb.into_dom()), one_shot);
}

// -- Helpers --------------------------------------------------------------------

fn feed(tb: &mut TreeBuilder, input: &str) {
    use explorer_html::Tokenizer;
    let mut t = Tokenizer::on(input);
    tb.drive(&mut t);
}

fn body_of(d: &Dom) -> NodeId {
    let html = d.children(d.document()).unwrap()[0];
    d.children(html)
        .unwrap()
        .into_iter()
        .find(|&n| d.tag_name(n) == Ok("body"))
        .expect("body")
}

fn head_of(d: &Dom) -> NodeId {
    let html = d.children(d.document()).unwrap()[0];
    d.children(html)
        .unwrap()
        .into_iter()
        .find(|&n| d.tag_name(n) == Ok("head"))
        .expect("head")
}

// -- Bornes (étape 39) --------------------------------------------------------

#[test]
fn deep_nesting_is_capped() {
    // 600 `<div>` imbriqués dépassent MAX_DEPTH (512) : le
    // tree-builder ignore les tags excédentaires plutôt que de
    // produire un arbre qui dépasserait la pile d'appels des
    // traversées récursives aval (boxtree, display list).
    let input = "<div>".repeat(600);
    let d = parse(&input);
    let mut stack = vec![(d.document(), 0usize)];
    let mut max_depth = 0;
    let mut divs = 0usize;
    while let Some((n, dep)) = stack.pop() {
        max_depth = max_depth.max(dep);
        if d.tag_name(n) == Ok("div") {
            divs += 1;
        }
        for c in d.children(n).unwrap_or_default() {
            stack.push((c, dep + 1));
        }
    }
    // html/body + ~510 divs — marge pour les wrappers synthétisés.
    assert!(max_depth <= 520, "profondeur {max_depth}");
    assert!(divs < 600, "{divs} divs insérés");
    assert!(divs > 400, "{divs} divs insérés");
}
