# Resources fixtures (synthetic)

Shared by the Rust `resources` command (`xplat/flockbook-modfeed/tests/resources.rs`) and the Swift `Resources.collect` (next release).
`conversation.jsonl` is a Claude Code log; `expected.json` is what is collected with the home `/home/me`, newest first
(same time: the later mention in the reply first). Only `kind`, `target`, `title`, `row` and `mentions` are compared.
