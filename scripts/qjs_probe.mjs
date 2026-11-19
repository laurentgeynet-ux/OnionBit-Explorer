// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Canal oracle QuickJS — quatrième moteur de triangulation via
// `quickjs-emscripten` (QuickJS réel compilé WASM, exécuté sous
// Node — zéro binaire système). Même protocole que
// `v8_probe.js` : lit une sonde sur stdin, l'évalue et imprime
// `<kind>\t<json>` sur stdout :
//   `echo 'src' | node scripts/qjs_probe.mjs`
// Outil d'arbitrage de conformité — jamais de code livré.
//
// Canonisation (miroir d'`Outcome`) :
//   complétion      -> `Value\tToString(v)`
//   SyntaxError     -> `Syntax\tmessage`
//   Error levée     -> `Throw\t<ctor.name>: <message>`
//   valeur non-Error -> `Throw\tToString(e)`
// Toute sortie autre que la ligne d'issue est interdite (le parent
// ne lit que la première ligne).

import { getQuickJS } from 'quickjs-emscripten';

let src = '';
process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) { src += chunk; }

const emit = (kind, text) => {
    process.stdout.write(kind + '\t' + JSON.stringify(text) + '\n');
};

const qjs = await getQuickJS();
const ctx = qjs.newContext();
try {
    // `evalCode` : évaluation globale — `var` crée des propriétés
    // globales, sémantique Script comparable à `Engine::eval`.
    const result = ctx.evalCode(src);
    if (result.error) {
        const e = ctx.dump(result.error);
        result.error.dispose();
        if (e && typeof e === 'object') {
            const name = e.name || 'Error';
            if (name === 'SyntaxError') {
                emit('Syntax', String(e.message || ''));
            } else {
                emit('Throw', name + ': ' + String(e.message || ''));
            }
        } else {
            emit('Throw', String(e));
        }
    } else {
        // ToString **dans** QuickJS : `String(handle)` — jamais de
        // conversion JS↔hôte qui changerait la sémantique.
        const strFn = ctx.getProp(ctx.global, 'String');
        const sHandle = ctx.callFunction(strFn, ctx.undefined, [result.value]);
        result.value.dispose();
        strFn.dispose();
        const unr = ctx.unwrapResult(sHandle);
        emit('Value', String(ctx.dump(unr)));
        unr.dispose();
    }
} catch (err) {
    emit('Throw', 'HostError: ' + String(err && err.message || err));
} finally {
    try { ctx.dispose(); } catch (_) { /* jobs pendants : les sondes ne testent pas le scheduling */ }
}
