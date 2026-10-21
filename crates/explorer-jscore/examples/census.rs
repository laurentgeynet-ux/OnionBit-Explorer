//! `census` — fait passer une page HTML sauvegardée dans le moteur
//! et dresse le compte-rendu des erreurs de scripts (quels APIs /
//! quelle syntaxe manquent réellement). Outil de mesure pilotant la
//! couverture — jamais un test : la sortie est un rapport, le code
//! de sortie reste 0.
//!
//! Usage : `cargo run -p explorer-jscore --example census -- <page.html> [--extra x.js …] [--dump-scripts dir]`
//!
//! - `<script src>` sont listés (pas fetchés — le crate est
//!   autonome) ; `--extra` évalue un fichier JS local à la fin,
//!   pour rejouer un bundle externe sauvegardé à la main.
//! - `--probe <js>` évalue un snippet après tout le reste et
//!   affiche sa valeur — diagnostic de l'état global final.
//! - `--dump-scripts <dir>` écrit chaque script inline dans
//!   `<dir>/script_<i>.js` pour inspection/rejeu.
//! - XHR/fetch sont installés avec un transport qui **enregistre**
//!   les requêtes sans jamais les livrer (pas de réseau ici) : le
//!   rapport dit ce que la page aurait voulu fetcher.
//! - Les erreurs sont agrégées par signature normalisée (les
//!   messages identiques comptent une fois, triés par fréquence).

use explorer_jscore::{
    CookieJar, Dom, DomRef, Engine, FormSubmit, Storage, XhrRequest, XhrTransport,
};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// Jar mémoire minimal (le census ne teste pas la politique cookies).
struct MemJar {
    map: RefCell<BTreeMap<String, String>>,
}

impl CookieJar for MemJar {
    fn get(&self, _url: &str) -> String {
        self.map
            .borrow()
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn set(&self, _url: &str, cookie: &str) {
        let kv = cookie.split(';').next().unwrap_or("").trim();
        let Some((k, v)) = kv.split_once('=') else {
            return;
        };
        if v.is_empty() {
            self.map.borrow_mut().remove(k);
        } else {
            self.map.borrow_mut().insert(k.to_string(), v.to_string());
        }
    }
}

/// Transport XHR de census : les requêtes sont **enregistrées**
/// (elles disent ce que la page veut fetcher) mais jamais livrées —
/// une requête en vol sans réseau est le comportement honnête.
struct DumpTransport {
    requests: Rc<RefCell<Vec<(String, String)>>>,
    next: Cell<u64>,
}

impl XhrTransport for DumpTransport {
    fn send(&self, req: &XhrRequest) -> u64 {
        let h = self.next.replace(self.next.get() + 1);
        self.requests
            .borrow_mut()
            .push((req.method.clone(), req.url.clone()));
        h
    }

    fn abort(&self, _handle: u64) {}
}

/// Signature normalisée d'une erreur : type + tête du message,
/// suffisante pour grouper « `xxx` is not defined » par nom.
fn signature(err: &str) -> String {
    let head: String = err.chars().take(160).collect();
    head.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn eval(engine: &mut Engine, code: &str, label: &str, counts: &mut BTreeMap<String, usize>) {
    match engine.eval(code) {
        Ok(_) => println!("{label}: ok"),
        Err(e) => {
            let s = e.to_string();
            *counts.entry(signature(&s)).or_default() += 1;
            println!("{label}: {s}");
        }
    }
}

#[allow(clippy::too_many_lines)]
fn main() {
    // Même filtre `type` que `collect_scripts` côté core :
    // seuls les types exécutables comptent — `speculationrules`,
    // `importmap`, `application/json`… sont des données, pas du JS.
    const CLASSIC_TYPES: &[&str] = &[
        "",
        "text/javascript",
        "application/javascript",
        "text/ecmascript",
        "application/ecmascript",
        "text/jscript",
    ];
    let mut path = None;
    let mut dump_dir = None;
    let mut extras = Vec::new();
    let mut probes = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dump-scripts" => dump_dir = args.next(),
            "--extra" => {
                if let Some(p) = args.next() {
                    extras.push(p);
                }
            }
            "--probe" => {
                if let Some(p) = args.next() {
                    probes.push(p);
                }
            }
            _ if path.is_none() => path = Some(a),
            _ => eprintln!("census: arg ignoré: {a}"),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: census <page.html> [--extra x.js] [--dump-scripts dir]");
        std::process::exit(2);
    };

    let html = match std::fs::read_to_string(&path) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("census: cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    println!("page: {path} ({} bytes)", html.len());

    // -- hôte : même surface que l'embedder navigateur ----------
    let dom: DomRef = Rc::new(RefCell::new(Dom::from_html(&html)));
    dom.borrow().take_dirty(); // la construction n'est pas une mutation

    let mut engine = Engine::new();
    // Pages réelles = scripts denses : le budget d'un `while(1){}`
    // hostile reste borné, les scripts légitimes passent.
    engine.set_op_limit(20_000_000);
    engine.install_timers();
    engine.set_page_url("https://www.google.com/");
    engine.set_console_hook(Rc::new(|line| println!("[console] {line}")));
    engine.set_dialog_hooks(
        Rc::new(|msg| println!("[alert] {msg}")),
        Rc::new(|msg| {
            println!("[confirm] {msg}");
            false
        }),
        Rc::new(|msg, _def| {
            println!("[prompt] {msg}");
            None
        }),
    );
    engine.set_nav_hooks(
        Rc::new(|t| println!("[navigate] {t}")),
        Rc::new(|n| println!("[history.go] {n}")),
        1,
    );
    engine.set_cookie_jar(Rc::new(MemJar {
        map: RefCell::new(BTreeMap::new()),
    }));
    let store: Storage = Rc::new(RefCell::new(BTreeMap::new()));
    engine.install_storage(&store);
    engine.set_viewport(1366, 768);
    engine.set_form_submit_hook(Rc::new(|f: FormSubmit| {
        println!("[form-submit] {} {}", f.method, f.action);
    }));
    engine.set_open_hook(Rc::new(|u| println!("[window.open] {u}")));
    let xhr_requests: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    engine.install_xhr(Rc::new(DumpTransport {
        requests: Rc::clone(&xhr_requests),
        next: Cell::new(1),
    }));
    engine.install_fetch();
    engine.install_dom(&dom);

    // Sources eval()ée/`Function` — le code obfusqué réel vit là
    // (positions des frames d'erreur). Dump vers <dir>/eval_<n>.js.
    if let Some(dir) = &dump_dir {
        let dir = dir.clone();
        let n = std::rc::Rc::new(std::cell::Cell::new(0usize));
        engine.set_eval_hook(Some(std::rc::Rc::new(move |src| {
            let i = n.get();
            n.set(i + 1);
            let file = format!("{dir}/eval_{i}.js");
            if let Err(e) = std::fs::write(&file, src) {
                eprintln!("census: cannot write {file}: {e}");
            }
        })));
    }

    // -- scripts dans l'ordre du document ------------------------
    if let Some(dir) = &dump_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("census: cannot create {dir}: {e}");
        }
    }
    let scripts = {
        let d = dom.borrow();
        d.by_tag(d.root, "script")
    };
    let (mut inline, mut external) = (0usize, 0usize);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();

    for (i, node) in scripts.iter().enumerate() {
        let (src, body, ty, nomodule) = {
            let d = dom.borrow();
            (
                d.attr(*node, "src").map(str::to_string),
                d.text_content(*node),
                d.attr(*node, "type")
                    .map(|t| t.trim().to_ascii_lowercase())
                    .unwrap_or_default(),
                d.attr(*node, "nomodule").is_some(),
            )
        };
        let module = ty == "module";
        if ty == "importmap" {
            // JS-C44 : enregistrée dans l'ordre du document — un
            // module qui la précède ne la verra pas (spec).
            engine.set_import_map(&body);
            println!("[{i}] importmap enregistrée ({}B)", body.len());
            continue;
        }
        if !(module || CLASSIC_TYPES.contains(&ty.as_str())) {
            println!("[{i}] SKIP type=\"{ty}\"");
            *skipped.entry(ty).or_default() += 1;
            continue;
        }
        if nomodule && !module {
            *skipped.entry("nomodule".to_string()).or_default() += 1;
            println!("[{i}] SKIP nomodule");
            continue;
        }
        if let Some(s) = src {
            external += 1;
            let short: String = s.chars().take(120).collect();
            let kind = if module { "MODULE " } else { "" };
            println!("[{i}] {kind}EXTERNAL {short}");
        } else {
            inline += 1;
            if let Some(dir) = &dump_dir {
                let file = format!("{dir}/script_{i}.js");
                if let Err(e) = std::fs::write(&file, &body) {
                    eprintln!("census: cannot write {file}: {e}");
                }
            }
            let label = format!("[{i}] inline {}B", body.len());
            if module {
                match engine.eval_module(&body, &format!("script_{i}")) {
                    Ok(_) => println!("{label}: ok"),
                    Err(e) => {
                        let s = e.to_string();
                        *counts.entry(signature(&s)).or_default() += 1;
                        println!("{label}: {s}");
                    }
                }
            } else {
                eval(&mut engine, &body, &label, &mut counts);
            }
        }
    }

    for path in &extras {
        match std::fs::read_to_string(path) {
            Ok(code) => {
                let label = format!("[extra {path}] {}B", code.len());
                eval(&mut engine, &code, &label, &mut counts);
            }
            Err(e) => eprintln!("census: cannot read {path}: {e}"),
        }
    }

    // -- timers : quelques passes pour les callbacks ------------
    for round in 0..3 {
        match engine.run_due_timers() {
            Ok(0) => break,
            Ok(n) => println!("[timers round {round}] {n} callbacks"),
            Err(t) => {
                let s = format!("timer threw: {}", engine.describe(t.0));
                *counts.entry(signature(&s)).or_default() += 1;
                println!("{s}");
                break;
            }
        }
    }

    // -- événements de chargement --------------------------------
    for ty in ["DOMContentLoaded", "load"] {
        match engine.dom_event(ty) {
            Ok(_) => println!("[event] {ty} dispatched"),
            Err(t) => {
                let s = format!("{ty} handler threw: {}", engine.describe(t.0));
                *counts.entry(signature(&s)).or_default() += 1;
                println!("{s}");
            }
        }
    }

    // -- sondes de diagnostic sur l'état global final -----------
    for p in &probes {
        match engine.eval(p) {
            Ok(v) => println!("[probe] {p} => {}", engine.describe(v)),
            Err(e) => println!("[probe] {p} => THROW {e}"),
        }
    }

    let reqs = xhr_requests.borrow();
    if !reqs.is_empty() {
        println!("\n=== {} requêtes XHR/fetch en vol ===", reqs.len());
        for (m, u) in reqs.iter().take(15) {
            let short: String = u.chars().take(140).collect();
            println!("  {m} {short}");
        }
    }
    drop(reqs);

    println!("\n=== census: {inline} inline / {external} external ===");
    if !skipped.is_empty() {
        let detail: Vec<String> = skipped
            .iter()
            .map(|(t, n)| format!("\"{t}\"×{n}"))
            .collect();
        println!("skipped (type non exécutable) : {}", detail.join(", "));
    }
    let mut sorted: Vec<(usize, String)> = counts.iter().map(|(s, n)| (*n, s.clone())).collect();
    sorted.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    for (n, sig) in sorted.iter().take(25) {
        println!("{n:>4}× {sig}");
    }
}
