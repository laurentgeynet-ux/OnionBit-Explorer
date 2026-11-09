// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C457 (JT-13) — `import('inline:N')` : le specifier
//! URL opaque d'un module déjà instancié ne part jamais au
//! fetch (vimeo — `DynImportReq` mourait « schéma non
//! supporté »). Identité `module_urls`/`module_specs`.

use explorer_jscore::Engine;

#[test]
fn c457_import_already_instantiated_inline_url() {
    let mut e = Engine::new();
    e.eval_module("export const x = 42; export default {ok:1};", "inline:1")
        .expect("eval inline module");
    // `import('inline:1')` → même namespace, zéro fetch.
    let r = e
        .eval("import('inline:1').then(m => { globalThis.__v = m.x + '|' + m.default.ok; })")
        .expect("import");
    let _ = r;
    e.run_microtasks().expect("drain");
    let v = e.eval("__v").map(|x| e.describe(x)).unwrap();
    assert_eq!(v, "42|1");
}
