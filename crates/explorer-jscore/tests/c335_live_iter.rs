// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C335 — itération **vivante** des méthodes `Array`
//! à callback : `cb(v, i, arr)` reçoit `arr` réellement peuplé
//! (`arr.length`, `arr.indexOf`, `arr[j]` répondent) et observe
//! les mutations des indices non encore visités (spec
//! `HasProperty`/`Get` par indice).
//!
//! Mur §B-90 — Bitmovin `test-stream` : le `M3u8Translator`
//! déduplique les préfixes de codec via
//! `hbb = list.map(Vab).filter((e,t,i)=>e && i.indexOf(e)===t)`.
//! `take_elems`/`with_elems` vidaient le backing pendant toute
//! la boucle → `i.indexOf(e)` valait −1 pour tout → `hbb` → `[]`
//! → bucketing `{unknown:[6 playlists]}` → **aucun adaptation
//! set vidéo** → `getSeekableRange` sur `Vc` undefined →
//! `player.load()` rejeté.

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// Le motif Bitmovin exact : dedup via `i.indexOf(e)===t`.
#[test]
fn bitmovin_codec_prefix_dedup() {
    assert_eq!(
        eval(
            "var a=['avc1','avc1','mp4a','avc1','hvc1','mp4a'];\
             a.filter(function(e,t,i){return e && i.indexOf(e)===t}).join(',')"
        ),
        "avc1,mp4a,hvc1"
    );
    // Même motif via `map` amont (forme exacte de `hbb`).
    assert_eq!(
        eval(
            "var pls=[{c:'avc1'},{c:'avc1'},{c:'mp4a'}];\
             pls.map(function(p){return p.c})\
             .filter(function(e,t,i){return e && i.indexOf(e)===t}).join(',')"
        ),
        "avc1,mp4a"
    );
}

/// Le 3ᵉ argument `arr` est peuplé dans toutes les méthodes à
/// callback.
#[test]
fn array_arg_populated_in_callbacks() {
    assert_eq!(
        eval("var a=['x','y','z'],s='';a.forEach(function(v,i,arr){s+=arr.length});s"),
        "333"
    );
    assert_eq!(
        eval("var a=['x','y','z'];a.map(function(v,i,arr){return arr[i]}).join(',')"),
        "x,y,z"
    );
    assert_eq!(
        eval("var a=[5,5,5];a.filter(function(v,i,arr){return arr.indexOf(v)===i}).length"),
        "1"
    );
    assert_eq!(
        eval("var a=[1,2,3];a.reduce(function(acc,v,i,arr){return acc+arr.length},0)"),
        "9"
    );
    assert_eq!(
        eval("var a=[1,2,3];a.reduceRight(function(acc,v,i,arr){return acc+arr.length},0)"),
        "9"
    );
    assert_eq!(
        eval("var a=[1,2];a.flatMap(function(v,i,arr){return [arr.length]}).join(',')"),
        "2,2"
    );
    // `find`/`some`/`every`/`findIndex`/`findLast`/`findLastIndex`.
    assert_eq!(
        eval("var a=[7,8,9],s='';a.some(function(v,i,arr){s+=arr[0];return false});s+'/'+a.find(function(v,i,arr){return v===arr[2]})"),
        "777/9"
    );
    assert_eq!(
        eval("var a=[7,8,9],s='';a.every(function(v,i,arr){s+=arr.length;return true});s+'/'+a.findIndex(function(v,i,arr){return v===arr[1]})+'/'+a.findLastIndex(function(v,i,arr){return arr.length===3})"),
        "333/1/2"
    );
    assert_eq!(
        eval("var a=[1,2,3];a.findLast(function(v,i,arr){return arr[0]===1})"),
        "3"
    );
}

/// `join` : le `toString` d'un élément voit `this` peuplé.
#[test]
fn join_tostring_sees_populated_array() {
    assert_eq!(
        eval("var a=[{toString:function(){return a.length}},'b'];a.join('-')"),
        "2-b"
    );
}

/// `sort(cmp)` : le comparateur voit `this` intact (spec : les
/// écritures n'ont lieu qu'après le tri).
#[test]
fn sort_cmp_sees_original_array() {
    assert_eq!(
        eval("var a=[3,1,2],s='';a.sort(function(x,y){s+=a.length;return x-y});a.join(',')+'/'+s"),
        "1,2,3/333"
    );
}

/// Mutations spec : la cb modifie un indice non visité → la
/// nouvelle valeur est itérée (`Get` vivant).
#[test]
fn live_mutation_visible_later() {
    assert_eq!(
        eval("var a=[1,2,3],seen=[];a.forEach(function(v,i){if(i===0){a[2]=42}seen.push(v)});seen.join(',')"),
        "1,2,42"
    );
    // Suppression d'un indice non visité → sauté (HasProperty).
    assert_eq!(
        eval("var a=[1,2,3],seen=[];a.forEach(function(v,i){if(i===0){delete a[2]}seen.push(v)});seen.join(',')"),
        "1,2"
    );
    // Rétrécissement : `length=1` en vol → indices suivants sautés.
    assert_eq!(
        eval("var a=[1,2,3],seen=[];a.forEach(function(v,i){if(i===0){a.length=1}seen.push(v)});seen.join(',')+'/'+a.length"),
        "1/1"
    );
}

/// Court-circuit spec : `find`/`some`/`every` cessent d'appeler
/// la cb au premier verdict.
#[test]
fn short_circuit() {
    assert_eq!(
        eval("var a=[1,2,3,4],n=0;a.find(function(v){n++;return v===2});n"),
        "2"
    );
    assert_eq!(
        eval("var a=[1,2,3,4],n=0;a.some(function(v){n++;return v===2});n"),
        "2"
    );
    assert_eq!(
        eval("var a=[1,2,3,4],n=0;a.every(function(v){n++;return v<2});n"),
        "2"
    );
}

/// Le tableau itéré reste intact après la boucle, cb qui jette
/// comprise (JS-C319 conservé — la visibilité ne coûte rien en
/// restauration puisqu'il n'y a plus de `take`).
#[test]
fn array_intact_after_iteration() {
    assert_eq!(
        eval(
            "var a=[1,2,3];try{a.forEach(function(v,i){if(i===1)throw 'x'})}catch(e){}\
             a.length+'/'+a.join(',')"
        ),
        "3/1,2,3"
    );
    // Les AUTRES tableaux lus dans la cb restent normaux (le mur
    // ne concernait que le tableau itéré).
    assert_eq!(
        eval("var a=['x'],b=['o','t','h','e','r'],s='';a.forEach(function(){s+=b.length});s"),
        "5"
    );
}

/// `hasOwn` spec : les trous sont sautés, les accesseurs proto
/// invoqués.
#[test]
fn holes_and_proto_elements() {
    // Trou dense sans source proto → cb non appelée sur l'indice.
    assert_eq!(eval("var a=[1,,3],n=0;a.forEach(function(){n++});n"), "2");
    // Élément fourni par la proto → cb appelée (HasProperty).
    assert_eq!(
        eval("var a=[,2];Array.prototype[0]=9;var n=0,vals=[];a.forEach(function(v){n++;vals.push(v)});delete Array.prototype[0];n+'/'+vals.join(',')"),
        "2/9,2"
    );
}
