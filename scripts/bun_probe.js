// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Canal oracle JavaScriptCore (Bun) — troisième moteur de
// triangulation, même protocole que `v8_probe.js` : lit une
// sonde sur stdin, l'évalue en sémantique `<script>` et imprime
// `<kind>\t<json>` sur stdout :
//   `echo 'src' | bun scripts/bun_probe.js`
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

// Même raisonnement que v8_probe : une promesse rejetée non
// rattrapée ne doit pas tuer le processus — l'issue est déjà
// produite, les sondes ne testent pas le scheduling.
process.on('unhandledRejection', () => {});

let src = '';
(async () => {
    for await (const chunk of Bun.stdin.stream()) {
        src += new TextDecoder().decode(chunk);
    }

    const emit = (kind, text) => {
        process.stdout.write(kind + '\t' + JSON.stringify(text) + '\n');
    };
    try {
        // `runInThisContext` : sémantique Script (var global
        // non-configurable) — même contrat que `Engine::eval`
        // (jscore). Bun le mappe sur l'évaluation JSC.
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
})();
