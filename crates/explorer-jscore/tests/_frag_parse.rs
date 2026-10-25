use explorer_jscore::Engine;
#[test]
fn frag() {
    let mut e = Engine::new();
    for s in [
        "let t=1,{children:n}={children:2},r;[t,n]",
        "let t=(0,Math.max)(1,2),{children:n}={children:2},r;[t,n]",
        "function VA(e){let t=(0,Math.max)(1,2),{children:n}=e,r;return n}VA({children:7})",
        "let {children:n}={children:2},r=3;n+r",
        "let t=1,{a}={a:5};t+a",
    ] {
        match e.eval(s) {
            Ok(v) => println!("OK  {:?} => {}", &s[..40.min(s.len())], e.describe(v)),
            Err(x) => println!(
                "ERR {:?} => {}",
                &s[..40.min(s.len())],
                x.to_string().chars().take(160).collect::<String>()
            ),
        }
    }
}
