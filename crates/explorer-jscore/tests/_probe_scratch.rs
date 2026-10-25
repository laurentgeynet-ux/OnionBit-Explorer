use explorer_jscore::{Dom, Engine, ModuleLoader};
use std::cell::RefCell;
use std::rc::Rc;

fn dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/probe_xcom")
}

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
        let known = [
            ("rolldown-runtime", "runtime.js"),
            ("startup-vendor", "vendor.js"),
            ("startup-D", "startup.js"),
            ("startup-dynamic", "dynamic.js"),
            ("entry-client", "entry.js"),
        ];
        for (pat, f) in known {
            if spec.contains(pat) {
                let mut src =
                    std::fs::read_to_string(dir().join(f)).map_err(|e| format!("{f}:{e}"))?;
                if pat == "startup-vendor" {
                    src = src.replacen(
                        "function sd(e,t=!1){let n=nd()",
                        "function sd(e,t=!1){globalThis.__sdarg=e;let n=nd()",
                        1,
                    );
                    src = src.replacen(
                        "Ud=class{constructor(e,t){",
                        "Ud=class{constructor(e,t){(globalThis.__calls=globalThis.__calls||[]).push(e);globalThis.__ctorArg=e;",
                        1,
                    );
                    src = src.replacen(
                        "this.routeTree=this.options.routeTree",
                        "globalThis.__rtOpts=this.options.routeTree;this.routeTree=this.options.routeTree",
                        1,
                    );
                    src = src.replacen(
                        "e.id in i&&lu()",
                        "e.id in i&&(globalThis.__collide=e.id,globalThis.__route=e,globalThis.__i=i,lu())",
                        1,
                    );
                }
                if pat == "startup-dynamic" {
                    src = src.replacen(
                        "h$=Q._addFileChildren(m$)._addFileTypes()",
                        "h$=(globalThis.__Q=Q,globalThis.__LB=LB,globalThis.__h=Q._addFileChildren(m$)._addFileTypes())",
                        1,
                    );
                    src = src.replacen("routeTree:h$", "routeTree:(globalThis.__hAtLp=h$,h$)", 1);
                }
                return Ok(src);
            }
        }
        let base = spec.rsplit('/').next().unwrap_or(spec);
        let p = dir().join(base);
        if p.is_file() {
            std::fs::read_to_string(&p).map_err(|e| format!("{base}:{e}"))
        } else {
            Err(format!("no-source:{spec}"))
        }
    }
}

#[test]
fn real_module_graph() {
    let html = std::fs::read_to_string(dir().join("xcom_page.html")).unwrap();
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(&html))));
    e.set_op_limit(u64::MAX / 4);
    e.set_mem_limit(512 * 1024 * 1024);
    e.set_call_depth_limit(1024);
    e.set_nest_limit(1024);
    e.set_diag_hook(Rc::new(|m| eprintln!("[diag] {m}")));
    e.sync_ctx_url(e.current_context(), "https://x.com/", 1);
    e.set_module_loader(Rc::new(RefCell::new(L)));

    let mut ord = 0usize;
    let mut rest = html.as_str();
    loop {
        let Some(o) = rest.find("<script") else { break };
        let rest2 = &rest[o + 7..];
        let Some(gt) = rest2.find('>') else { break };
        let tag = &rest2[..gt];
        let Some(c) = rest2.find("</script>") else {
            break;
        };
        let body = &rest2[gt + 1..c];
        let set = std::env::var("PROBE_SET").unwrap_or_else(|_| "all".into());
        let is_inline = !tag.contains("src")
            && !tag.contains("module")
            && !body.trim().is_empty()
            && (set == "all" || set.split(',').any(|x| x.trim() == ord.to_string()));
        if is_inline {
            match e.eval_script_el(body, Some("inline"), ord) {
                Ok(_) => println!("script[{ord}] OK"),
                Err(err) => println!(
                    "script[{ord}] ERR {}",
                    err.to_string().chars().take(200).collect::<String>()
                ),
            }
        }
        ord += 1;
        rest = &rest2[c + 9..];
    }
    let src = std::fs::read_to_string(dir().join("entry.js")).unwrap();
    match e.eval_module(
        &src,
        "https://abs.twimg.com/x-web/x-web/assets/entry-client.js",
    ) {
        Ok(_) => println!("MODULE EVAL-OK"),
        Err(err) => println!(
            "MODULE EVAL-ERR {}",
            err.to_string().chars().take(1600).collect::<String>()
        ),
    }
    for probe in [
        "String(globalThis.__collide)",
        "(()=>{let e=globalThis.__route;return e?Object.keys(e).join(',')+'|id='+e.id+'|path='+e.path+'|opts='+(e.options?Object.keys(e.options).join(','):'none'):'none'})()",
        "(()=>{let i=globalThis.__i;return i?Object.keys(i).join(','):'none'})()",
        "(()=>{let e=globalThis.__route,p=e&&e.parentRoute;return p?(typeof p)+'|keys='+Object.keys(p).join(',')+'|id='+p.id+'|path='+p.path:'none'})()",
        "(()=>{let e=globalThis.__route,g=e.options&&e.options.getParentRoute;return 'opt.gPR='+(typeof g)+'|call='+(g?String(g()).slice(0,60):'-')})()",
        "(()=>{let e=globalThis.__route,g=e.options&&e.options.getParentRoute,r=g&&g();return 'gPR()=>type='+(typeof r)+'|keys='+(r?Object.keys(r).join(','):'-')+'|id='+(r&&r.id)+'|same='+(r===e.parentRoute)})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute,reg=i&&i['/$username'];return 'reg_keys='+(reg?Object.keys(reg).length:'-')+'|sameLB='+(reg===p)+'|reg_children='+(reg&&reg.children?reg.children.length:'-')+'|p_children='+(p.children?p.children.length:'-')+'|children_same='+(reg&&p&&reg.children===p.children)})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute,reg=i&&i['/$username'];return reg?'reg._id='+reg._id+'|reg===p?'+String(reg===p)+'|proto_same='+(Object.getPrototypeOf(reg)===Object.getPrototypeOf(p)):'-'})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute;return 'p_ctor='+(p.constructor?p.constructor.name:'?')+'|p_id='+p._id+'|p_children===opts?'+(p.children===p.options.children)})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute;return 'p===root?'+String(p===i['__root__'])+'|p.children[0].id='+(p.children&&p.children[0]?p.children[0].id:'-')+'|p.isRoot='+p.isRoot+'|root_child0='+(i['__root__']&&i['__root__'].children?i['__root__'].children[0].id:'-')})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute,reg=i['/$username'];return 'p_in_children?'+((reg&&reg.children||[]).includes(p))+'|reg_parentRoute_id='+(reg&&reg.parentRoute?reg.parentRoute.id:'-')+'|e_in_reg_children?'+((reg&&reg.children||[]).includes(e))})()",
        "(()=>{let e=globalThis.__route;return 'gPR_src='+String(e.options.getParentRoute).slice(0,120)})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute;return 'p_options_keys='+(p.options?Object.keys(p.options).join(','):'-')+'|p_opt_gPR='+(p.options&&p.options.getParentRoute?String(p.options.getParentRoute()).slice(0,80):'-')})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute;return 'p_children0_keys='+(p.children&&p.children[0]?Object.keys(p.children[0]).join(',').slice(0,200):'-')})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute,r=i['__root__'];return 'p_in_reg='+Object.values(i).includes(p)+'|same_children_arr='+(p.children===r.children)+'|p.child0===reg='+(p.children[0]===i['/$username'])+'|p.child_count='+p.children.length+'|root_child_count='+r.children.length})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute;return 'p_opts===Q_opts?n/a|p_opts_id_keys='+(p.options?Object.keys(p.options).length:'-')+'|e.parent_is_p?'+String(e.parentRoute===p)})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute,reg=i['/$username'];return 'reg_children0_id='+(reg.children[0]?reg.children[0].id:'-')+'|p_children0_gPR='+(p.children[0].options&&p.children[0].options.getParentRoute?String(p.children[0].options.getParentRoute()).slice(0,60):'-')+'|p_child0_isRoot='+(p.children[0]?p.children[0].isRoot:'-')})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,p=e.parentRoute,r=i['__root__'],reg=i['/$username'];return 'r_opts===p_opts?'+String(r.options===p.options)+'|reg_opts===p_child0_opts?'+String(reg.options===p.children[0].options)+'|reg_children===p_child0_children?'+String(reg.children===p.children[0].children)+'|reg_proto_r_proto?'+String(Object.getPrototypeOf(reg)===Object.getPrototypeOf(r))})()",
        "(()=>{let i=globalThis.__i,e=globalThis.__route,r=i['__root__'],reg=i['/$username'];return 'r_child0_gPR_id='+(reg.parentRoute===r)+'|e_gPR_same_as_rg?'+String(e.options.getParentRoute()===reg.parentRoute)+'|reg_init_this?'+String(reg.init.length)})()",
        "(()=>{let p=globalThis.__route.parentRoute,Q=globalThis.__Q,h=globalThis.__h,i=globalThis.__i,r=i['__root__'];return 'p===Q?'+String(p===Q)+'|p===h?'+String(p===h)+'|r===Q?'+String(r===Q)+'|r===h?'+String(r===h)+'|Q_children='+(Q?Q.children.length:'-')+'|h_children='+(h?h.children.length:'-')+'|Q_id='+(Q?Q._id:'-')+'|h_id='+(h?h._id:'-')})()",
        "(()=>{let LB=globalThis.__LB,p=globalThis.__route.parentRoute;return 'LB===p?'+String(LB===p)+'|LB_id='+(LB?LB._id:'-')+'|LB_children='+(LB?LB.children.length:'-')})()",
        "(()=>{let d=globalThis.__sdarg,Q=globalThis.__Q,h=globalThis.__h,i=globalThis.__i,r=i['__root__'];return 'sdarg===Q?'+String(d===Q)+'|sdarg===h?'+String(d===h)+'|sdarg===r?'+String(d===r)+'|sd_children='+(d?d.children.length:'-')+'|sd_opts='+(d?Object.keys(d.options).length:'-')+'|sd_init_len='+(d?d.init.length:'-')})()",
        "(()=>{let o=globalThis.__rtOpts,h=globalThis.__h,Q=globalThis.__Q,r=globalThis.__sdarg;return 'rtOpts===h?'+String(o===h)+'|rtOpts===Q?'+String(o===Q)+'|rtOpts===r?'+String(o===r)+'|rtOpts_children='+(o?o.children.length:'-')+'|rtOpts_id='+(o?o._id:'-')})()",
        "(()=>{let c=globalThis.__ctorArg,h=globalThis.__h,Q=globalThis.__Q,r=globalThis.__sdarg;return 'ctorArg_rt===h?'+String(c.routeTree===h)+'|ctorArg_rt===Q?'+String(c.routeTree===Q)+'|ctorArg_rt===r?'+String(c.routeTree===r)+'|ctorArg_keys='+Object.keys(c).length+'|rt_children='+(c.routeTree?c.routeTree.children.length:'-')})()",
        "(()=>{let x=globalThis.__hAtLp,Q=globalThis.__Q,h=globalThis.__h,r=globalThis.__sdarg;return 'hAtLp===Q?'+String(x===Q)+'|hAtLp===h?'+String(x===h)+'|hAtLp===r?'+String(x===r)+'|hAtLp_type='+(typeof x)+'|hAtLp_children='+(x?x.children&&x.children.length:'-')})()",
        "(()=>{let cs=globalThis.__calls||[];return 'calls='+cs.length+'|rt_is='+cs.map(c=>c.routeTree===globalThis.__h?'h':c.routeTree===globalThis.__Q?'Q':c.routeTree===globalThis.__sdarg?'r':'other'+ (c.routeTree?Object.keys(c.routeTree.options||{}).length:'')).join(',')})()",
    ] {
        match e.eval(probe) {
            Ok(v) => println!("PROBE {probe} => {}", e.describe(v)),
            Err(x) => println!("PROBE {probe} ERR {}", x.to_string().chars().take(300).collect::<String>()),
        }
    }
}

#[test]
fn root_identity() {
    // même harnais que real_module_graph — sondes i['__root__']
    let html = std::fs::read_to_string(dir().join("xcom_page.html")).unwrap();
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(&html))));
    e.set_op_limit(u64::MAX / 4);
    e.set_mem_limit(512 * 1024 * 1024);
    e.set_call_depth_limit(1024);
    e.set_nest_limit(1024);
    e.sync_ctx_url(e.current_context(), "https://x.com/", 1);
    e.set_module_loader(Rc::new(RefCell::new(L)));
    // même amorçage que real_module_graph : scripts inline
    // de la page d'abord (ils préparent l'environnement).
    let mut ord = 0usize;
    let mut rest = html.as_str();
    while let Some(o) = rest.find("<script") {
        let rest2 = &rest[o + 7..];
        let Some(gt) = rest2.find('>') else { break };
        let tag = &rest2[..gt];
        let Some(c) = rest2.find("</script>") else {
            break;
        };
        let body = &rest2[gt + 1..c];
        if !tag.contains("src") && !tag.contains("module") && !body.trim().is_empty() {
            let _ = e.eval_script_el(body, Some("inline"), ord);
        }
        ord += 1;
        rest = &rest2[c + 9..];
    }
    let src = std::fs::read_to_string(dir().join("entry.js")).unwrap();
    match e.eval_module(
        &src,
        "https://abs.twimg.com/x-web/x-web/assets/entry-client.js",
    ) {
        Ok(_) => println!("MODULE OK"),
        Err(x) => println!(
            "MOD-ERR {}",
            x.to_string().chars().take(400).collect::<String>()
        ),
    }
    for probe in [
        "(()=>{let r=globalThis.__rootobj,p=globalThis.__route&&globalThis.__route.parentRoute;return 'root_keys='+Object.keys(r||{}).join(',').slice(0,200)})()",
        "(()=>{let r=globalThis.__rootobj,p=globalThis.__route.parentRoute;return 'r===p?'+String(r===p)+'|r._id='+r._id+'|r.children='+(r.children?r.children.length:'-')+'|p_in_r_children?'+((r.children||[]).includes(p))+'|r_getter='+(Object.getOwnPropertyDescriptor(Object.getPrototypeOf(r),'id')?'Y':'N')})()",
        "(()=>{let r=globalThis.__rootobj,p=globalThis.__route.parentRoute;return 'r_keys_has_id?'+('id' in r)+'|p_keys_has_id?'+('id' in p)+'|r_ctor='+(r.constructor&&r.constructor.name)+'|p_ctor='+(p.constructor&&p.constructor.name)})()",
        "(()=>{let r=globalThis.__rootobj;return 'r_child_undef='+(r.children||[]).filter(c=>c._id===void 0).length+'/'+ (r.children||[]).length+'|r_children0_id='+(r.children&&r.children[0]&&r.children[0].id)+'|r_children_ids_sample='+(r.children||[]).slice(0,8).map(c=>c.id).join(',')})()",
        "(()=>{let p=globalThis.__route.parentRoute;return 'p_children_len='+p.children.length+'|p_children0_id='+(p.children[0]&&p.children[0].id)+'|p_children_sample='+(p.children||[]).slice(0,6).map(c=>c.id).join(',')})()",
    ] {
        match e.eval(probe) {
            Ok(v) => println!("P {} => {}", &probe[5..60.min(probe.len())], e.describe(v)),
            Err(x) => println!("P {} ERR {}", &probe[5..60.min(probe.len())], x.to_string().chars().take(200).collect::<String>()),
        }
    }
}
