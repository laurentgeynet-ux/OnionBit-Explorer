# Contributing

Thanks for your interest in OnionBit Explorer.

**Start here: [`AGENTS.md`](AGENTS.md)** — it is the working contract
for this repository and applies to humans and agents alike: mission
scope (from-scratch engine, no imported engine crates), the hard
rules (`forbid(unsafe_code)`, no `unwrap`/`panic` in library code,
sockets only in `explorer-net`, pure-Rust dependencies), the
architecture map, and the required gate.

## Before any commit

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps

# If app/ is touched:
cd app; flutter analyze; flutter test
```

Or all at once: `pwsh -File scripts/verify_all.ps1`.

## Conventions

- One step = one dedicated commit, `feat(<crate>): …` /
  `fix(<crate>): …` / `docs: …` / `chore: …`.
- `CHANGELOG.md` updated in the same commit; the owning team's
  roadmap checkbox ticked in the same commit.
- License header on every source file (`SPDX-License-Identifier:
  MIT OR Apache-2.0`).
- Internal docs in French; identifiers and code in English.
- Compatibility claims are *measured* — a score always carries its
  denominator (subset extracted, files run, skips).

## Team model

The repo is governed as two parallel tracks — **PIPELINE** (pure
domain: `dom`, `html`, `css`, `layout`, `render`) and **NAVIGATEUR**
(platform: `net`, `core`, `api`, `daemon`, `cli`, `ffi`, `app/`,
all of JS). Cross-track needs go through the seam journal
`docs/interface-equipes.md`, not direct edits outside one's
perimeter. See `AGENTS.md` §10.

## License

By contributing you agree your work is dual-licensed under
**MIT OR Apache-2.0**, at the recipient's option.
