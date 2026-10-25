use explorer_jscore::{Engine, ModuleLoader};
use std::cell::RefCell;
use std::rc::Rc;

struct L;
impl ModuleLoader for L {
    fn resolve(&mut self, spec: &str, referrer: &str) -> Option<String> {
        if spec.starts_with("http") {
            return Some(spec.to_string());
        }
        let base = referrer
            .rsplit_once('/')
            .map(|(b, _)| b)
            .unwrap_or(referrer);
        let mut path: Vec<&str> = base.split('/').collect();
        for seg in spec.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    path.pop();
                }
                s => path.push(s),
            }
        }
        Some(path.join("/"))
    }
    fn load(&mut self, spec: &str, _ref: &str) -> Result<String, String> {
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/probe_xcom");
        let known = [
            ("rolldown-runtime", "runtime.js"),
            ("startup-vendor", "vendor.js"),
            ("startup-D", "startup.js"),
            ("startup-dynamic", "dynamic.js"),
            ("entry-client", "entry.js"),
        ];
        for (pat, f) in known {
            if spec.contains(pat) {
                return std::fs::read_to_string(dir.join(f)).map_err(|e| format!("{f}:{e}"));
            }
        }
        let base = spec.rsplit('/').next().unwrap_or(spec);
        std::fs::read_to_string(dir.join(base)).map_err(|e| format!("{base}:{e}"))
    }
}

#[test]
fn q_eval_count() {
    let mut e = Engine::new();
    e.set_op_limit(u64::MAX / 4);
    e.set_mem_limit(512 * 1024 * 1024);
    e.set_call_depth_limit(1024);
    e.set_nest_limit(1024);
    e.sync_ctx_url(e.current_context(), "https://x.com/", 1);
    e.set_module_loader(Rc::new(RefCell::new(L)));
    let src = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/probe_xcom/entry.js"),
    )
    .unwrap();
    match e.eval_module(
        &src,
        "https://abs.twimg.com/x-web/x-web/assets/entry-client.js",
    ) {
        Ok(_) => println!("MOD OK"),
        Err(x) => println!(
            "MOD-ERR {}",
            x.to_string().chars().take(300).collect::<String>()
        ),
    }
    match e.eval("globalThis.__qc") {
        Ok(v) => println!("__qc = {}", e.describe(v)),
        Err(x) => println!("__qc ERR {x}"),
    }
}

#[test]
fn abort_signal_surface() {
    use explorer_jscore::{Dom, Engine};
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html("<p>x</p>"))));
    // ctor Illegal (spec) + statics présentes + proto réel.
    let v = e.eval("typeof AbortSignal+'/'+typeof AbortSignal.timeout+'/'+typeof AbortSignal.abort+'/'+typeof AbortSignal.any").unwrap();
    assert_eq!(e.describe(v), "function/function/function/function");
    // `AbortSignal.abort(r)` → aborted+reason ; `throwIfAborted` jette reason.
    let v = e.eval("var s=AbortSignal.abort('R');s.aborted+'/'+s.reason+'/'+(function(){try{s.throwIfAborted();return 'nothrow'}catch(x){return x}})()").unwrap();
    assert_eq!(e.describe(v), "true/R/R");
    // `AbortController` → signal réel ; abort tire onabort+listener.
    let v = e.eval("var c=new AbortController(),log='';c.signal.addEventListener('abort',function(){log+='L'});c.signal.onabort=function(){log+='O'};c.abort('Z');c.signal.aborted+'/'+c.signal.reason+'/'+log").unwrap();
    assert_eq!(e.describe(v), "true/Z/OL");
    // `any` : une source abortée propage.
    let v = e
        .eval("var s=AbortSignal.any([AbortSignal.abort('A')]);s.aborted+'/'+s.reason")
        .unwrap();
    assert_eq!(e.describe(v), "true/A");
}
