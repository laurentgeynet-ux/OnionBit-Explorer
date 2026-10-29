// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-59 — régression « fuite pompe post-nav ».
//!
//! Repro in-process d'un « flux continu » presse/radio : un
//! interval qui mute le DOM + churn observers + churn JS, avec
//! drain journal entre pompes (rôle de la synchro session) et
//! GC complet à chaque round. Avant le fix : `cbs`/`regs`/
//! `nodes`/`journal`/heap croissaient **monotone** (+1..+63
//! par tick, jamais réclamé) — la signature des vols
//! (+250-600 Mo/s). Verrou : après chauffe, chaque compteur
//! reste **borné** sur 12 rounds supplémentaires.

#[cfg(test)]
mod tests {
    use crate::dom::Dom;
    use crate::dom_host::DomRef;
    use crate::Engine;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn stats(e: &Engine) -> (usize, usize, usize, usize, usize, usize, usize) {
        let (nodes, muts, oregs, journal, pend) = match &e.dom_ctx {
            Some((d, _)) => d.borrow().b59_counts(),
            None => (0, 0, 0, 0, 0),
        };
        let cache = match &e.dom_ctx {
            Some((_, c)) => c.borrow().len(),
            None => 0,
        };
        (
            e.live_count(),
            e.mutobs.b59_len(),
            e.obs.b59_len(),
            nodes,
            journal,
            cache,
            oregs + muts + pend,
        )
    }

    #[test]
    fn b59_flux_continu_borne() {
        let dom: DomRef = Rc::new(RefCell::new(Dom::from_html(
            "<html><body><div id='feed'></div></body></html>",
        )));
        let mut e = Engine::new();
        e.install_dom(&dom);
        e.install_timers();
        // Flux continu type presse : refresh de tuiles + un
        // observer par carte lazy-load + mutations + promises.
        e.eval(
            "var feed=document.getElementById('feed');var i=0;\
             setInterval(function(){i++;\
               feed.innerHTML='<article class=card>'+i+'</article><article class=card>'+(i+1)+'</article>';\
               var io=new IntersectionObserver(function(){});io.observe(feed);io.disconnect();\
               var ro=new ResizeObserver(function(){});ro.observe(feed);ro.disconnect();\
               var mo=new MutationObserver(function(){});mo.observe(feed,{childList:true});mo.disconnect();\
               var a=[];for(var k=0;k<50;k++)a.push({x:k});\
               Promise.resolve(k).then(function(){});\
             },0)",
        )
        .unwrap();
        // Chauffe : 4 rounds pour laisser les structures se
        // stabiliser (journal drainé = synchro session réelle).
        let mut last = (0, 0, 0, 0, 0, 0, 0);
        for _ in 0..4 {
            let _ = e.run_due_timers_bounded(100);
            if let Some((d, _)) = &e.dom_ctx {
                let _ = d.borrow_mut().take_journal();
            }
            let _ = e.collect();
            last = stats(&e);
        }
        // Fenêtre : tout doit rester borné — la fuite §B-59
        // se manifestait en croissance monotone dès le round 1.
        let (mut hmax, mut cmax, mut omax, mut nmax, mut jmax, mut wmax, mut qmax) = last;
        for _ in 0..12 {
            let _ = e.run_due_timers_bounded(100);
            if let Some((d, _)) = &e.dom_ctx {
                let _ = d.borrow_mut().take_journal();
            }
            let _ = e.collect();
            let s = stats(&e);
            hmax = hmax.max(s.0);
            cmax = cmax.max(s.1);
            omax = omax.max(s.2);
            nmax = nmax.max(s.3);
            jmax = jmax.max(s.4);
            wmax = wmax.max(s.5);
            qmax = qmax.max(s.6);
        }
        assert_eq!(cmax, 0, "mutobs.cbs doit être purgé (observers morts)");
        assert_eq!(omax, 0, "obs.regs doit être purgé (observers morts)");
        assert!(
            nmax <= last.3 + 24,
            "arène Dom bornée par réutilisation : base={} max={nmax}",
            last.3
        );
        assert!(
            wmax <= last.5 + 24,
            "NodeCache borné : base={} max={wmax}",
            last.5
        );
        assert!(
            hmax <= last.0 + 400,
            "tas JS borné sous churn observers : base={} max={hmax}",
            last.0
        );
        assert!(jmax <= 64, "journal drainé : max={jmax}");
        assert!(qmax <= 64, "files internes bornées : max={qmax}");
    }

    /// Un observer déconnecté puis **ré-armé** doit refonctionner
    /// — la purge ne doit pas casser `observe` sur instance vivante.
    #[test]
    fn b59_rearm_after_disconnect_and_gc() {
        let dom: DomRef = Rc::new(RefCell::new(Dom::from_html(
            "<html><body><div id='feed'></div></body></html>",
        )));
        let mut e = Engine::new();
        e.install_dom(&dom);
        e.eval(
            "var feed=document.getElementById('feed');var hits=0;\
             var mo=new MutationObserver(function(rs){hits+=rs.length});\
             mo.observe(feed,{childList:true});mo.disconnect();",
        )
        .unwrap();
        let _ = e.collect();
        // L'observer vit encore (variable globale `mo`) → ré-arme.
        e.eval(
            "mo.observe(feed,{childList:true});\
             feed.innerHTML='<p>x</p>';",
        )
        .unwrap();
        // Livraison : checkpoint microtask → callback → hits > 0.
        let _ = e.run_microtasks();
        let hits = e.eval("hits").unwrap();
        assert!(matches!(hits, crate::value::Value::Num(n) if n > 0.0));
    }
}
