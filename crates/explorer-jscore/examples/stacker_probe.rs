fn rss() -> (usize, usize) {
    let m = memory_stats::memory_stats().unwrap();
    (m.physical_mem / (1 << 20), m.virtual_mem / (1 << 20))
}
fn churn(n: usize) -> usize {
    if n == 0 { return 0; }
    // red_zone 4 Mo > stack réelle ⇒ fibre à CHAQUE appel
    stacker::maybe_grow(4 * 1024 * 1024, 2 * 1024 * 1024, || churn(n - 1) + 1)
}
fn main() {
    println!("start {:?}", rss());
    // simule le churn de statements: 40_000 appels maybe_grow
    let mut acc = 0;
    for _ in 0..40_000 {
        acc += stacker::maybe_grow(4 * 1024 * 1024, 2 * 1024 * 1024, || 1);
    }
    println!("after 40k fibers {acc} {:?}", rss());
    let d = churn(200);
    println!("nested 200 {d} {:?}", rss());
}
