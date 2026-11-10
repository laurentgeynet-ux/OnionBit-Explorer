// sonde one-shot : évalue analytics puis app dans un Engine nu
#[test]
fn govuk_pair_bare_engine() {
    // Fixtures de vol non versionnées — skip propre (convention b104).
    let Ok(an) =
        std::fs::read_to_string("C:/Users/Lou/AppData/Local/Temp/govuk_load-analytics-d.js")
    else {
        eprintln!("fixtures govuk absentes — test sauté");
        return;
    };
    let Ok(app) = std::fs::read_to_string("C:/Users/Lou/AppData/Local/Temp/govuk_app.js") else {
        eprintln!("fixtures govuk absentes — test sauté");
        return;
    };
    let mut e = explorer_jscore::Engine::new();
    eprintln!("eval analytics…");
    let r1 = e.eval(&an);
    eprintln!("analytics -> {:?}", r1.is_ok());
    eprintln!("eval app…");
    let r2 = e.eval(&app);
    eprintln!("app -> {:?}", r2.is_ok());
}
