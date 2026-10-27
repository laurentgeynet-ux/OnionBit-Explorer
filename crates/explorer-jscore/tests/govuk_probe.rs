// sonde one-shot : évalue analytics puis app dans un Engine nu
#[test]
fn govuk_pair_bare_engine() {
    let an = std::fs::read_to_string("C:/Users/Lou/AppData/Local/Temp/govuk_load-analytics-d.js")
        .unwrap();
    let app = std::fs::read_to_string("C:/Users/Lou/AppData/Local/Temp/govuk_app.js").unwrap();
    let mut e = explorer_jscore::Engine::new();
    eprintln!("eval analytics…");
    let r1 = e.eval(&an);
    eprintln!("analytics -> {:?}", r1.is_ok());
    eprintln!("eval app…");
    let r2 = e.eval(&app);
    eprintln!("app -> {:?}", r2.is_ok());
}
