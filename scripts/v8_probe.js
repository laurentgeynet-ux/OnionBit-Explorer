// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Canal oracle V8 standalone (Node.js) — survivant du crate
// `explorer-boa-diff` (débranché JS-C589, historique git). Lit
// une sonde sur stdin, l'évalue en sémantique `<script>` et
// imprime `<kind>\t<json>` sur stdout :
//   `echo 'src' | node scripts/v8_probe.js`
// Outil d'arbitrage de conformité — jamais de code livré.
//
// Canonisation (miroir d'`Outcome`) :
//   complétion      -> `Value\tToString(v)`
//   SyntaxError     -> `Syntax\tmessage`
//   Error levée     -> `Throw\t<ctor.name>: <message>`
//   valeur non-Error -> `Throw\tToString(e)`
// Toute sortie autre que la ligne d'issue est interdite (le parent
// ne lit que la première ligne).

'use strict';

const vm = require('node:vm');

// Une promesse rejetée non rattrapée tuerait le processus en fin
// de run (exit != 0) alors que l'issue est déjà produite — les
// sondes ne testent pas le scheduling, on avale l'événement.
process.on('unhandledRejection', () => {});

let src = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => { src += chunk; });
process.stdin.on('end', () => {
    // `runInThisContext` : la sonde s'exécute comme un **Script**
    // (sémantique `<script>` : `var` global non-configurable) —
    // même contrat que `Engine::eval` (jscore) et `Context::eval`
    // (boa). L'eval indirect donnerait des bindings *configurables*
    // (sémantique eval, fausse l'oracle : `var q;delete q` → true).
    // Le texte est JSON-encodé : une seule ligne quels que soient
    // les `\n` de la valeur (protocole `kind\t<json>` — sinon les
    // valeurs multi-lignes étaient tronquées à la première ligne).
    const emit = (kind, text) => {
        process.stdout.write(kind + '\t' + JSON.stringify(text) + '\n');
    };
    try {
        const v = vm.runInThisContext(src);
        let s;
        try { s = String(v); } catch (_) { s = '<tostring>'; }
        emit('Value', s);
    } catch (e) {
        if (e instanceof SyntaxError) {
            emit('Syntax', e.message || '');
        } else if (e instanceof Error) {
            const name = (e.constructor && e.constructor.name) || 'Error';
            emit('Throw', name + ': ' + (e.message || ''));
        } else {
            let s;
            try { s = String(e); } catch (_) { s = '<tostring>'; }
            emit('Throw', s);
        }
    }
});
