"""Demo content for scripts/demo/gen.py: nine fictional projects, their tabs, panes and conversations.

All names, code and data here are made up. Turn forms:
  ("u", text)                      user message
  ("a", markdown)                  assistant message
  ("bash", cmd, output[, error])   shell command; output None = still running (or waiting for approval)
  ("read", path, body)  ("edit", path, old, new)  ("write", path, body)  ("grep", pattern, output)
  ("glob", pattern, output)        file search
  Claude only: ("u", text, [picture, ...])  a user message with pasted pictures (files in scripts/demo/pictures/)
               ("readimg", path, picture)   a Read of an image file
  Claude only: ("tool", name, input, output)       any other tool call (an Artifact publish)
  Claude only: ("task", description, prompt, result)  ("todo", [(text, status)])  ("fetch", url, prompt, result)
               ("ask", questions, answers)  an answered AskUserQuestion
Agent status: idle | working | done | question | approval. Codex, working: "queued" = follow-ups waiting on its screen. `start`/`end`: minutes ago of the first/last turn.
Prompts answer keys (Claude only; see gen.interactive):
  approval: the last turn is ("bash", cmd, None); "allow" = {"output", "then": [turns after it], "verb"}.
  question: "question" = one single-choice question; "answers" = {option label: follow-up text}.
"""

import resdemo  # the Resources demo agent (lighthouse-audit-docs); scripts/demo is on the path

# ------------------------------------------------------------------ Orchard: storefront web app

ORCHARD_API = [
    ("u", "Order totals are off by a cent on some multi-item carts. Find the cause in orchard/pricing.py and fix it with a test."),
    ("a", "I'll read the pricing module and the existing tests first."),
    ("read", "orchard/pricing.py", '''from decimal import Decimal

TAX_RATE = 0.0825


def line_total(price, qty):
    return round(price * qty, 2)


def order_total(lines):
    subtotal = sum(line_total(l["price"], l["qty"]) for l in lines)
    return round(subtotal * (1 + TAX_RATE), 2)'''),
    ("grep", "order_total", '''orchard/pricing.py:10:def order_total(lines):
orchard/checkout.py:41:    total = order_total(cart.lines)
tests/test_pricing.py:8:def test_order_total_single_line():'''),
    ("bash", "python -c \"from orchard.pricing import order_total; print(order_total([{'price': 19.99, 'qty': 3}, {'price': 4.35, 'qty': 1}]))\"", "69.65"),
    ("a", '''Found it. Prices are floats, and every line is rounded before tax is applied, so the error adds up.

| Cart | Expected | Got |
|---|---|---|
| 3 × 19.99 + 1 × 4.35 | 69.66 | 69.65 |
| 7 × 0.15 | 1.14 | 1.13 |

The fix: use `Decimal` end to end and round once, with `ROUND_HALF_UP`, at the very end.'''),
    ("edit", "orchard/pricing.py", '''TAX_RATE = 0.0825


def line_total(price, qty):
    return round(price * qty, 2)


def order_total(lines):
    subtotal = sum(line_total(l["price"], l["qty"]) for l in lines)
    return round(subtotal * (1 + TAX_RATE), 2)''', '''TAX_RATE = Decimal("0.0825")
CENT = Decimal("0.01")


def line_total(price, qty):
    return Decimal(str(price)) * qty


def order_total(lines):
    subtotal = sum((line_total(l["price"], l["qty"]) for l in lines), Decimal(0))
    return (subtotal * (1 + TAX_RATE)).quantize(CENT, rounding=ROUND_HALF_UP)'''),
    ("edit", "orchard/pricing.py", "from decimal import Decimal", "from decimal import ROUND_HALF_UP, Decimal"),
    ("edit", "tests/test_pricing.py", "def test_order_total_single_line():", '''@pytest.mark.parametrize("lines, want", [
    ([{"price": 19.99, "qty": 3}, {"price": 4.35, "qty": 1}], Decimal("69.66")),
    ([{"price": 0.15, "qty": 7}], Decimal("1.14")),
])
def test_order_total_rounds_once(lines, want):
    assert order_total(lines) == want


def test_order_total_single_line():'''),
    ("bash", "pytest -q tests/test_pricing.py", '''.....                                                                    [100%]
5 passed in 0.21s'''),
    ("bash", "pytest -q", '''........................................                                 [100%]
40 passed in 1.84s'''),
    ("a", '''Fixed. Totals now use `Decimal` and round once at the end.

- `orchard/pricing.py`: `Decimal` prices, one `quantize` with `ROUND_HALF_UP`.
- `tests/test_pricing.py`: two regression carts from the table above.

All 40 tests pass. `checkout.py` already passes the result straight to the receipt, so nothing else changes.'''),
    ("u", "Nice. Does the receipt template still format it right?"),
    ("grep", "total", '''orchard/templates/receipt.html:22:  <td class="amount">{{ total | money }}</td>
orchard/filters.py:5:def money(value):'''),
    ("read", "orchard/filters.py", '''def money(value):
    return f"${value:,.2f}"'''),
    ("a", "Yes. `money` uses a format spec, and `Decimal` supports `:,.2f` the same way a float does. `$1,204.50` renders exactly as before."),
]

ORCHARD_REVIEW = [
    ("u", "Review the pricing fix on this branch. Only report correctness problems."),
    ("bash", "git diff main --stat", ''' orchard/pricing.py     | 12 +++++++-----
 tests/test_pricing.py  |  9 +++++++++
 2 files changed, 16 insertions(+), 5 deletions(-)'''),
    ("read", "orchard/pricing.py", '''from decimal import ROUND_HALF_UP, Decimal

TAX_RATE = Decimal("0.0825")
CENT = Decimal("0.01")


def line_total(price, qty):
    return Decimal(str(price)) * qty


def order_total(lines):
    subtotal = sum((line_total(l["price"], l["qty"]) for l in lines), Decimal(0))
    return (subtotal * (1 + TAX_RATE)).quantize(CENT, rounding=ROUND_HALF_UP)'''),
    ("grep", "order_total(", '''orchard/checkout.py:41:    total = order_total(cart.lines)
orchard/api/quotes.py:18:    return {"total": order_total(lines)}'''),
    ("read", "orchard/api/quotes.py", '''from flask import jsonify

from orchard.pricing import order_total


def quote(lines):
    return {"total": order_total(lines)}'''),
    ("bash", "pytest -q tests/api/test_quotes.py", '''F
FAILED tests/api/test_quotes.py::test_quote_json - TypeError: Object of type Decimal is not JSON serializable
1 failed in 0.33s''', True),
    ("a", '''One P1:

**`/api/quote` now returns a `Decimal`, and the JSON encoder rejects it.** `quotes.py:18` puts the result of `order_total` straight into the response. `test_quote_json` fails with `TypeError: Object of type Decimal is not JSON serializable`.

Fix options:
1. Return `str(total)` from the quote endpoint (keeps exact cents; clients already parse strings for `subtotal`).
2. Register a JSON provider that encodes `Decimal` as a string.

No other findings. The rounding change itself is correct.'''),
]

ORCHARD_UI = [
    ("u", "Build the new cart drawer: slides in from the right, lists items with qty steppers, shows the total, and has a Checkout button. Use the existing Button and Stepper components."),
    ("todo", [("Read the existing components", "completed"), ("Create CartDrawer.tsx", "in_progress"),
              ("Wire it into the header", "pending"), ("Add tests", "pending")]),
    ("glob", "web/src/components/**/*.tsx", '''web/src/components/Button.tsx
web/src/components/Stepper.tsx
web/src/components/Header.tsx
web/src/components/ProductCard.tsx'''),
    ("read", "web/src/components/Stepper.tsx", '''type Props = { value: number; min?: number; onChange: (v: number) => void };

export function Stepper({ value, min = 0, onChange }: Props) {
  return (
    <div className="stepper">
      <button aria-label="Less" onClick={() => onChange(Math.max(min, value - 1))}>−</button>
      <span>{value}</span>
      <button aria-label="More" onClick={() => onChange(value + 1)}>+</button>
    </div>
  );
}'''),
    ("write", "web/src/components/CartDrawer.tsx", '''import { Button } from "./Button";
import { Stepper } from "./Stepper";
import { useCart } from "../cart/useCart";

export function CartDrawer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { lines, setQty, total } = useCart();
  return (
    <aside className={open ? "drawer drawer--open" : "drawer"} aria-hidden={!open}>
      <header>
        <h2>Your cart</h2>
        <button aria-label="Close" onClick={onClose}>×</button>
      </header>
      <ul>
        {lines.map((l) => (
          <li key={l.sku}>
            <span>{l.name}</span>
            <Stepper value={l.qty} min={0} onChange={(q) => setQty(l.sku, q)} />
          </li>
        ))}
      </ul>
      <footer>
        <strong>{total}</strong>
        <Button variant="primary" disabled={lines.length === 0}>Checkout</Button>
      </footer>
    </aside>
  );
}'''),
    ("edit", "web/src/components/Header.tsx", "      <CartIcon count={count} />", '''      <CartIcon count={count} onClick={() => setOpen(true)} />
      <CartDrawer open={open} onClose={() => setOpen(false)} />'''),
    ("bash", "npm run typecheck", '''> web@0.9.0 typecheck
> tsc --noEmit

src/components/Header.tsx:14:7 - error TS2304: Cannot find name 'setOpen'.''', True),
    ("edit", "web/src/components/Header.tsx", "  const count = useCartCount();", '''  const count = useCartCount();
  const [open, setOpen] = useState(false);'''),
    ("bash", "npm run typecheck", '''> web@0.9.0 typecheck
> tsc --noEmit'''),
    ("a", "Typecheck is clean. Now I'll write tests for the drawer: open and close, qty changes, and the disabled Checkout button on an empty cart."),
    ("bash", "npx vitest run src/components/CartDrawer.test.tsx", None),
]

ORCHARD_E2E = [
    ("u", "The checkout e2e test is flaky on CI. Make it stable."),
    ("bash", "npx playwright test checkout.spec.ts --repeat-each 20", '''Running 20 tests using 4 workers
  3 failed
    [chromium] › checkout.spec.ts:12:5 › guest checkout › pays with test card
  17 passed (48.1s)'''),
    ("read", "web/e2e/checkout.spec.ts", '''test("pays with test card", async ({ page }) => {
  await page.goto("/cart");
  await page.click("text=Checkout");
  await page.fill("#card", "4242 4242 4242 4242");
  await page.waitForTimeout(500);
  await page.click("text=Pay");
  await expect(page.locator(".receipt")).toBeVisible();
});'''),
    ("a", "The test waits a fixed 500 ms for the card iframe. On a slow runner the iframe is not ready yet, so the Pay click goes nowhere. I'll wait for the frame's ready state instead."),
    ("edit", "web/e2e/checkout.spec.ts", '''  await page.fill("#card", "4242 4242 4242 4242");
  await page.waitForTimeout(500);''', '''  const card = page.frameLocator("#card-frame");
  await card.locator("input[name=number]").fill("4242 4242 4242 4242");
  await expect(page.locator("[data-card-ready=true]")).toBeVisible();'''),
    ("bash", "npx playwright test checkout.spec.ts --repeat-each 40", '''Running 40 tests using 4 workers
  40 passed (1.3m)'''),
    ("a", "Stable now: 40 of 40 runs pass. The fixed sleep is gone; the test waits for the card frame to say it is ready."),
]

# ------------------------------------------------------------------ Atlas: harvest data pipeline

ATLAS_IMPORT = [
    ("u", "The nightly harvest import dies on one malformed row and the whole batch is lost. Make the importer skip bad rows, report them, and keep going."),
    ("read", "atlas/importer.py", '''import csv

from atlas.db import insert_harvest


def run(path):
    with open(path) as f:
        for row in csv.DictReader(f):
            insert_harvest(
                farm=row["farm_id"],
                crop=row["crop"],
                kg=float(row["kg"]),
                picked=row["picked_on"],
            )'''),
    ("bash", "head -6 sample.csv", '''farm_id,crop,kg,picked_on
F-104,apples,412.5,2026-10-01
F-104,pears,88,2026-10-01
F-221,apples,,2026-10-02
F-309,plums,n/a,2026-10-02
F-221,apples,301.25,2026-10-03'''),
    ("bash", "python -m atlas.importer sample.csv", '''Traceback (most recent call last):
  File "atlas/importer.py", line 11, in run
    kg=float(row["kg"]),
ValueError: could not convert string to float: \'\'''', True),
    ("a", "Two kinds of bad rows in the sample: empty `kg` and `n/a`. I'll validate each row, collect the rejects with the reason and line number, and write them to a rejects file next to the input."),
    ("edit", "atlas/importer.py", '''def run(path):
    with open(path) as f:
        for row in csv.DictReader(f):
            insert_harvest(
                farm=row["farm_id"],
                crop=row["crop"],
                kg=float(row["kg"]),
                picked=row["picked_on"],
            )''', '''def parse(row):
    try:
        kg = float(row["kg"])
    except ValueError:
        raise BadRow(f"kg is not a number: {row['kg']!r}")
    if kg <= 0:
        raise BadRow(f"kg must be positive: {kg}")
    return dict(farm=row["farm_id"], crop=row["crop"], kg=kg, picked=row["picked_on"])


def run(path):
    ok, rejects = 0, []
    with open(path) as f:
        for line, row in enumerate(csv.DictReader(f), start=2):
            try:
                insert_harvest(**parse(row))
                ok += 1
            except BadRow as e:
                rejects.append((line, str(e)))
    write_rejects(path, rejects)
    return ok, rejects'''),
    ("bash", "python -m atlas.importer sample.csv", '''imported 1,204 rows, rejected 3 (see sample.rejects.csv)'''),
    ("bash", "cat sample.rejects.csv", '''line,reason
4,kg is not a number: ''
5,kg is not a number: 'n/a'
917,kg must be positive: -12.0'''),
    ("bash", "pytest -q tests/test_importer.py", '''........                                                                 [100%]
8 passed in 0.40s'''),
    ("a", '''Done. The importer now keeps going past bad rows.

| | Before | After |
|---|---|---|
| Bad row | whole batch fails | row skipped, reason logged |
| Output | nothing | `<input>.rejects.csv` with line and reason |
| Exit code | 1 | 0, or 2 when more than 5% of rows are rejected |

On `sample.csv`: 1,204 imported, 3 rejected. I added tests for empty, `n/a` and negative weights.'''),
]

ATLAS_BACKFILL = [
    ("u", "Backfill September harvests from the archive bucket into the warehouse. Do it day by day so a failure only costs one day."),
    ("bash", "ls archive/2026-09 | head -5 && ls archive/2026-09 | wc -l", '''harvest-2026-09-01.csv.gz
harvest-2026-09-02.csv.gz
harvest-2026-09-03.csv.gz
harvest-2026-09-04.csv.gz
harvest-2026-09-05.csv.gz
30'''),
    ("write", "scripts/backfill.sh", '''#!/bin/sh
set -eu
for f in archive/2026-09/*.csv.gz; do
  day=$(basename "$f" .csv.gz)
  if grep -qx "$day" .backfill-done 2>/dev/null; then continue; fi
  gunzip -c "$f" > "/tmp/$day.csv"
  python -m atlas.importer "/tmp/$day.csv" && echo "$day" >> .backfill-done
done'''),
    ("bash", "sh scripts/backfill.sh 2>&1 | tail -4", '''harvest-2026-09-11: imported 1,388 rows, rejected 0
harvest-2026-09-12: imported 1,402 rows, rejected 1
harvest-2026-09-13: imported 1,377 rows, rejected 0
harvest-2026-09-14: imported 1,415 rows, rejected 2'''),
    ("a", "14 of 30 days are in. The script records each finished day in `.backfill-done`, so a rerun resumes where it stopped."),
    ("bash", "sh scripts/backfill.sh 2>&1 | tail -4", None),
]

ATLAS_SCHEMA = [
    ("u", "Add a `variety` column to harvests (e.g. Gala, Fuji). Nullable for old rows. Write the migration."),
    ("bash", "ls migrations | tail -3", '''0012_add_farm_region.sql
0013_index_picked_on.sql
0014_harvest_units.sql'''),
    ("write", "migrations/0015_harvest_variety.sql", '''ALTER TABLE harvests ADD COLUMN variety text;
CREATE INDEX harvests_crop_variety_idx ON harvests (crop, variety);'''),
    ("bash", "make migrate-test", '''applying 0015_harvest_variety.sql ... ok
schema check ... ok'''),
    ("a", "Migration `0015_harvest_variety.sql` adds a nullable `variety` text column and an index on `(crop, variety)`. It applies cleanly on the test database."),
]

ATLAS_SCHEMA_CHECK = [
    ("u", "Check migration 0015 against production table size. Will the index build lock writes?"),
    ("read", "migrations/0015_harvest_variety.sql", '''ALTER TABLE harvests ADD COLUMN variety text;
CREATE INDEX harvests_crop_variety_idx ON harvests (crop, variety);'''),
    ("bash", "psql $WAREHOUSE_RO -c \"select pg_size_pretty(pg_total_relation_size('harvests'))\"", ''' pg_size_pretty
----------------
 38 GB
(1 row)'''),
    ("a", "The table is 38 GB. A plain `CREATE INDEX` takes a lock that blocks writes for the whole build. It should be `CREATE INDEX CONCURRENTLY`, in its own migration, since that cannot run inside a transaction."),
    ("edit", "migrations/0015_harvest_variety.sql", "CREATE INDEX harvests_crop_variety_idx ON harvests (crop, variety);", "-- index moved to 0016 (CONCURRENTLY cannot run in a transaction)"),
    ("write", "migrations/0016_harvest_variety_index.sql", '''-- no-transaction
CREATE INDEX CONCURRENTLY harvests_crop_variety_idx ON harvests (crop, variety);'''),
    ("bash", "make migrate-test", None),
]

# ------------------------------------------------------------------ Harbor: deploy tooling

HARBOR_DEPLOY = [
    ("u", "Run scripts/deploy.sh staging as a dry run for the harbor service, then summarize what it would change."),
    ("read", "scripts/deploy.sh", '''#!/bin/sh
set -eu
env="$1"
echo "rendering manifests for $env"
harbor render --env "$env" > "build/$env.yaml"
harbor diff --env "$env" "build/$env.yaml"
[ "${DRY_RUN:-1}" = "1" ] && exit 0
harbor apply --env "$env" "build/$env.yaml"'''),
    ("a", "The script only diffs unless `DRY_RUN=0`, so the default run is safe. It still needs cluster credentials for `harbor diff`."),
    ("bash", "sh scripts/deploy.sh staging", None),
]

HARBOR_DEPLOY_ALLOW = {"output": '''rendering manifests for staging
~ deployment/harbor-api   image harbor:0.14.1 -> harbor:0.14.2
~ configmap/harbor-env    LOG_LEVEL info -> warn
+ cronjob/harbor-prune    schedule "0 3 * * *"
3 changes (2 modified, 1 added), dry run: nothing applied''', "verb": "Reading the diff", "then": [
    ("a", '''The dry run would change three things on staging. Nothing was applied.

| Resource | Change |
|---|---|
| `deployment/harbor-api` | image `0.14.1` → `0.14.2` |
| `configmap/harbor-env` | `LOG_LEVEL` `info` → `warn` |
| `cronjob/harbor-prune` | new, runs daily at 03:00 |

The image bump and the prune job are expected. The log level change is not in the release notes; check it is wanted before a real deploy.'''),
]}

HARBOR_CI = [
    ("u", '''CI broke on main after the dependency bump. Log below.

<pasted_content id="2207">
#14 [build 5/7] RUN cargo build --release --locked
#14 0.412 error: the lock file Cargo.lock needs to be updated but --locked was passed to prevent this
#14 0.412 If you want to try to generate the lock file without accessing the network, remove the --locked flag and use --offline instead.
#14 ERROR: process "/bin/sh -c cargo build --release --locked" did not complete successfully: exit code: 101
------
 > [build 5/7] RUN cargo build --release --locked:
------
ERROR: failed to solve: exit code: 101
</pasted_content id="2207">'''),
    ("a", "The bump PR changed `Cargo.toml` but not `Cargo.lock`, and the Docker build uses `--locked`. I'll regenerate the lock file and check that only the bumped crates move."),
    ("bash", "cargo update -p tokio -p serde_yaml && git diff --stat Cargo.lock", ''' Cargo.lock | 18 +++++++++---------
 1 file changed, 9 insertions(+), 9 deletions(-)'''),
    ("bash", "git diff Cargo.lock | grep '^[-+]version'", '''-version = "1.47.1"
+version = "1.48.0"
-version = "0.9.34"
+version = "0.9.35"'''),
    ("bash", "cargo build --release --locked 2>&1 | tail -2", '''   Compiling harbor v0.14.2
    Finished `release` profile [optimized] target(s) in 41.07s'''),
    ("bash", "cargo test --locked 2>&1 | tail -3", '''test result: ok. 112 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.81s'''),
    ("a", '''Fixed locally. Only the two bumped crates changed in `Cargo.lock`:

| crate | from | to |
|---|---|---|
| tokio | 1.47.1 | 1.48.0 |
| serde_yaml | 0.9.34 | 0.9.35 |

Release build and all 112 tests pass with `--locked`. Commit is ready as `Update Cargo.lock for the dependency bump`.'''),
]

HARBOR_RUNBOOK = [
    ("u", "Draft a rollback runbook for harbor deploys. Short, numbered steps."),
    ("bash", "harbor history --env production --limit 3", '''REV  DEPLOYED            IMAGE                   STATUS
41   2026-10-09 16:02    harbor:0.14.1           active
40   2026-10-02 11:47    harbor:0.14.0           superseded
39   2026-09-25 10:13    harbor:0.13.9           superseded'''),
    ("write", "docs/runbooks/rollback.md", '''# Rolling back a harbor deploy

1. Find the last good revision: `harbor history --env production --limit 5`.
2. Roll back: `harbor rollback --env production --to <rev>`.
3. Watch the rollout: `harbor status --env production --watch` until all pods are Ready.
4. Check the health page and the error rate panel for 10 minutes.
5. Post the revision and the reason in the deploy channel.'''),
    ("a", "Wrote `docs/runbooks/rollback.md`: five steps, from finding the last good revision to posting in the deploy channel. Each step has the exact command."),
]

# ------------------------------------------------------------------ Lighthouse: docs site

LIGHTHOUSE_START = [
    ("u", "We need a Getting Started page for the docs site. Ask me who it is for before you write anything."),
    ("a", "I'll look at the current docs first, so my recommendation fits what is already there."),
    ("glob", "docs/**/*.md", '''docs/index.md
docs/api/orchard.md
docs/api/atlas.md
docs/ops/harbor.md'''),
    ("read", "docs/index.md", '''# Lighthouse

Docs for the co-op services:

- Orchard: order pricing and checkout
- Atlas: harvest imports
- Harbor: deploys'''),
    ("a", "The index lists the three services but has no setup steps yet. Three audiences make sense, and they need very different pages, so I'll ask before writing."),
]

LIGHTHOUSE_START_Q = {
    "header": "Audience",
    "question": "Who is the Getting Started page for?",
    "options": [
        {"label": "New contributors (Recommended)", "description": "Developers joining the repo: layout, running the docs site locally, making a first change. The index has no setup steps, so this fills the biggest gap."},
        {"label": "Operators who deploy", "description": "People who run deploys: the Harbor flow, environment setup, and what to check after a deploy."},
        {"label": "API users", "description": "Consumers of the services: calling Orchard and Atlas, with request and response basics."},
    ],
}

LIGHTHOUSE_START_A = {
    "New contributors": "New contributors it is. I'll write `docs/getting-started.md`: clone, `make docs-serve`, the repo layout, and a first small change, then link it from the index.",
    "Operators who deploy": "Operators it is. I'll write the page around the Harbor flow: credentials, `scripts/deploy.sh` with a dry run first, and the checks after a deploy.",
    "API users": "API users it is. I'll start from the OpenAPI pages and write one worked request per service, with the auth header and a sample response.",
}

LIGHTHOUSE_API = [
    ("u", "Generate API reference pages for Orchard from the OpenAPI file. One page per tag."),
    ("bash", "yq '.tags[].name' orchard/openapi.yaml", '''carts
orders
quotes'''),
    ("bash", "python tools/openapi_to_md.py orchard/openapi.yaml --out docs/api/orchard", '''wrote docs/api/orchard/carts.md (4 endpoints)
wrote docs/api/orchard/orders.md (6 endpoints)
wrote docs/api/orchard/quotes.md (1 endpoint)'''),
    ("read", "docs/api/orchard/quotes.md", '''# Quotes

## POST /api/quote

Returns the total for a list of lines, tax included.

| Field | Type | Notes |
|---|---|---|
| lines[].sku | string | required |
| lines[].qty | integer | ≥ 1 |
| total | string | decimal, two places |'''),
    ("bash", "mkdocs build --strict 2>&1 | tail -2", '''INFO    -  Documentation built in 1.92 seconds'''),
    ("a", "Three pages generated under `docs/api/orchard/` (carts, orders, quotes; 11 endpoints). `mkdocs build --strict` passes, so there are no broken links."),
]

LIGHTHOUSE_LINKS = [
    ("u", "Check the docs for dead external links."),
    ("bash", "lychee --no-progress docs/", '''✗ [404] https://example.org/old-pricing-guide | docs/api/orchard.md
✗ [TIMEOUT] https://status.example.net/ | docs/ops/harbor.md

🔍 48 Total ✅ 46 OK 🚫 2 Errors'''),
    ("a", '''Two problems out of 48 links:

| Link | Page | Problem |
|---|---|---|
| example.org/old-pricing-guide | api/orchard.md | 404, page removed |
| status.example.net | ops/harbor.md | timeout |

The status page may just be slow; the pricing guide link is really gone. Want me to remove it?'''),
]

# ------------------------------------------------------------------ Tidepool: log tailing CLI (Rust)

TIDEPOOL_PARSER = [
    ("u", "tidepool chokes on JSON log lines that contain escaped newlines. Fix the line parser in src/parse.rs."),
    ("read", "src/parse.rs", '''pub fn split_records(buf: &str) -> Vec<&str> {
    buf.split('\\n').filter(|l| !l.is_empty()).collect()
}'''),
    ("bash", "cargo test parse 2>&1 | tail -6", '''running 4 tests
test parse::tests::plain_lines ... ok
test parse::tests::empty_input ... ok
test parse::tests::json_escaped_newline ... FAILED
test parse::tests::crlf ... ok'''),
    ("a", '''The splitter is fine: `\\n` inside a JSON string is two characters, not a newline. The bug is one step later, in `render`, which unescapes before it splits for display. I'll move the unescape after the split.'''),
    ("grep", "unescape", '''src/render.rs:31:    let text = unescape(raw);
src/render.rs:32:    for line in text.lines() {'''),
    ("edit", "src/render.rs", '''    let text = unescape(raw);
    for line in text.lines() {''', '''    for line in raw.lines().map(unescape) {'''),
    ("bash", "cargo test 2>&1 | tail -2", '''test result: ok. 37 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s'''),
    ("bash", "cargo bench --bench tail -- --quick 2>&1 | grep time", '''tail/1MB                time:   [3.912 ms 3.947 ms 3.990 ms]'''),
    ("a", "Fixed in `render.rs`: unescape now runs per displayed line. All 37 tests pass, and the 1 MB tail benchmark is unchanged at about 3.9 ms."),
    ("u", "Now add a --since flag that takes 10m, 2h, or an RFC 3339 time."),
    ("read", "src/cli.rs", '''#[derive(Parser)]
pub struct Args {
    /// Files to follow
    pub files: Vec<PathBuf>,
    /// Only lines matching this regex
    #[arg(short, long)]
    pub grep: Option<String>,
}'''),
    ("edit", "src/cli.rs", '''    #[arg(short, long)]
    pub grep: Option<String>,
}''', '''    #[arg(short, long)]
    pub grep: Option<String>,
    /// Only lines newer than this: 10m, 2h, or an RFC 3339 time
    #[arg(long, value_parser = parse_since)]
    pub since: Option<OffsetDateTime>,
}'''),
    ("bash", "cargo test since 2>&1 | tail -3", None),
]

TIDEPOOL_FUZZ = [
    ("u", "Run the parser fuzzer for a few minutes and triage anything it finds."),
    ("bash", "cargo fuzz run parse_line -- -max_total_time=180 2>&1 | tail -3", '''#1048576 pulse  cov: 912 ft: 2214 corp: 301/18Kb exec/s: 5825
Done 1048576 runs in 180 second(s)
INFO: no crashes found'''),
    ("a", "Three minutes, about a million runs, no crashes. Coverage grew from 870 to 912 edges with the new corpus."),
]

TIDEPOOL_DOCS = [
    ("u", "Update the README usage section for the new --since flag."),
    ("read", "README.md", '''## Usage

    tidepool app.log
    tidepool --grep ERROR *.log'''),
    ("edit", "README.md", "    tidepool --grep ERROR *.log", '''    tidepool --grep ERROR *.log
    tidepool --since 10m app.log
    tidepool --since 2026-10-10T08:00:00Z app.log'''),
    ("a", "README updated with two `--since` examples: a relative time and an RFC 3339 time."),
]

TIDEPOOL_RELEASE = [
    ("u", "Prepare the 0.6.0 release notes from the commits since 0.5.2."),
    ("bash", "git log --oneline v0.5.2..HEAD", '''a91c2e4 Add --since flag
5d03b7f Unescape per displayed line (fixes escaped newlines)
c22e810 Follow rotated files by inode
7f3a9d1 Color levels in --json mode
1e0b4c6 Bump clap to 4.6'''),
    ("write", "CHANGELOG.md", '''## 0.6.0

### Added
- `--since 10m|2h|<RFC 3339>` shows only newer lines.
- Level colors in `--json` mode.

### Fixed
- JSON lines with escaped newlines render correctly.
- Rotated files are followed by inode, so `logrotate` no longer stops the tail.'''),
    ("a", "Release notes for 0.6.0 are in `CHANGELOG.md`: two additions, two fixes. The clap bump is internal, so I left it out."),
]

# ------------------------------------------------------------------ Quarry: search service

QUARRY_RANK = [
    ("u", "Search results for short queries like 'gala' rank old listings above new ones. Look at the ranking and propose a fix."),
    ("task", "Find the ranking code", "Find where search results are scored and sorted in the quarry service. Report file paths and the scoring formula.", '''Scoring lives in quarry/rank.py: score = bm25 * 0.8 + popularity * 0.2. Popularity is total clicks, never decayed. Results are sorted in quarry/search.py:57.'''),
    ("read", "quarry/rank.py", '''def score(doc, bm25):
    return bm25 * 0.8 + doc.popularity * 0.2'''),
    ("bash", "python -m quarry.explain 'gala' --top 5", '''rank  listing                       bm25   popularity  score  age
1     Gala apples, 10 kg (2024)     4.10   912         184.7  731d
2     Gala crate, wholesale (2025)  3.95   604         123.9  402d
3     Gala apples, new season       4.30   12          5.8    3d
4     Royal Gala, organic           3.70   9           4.8    6d
5     Gala seedlings                2.10   40          9.7    90d'''),
    ("a", '''The cause is clear in the table: popularity is raw click count, so it swamps text relevance. A 2-year-old listing with 912 clicks beats a better match from this week.

Proposal:
1. Use `log1p(clicks)` so popularity cannot dominate.
2. Decay clicks with a 30-day half-life.

```python
def score(doc, bm25, now):
    age_days = (now - doc.created).days
    fresh = 0.5 ** (age_days / 30)
    return bm25 * 0.8 + math.log1p(doc.clicks * fresh) * 0.2
```'''),
    ("u", "Try it and show me the same query."),
    ("edit", "quarry/rank.py", '''def score(doc, bm25):
    return bm25 * 0.8 + doc.popularity * 0.2''', '''def score(doc, bm25, now):
    fresh = 0.5 ** ((now - doc.created).days / HALF_LIFE_DAYS)
    return bm25 * 0.8 + math.log1p(doc.clicks * fresh) * 0.2'''),
    ("bash", "python -m quarry.explain 'gala' --top 5", '''rank  listing                       bm25   clicks  score  age
1     Gala apples, new season       4.30   12      4.01   3d
2     Gala apples, 10 kg (2024)     4.10   912     3.29   731d
3     Royal Gala, organic           3.70   9       3.42   6d
4     Gala crate, wholesale (2025)  3.95   604     3.17   402d
5     Gala seedlings                2.10   40      2.06   90d'''),
    ("a", '''The new-season listing is first now, and the old bestseller still ranks second on relevance.

| Listing | Rank before | Rank after |
|---|---|---|
| Gala apples, new season | 3 | **1** |
| Gala apples, 10 kg (2024) | 1 | 2 |
| Royal Gala, organic | 4 | 3 |

Next I'll replay last week's query log to check the change does not hurt long queries.'''),
    ("bash", "python -m quarry.replay logs/queries-2026-w40.jsonl --metric ndcg@10", None),
]

QUARRY_BENCH = [
    ("u", "Benchmark the index build on the full listings dump."),
    ("bash", "make bench-index", '''building index from dumps/listings-2026-10-01.jsonl (2.4M docs)
  tokenize   41.2s
  postings   63.8s
  write      12.5s
total 117.5s, index 1.9 GB'''),
    ("a", "Full build takes 117.5 s for 2.4M listings. Postings is the slowest phase at 64 s; I'll profile it next."),
    ("bash", "py-spy record -o build/postings.svg -- python -m quarry.index --phase postings", None),
]

QUARRY_NOTES = [
    ("u", "Summarize what changed in quarry this week for the team update. Three bullets."),
    ("bash", "git log --since=7.days --oneline", '''e41a0b2 Decay popularity with a 30-day half-life
8b7d2c9 Explain tool prints per-term scores
33f0e15 Cap synonym expansion at 5 terms'''),
    ("a", '''- **Ranking:** popularity now decays with a 30-day half-life, so new listings can outrank old bestsellers.
- **Debugging:** `quarry.explain` prints per-term scores for any query.
- **Synonyms:** expansion is capped at 5 terms, which cut p95 latency on long queries.'''),
]

# ------------------------------------------------------------------ more Orchard / Harbor tabs

ORCHARD_A11Y = [
    ("u", "The cart drawer fails the accessibility check: focus is not trapped and the close button has no label in Safari. Fix both and refresh the a11y snapshots."),
    ("bash", "npx playwright test a11y.spec.ts", '''Running 6 tests using 3 workers
  2 failed
    [webkit] › a11y.spec.ts:21:3 › cart drawer › traps focus
    [webkit] › a11y.spec.ts:34:3 › cart drawer › close button has a name
  4 passed (9.8s)''', True),
    ("read", "web/src/components/CartDrawer.tsx", '''<aside className={open ? "drawer drawer--open" : "drawer"} aria-hidden={!open}>
  <header>
    <h2>Your cart</h2>
    <button onClick={onClose}>×</button>
  </header>'''),
    ("edit", "web/src/components/CartDrawer.tsx", '''<aside className={open ? "drawer drawer--open" : "drawer"} aria-hidden={!open}>''',
     '''<aside className={open ? "drawer drawer--open" : "drawer"} role="dialog" aria-modal="true" aria-labelledby="cart-title" ref={trapRef}>'''),
    ("edit", "web/src/components/CartDrawer.tsx", "    <button onClick={onClose}>×</button>", '''    <button aria-label="Close cart" onClick={onClose}>×</button>'''),
    ("a", "Both fixes are in: the drawer is a modal dialog with a focus trap, and the close button has an accessible name. The two snapshot baselines are now out of date, so I'll regenerate them."),
    ("bash", "npx playwright test a11y.spec.ts --update-snapshots", None),
]

ORCHARD_A11Y_ALLOW = {"output": '''Running 6 tests using 3 workers
  6 passed (10.4s)
  2 snapshots updated: cart-drawer-open.aria.yml, cart-drawer-empty.aria.yml''', "verb": "Updating snapshots", "then": [
    ("a", '''All 6 accessibility tests pass on Chromium, Firefox and WebKit.

- `CartDrawer.tsx`: `role="dialog"`, `aria-modal`, focus trap, and a labelled close button.
- Two aria snapshots regenerated. The diff only adds the dialog role and the button name.'''),
]}

ORCHARD_PROMO = [
    ("u", "Add promo codes to checkout. Codes can be a percentage or a fixed amount. Some carts will have two codes."),
    ("read", "orchard/pricing.py", '''def order_total(lines):
    subtotal = sum((line_total(l["price"], l["qty"]) for l in lines), Decimal(0))
    return (subtotal * (1 + TAX_RATE)).quantize(CENT, rounding=ROUND_HALF_UP)'''),
    ("grep", "promo", '''orchard/models.py:88:class PromoCode(Model):
orchard/models.py:91:    kind = Enum("percent", "fixed")'''),
    ("a", "The model already has `PromoCode` with a percent or fixed kind. The open question is how two codes combine, and it changes the totals, so I'll ask before I write the code."),
]

ORCHARD_PROMO_Q = {
    "header": "Stacking",
    "question": "When a cart has two promo codes, how should they combine?",
    "options": [
        {"label": "Best single code (Recommended)", "description": "Apply only the code that saves the most. Simple to explain, and it never stacks two discounts by accident."},
        {"label": "Fixed first, then percent", "description": "Subtract fixed amounts, then apply percentages to what is left. Common in shops; slightly smaller discount."},
        {"label": "Percent first, then fixed", "description": "Apply percentages to the full subtotal, then subtract fixed amounts. Gives the customer the larger discount."},
    ],
}

ORCHARD_PROMO_A = {
    "Best single code": "Best single code it is. I'll add `best_discount(lines, codes)` to `pricing.py`, apply it before tax, and add tests for a percent code beating a fixed one and the reverse.",
    "Fixed first, then percent": "Fixed first, then percent. I'll apply fixed codes to the subtotal, then percentages to the remainder, never below zero, and test a cart with one of each.",
    "Percent first, then fixed": "Percent first, then fixed. I'll apply percentages to the full subtotal, then fixed amounts, clamp at zero, and add a test with both kinds.",
}

HARBOR_SECRETS = [
    ("u", "The staging database password is 90 days old. Rotate it with the harbor CLI and make sure the API pods pick up the new one."),
    ("bash", "harbor secrets list --env staging", '''NAME          AGE    USED BY
db-password   92d    harbor-api, harbor-worker
smtp-token    31d    harbor-mailer
signing-key   12d    harbor-api'''),
    ("a", "`db-password` is 92 days old and used by `harbor-api` and `harbor-worker`. `harbor secrets rotate` writes a new value and restarts the pods that mount it, one at a time."),
    ("bash", "harbor secrets rotate --env staging --key db-password --restart", None),
]

HARBOR_SECRETS_ALLOW = {"output": '''rotating db-password in staging
  new version v7 written
  restarting harbor-api (3 pods) ... ok
  restarting harbor-worker (2 pods) ... ok
old version v6 disabled''', "verb": "Watching the restart", "then": [
    ("a", "Rotated. `db-password` is now version 7 on staging; all 5 pods restarted one at a time and are Ready. Version 6 is disabled, not deleted, so a rollback is one command if something still uses it."),
]}

HARBOR_CANARY = [
    ("u", "Add a canary step to the production deploy: 10% of traffic for 15 minutes, roll back automatically if the error rate doubles."),
    ("read", "deploy/production.yaml", '''strategy:
  type: rolling
  maxUnavailable: 1'''),
    ("edit", "deploy/production.yaml", '''strategy:
  type: rolling
  maxUnavailable: 1''', '''strategy:
  type: canary
  steps:
    - weight: 10
      pause: 15m
      abortIf: error_rate > 2 * baseline
    - weight: 100'''),
    ("bash", "harbor validate deploy/production.yaml", "ok: 1 file, strategy canary (2 steps)"),
    ("bash", "harbor simulate --env production --fail-at step1", None),
]

# ------------------------------------------------------------------ Meadow: iOS app

MEADOW_ONBOARD = [
    ("u", "Rework the onboarding flow in the Meadow app. Three screens max, then sign in."),
    ("glob", "Meadow/Onboarding/*.swift", '''Meadow/Onboarding/OnboardingView.swift
Meadow/Onboarding/WelcomePage.swift
Meadow/Onboarding/PermissionsPage.swift
Meadow/Onboarding/SignInPage.swift'''),
    ("read", "Meadow/Onboarding/SignInPage.swift", '''struct SignInPage: View {
    var body: some View {
        VStack(spacing: 16) {
            EmailField()
            PasswordField()
            Button("Sign in") { model.signIn() }
        }
    }
}'''),
    ("a", "Today sign-in is email and password only. The new flow ends on the sign-in screen, so which methods it offers decides its layout. I'll ask first."),
]

MEADOW_ONBOARD_Q = {
    "header": "Sign in",
    "question": "Which sign-in methods should the last onboarding screen offer?",
    "options": [
        {"label": "Sign in with Apple + email (Recommended)", "description": "Apple first, email link as the fallback. No passwords to store, and App Review expects Apple when other social logins exist."},
        {"label": "Email link only", "description": "One field, a magic link by email. The simplest screen, but every sign-in needs the mail app."},
        {"label": "Keep email + password", "description": "No backend change. The screen stays as it is today, only restyled."},
    ],
}

MEADOW_ONBOARD_A = {
    "Sign in with Apple + email": "Apple plus email link. I'll put `SignInWithAppleButton` at the top of `SignInPage`, the email link below it, and cut onboarding to Welcome, Permissions, Sign in.",
    "Email link only": "Email link only. I'll replace both fields with one email field and a Send link button, and handle the link in `MeadowApp.onOpenURL`.",
    "Keep email + password": "Keeping email and password. I'll restyle `SignInPage` to match the new onboarding pages and leave the auth calls untouched.",
}

MEADOW_CARD = [
    ("u", "The harvest card is cramped on the iPhone SE: the total wraps under the number and the chart is cut off after Saturday. Can you fix the layout?",
     ["card-before.png"]),
    ("read", "Meadow/Views/HarvestCard.swift", '''struct HarvestCard: View {
    let total: Measurement<UnitMass>
    let week: [DayHarvest]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Today").font(.caption.weight(.semibold)).textCase(.uppercase)
            HStack(alignment: .firstTextBaseline) {
                Text(total.value, format: .number).font(.system(size: 54, weight: .bold))
                Text("kg").font(.title3.weight(.semibold))
            }
            .frame(width: 150)
            Text(delta).foregroundStyle(.green)
            HStack(spacing: 14) {
                ForEach(week) { day in Bar(day: day).frame(width: 34) }
            }
        }
    }
}'''),
    ("a", '''Two fixed sizes cause both problems:

- The total is 54 pt in a 150 pt frame, so "kg" has no room and wraps.
- Each bar is 34 pt wide with 14 pt gaps: seven days need 320 pt, more than the card has on a 320 pt screen.

I'll let the bars share the width and give the number a smaller size that scales with Dynamic Type.'''),
    ("edit", "Meadow/Views/HarvestCard.swift", '''                Text(total.value, format: .number).font(.system(size: 54, weight: .bold))
                Text("kg").font(.title3.weight(.semibold))
            }
            .frame(width: 150)''', '''                Text(total.value, format: .number).font(.largeTitle.weight(.bold))
                Text("kg").font(.title3.weight(.semibold))
            }
            .lineLimit(1)
            .minimumScaleFactor(0.7)'''),
    ("edit", "Meadow/Views/HarvestCard.swift", '''            HStack(spacing: 14) {
                ForEach(week) { day in Bar(day: day).frame(width: 34) }
            }''', '''            HStack(spacing: 8) {
                ForEach(week) { day in Bar(day: day).frame(maxWidth: .infinity) }
            }'''),
    ("bash", "xcrun simctl io booted screenshot /tmp/harvest-card.png", "Wrote screenshot to: /tmp/harvest-card.png", False,
     "Take a simulator screenshot"),
    ("readimg", "/tmp/harvest-card.png", "card-after.png"),
    ("a", '''Fixed. On the iPhone SE simulator the total and "kg" stay on one line, and all seven days fit: the bars share the card's width.

The pickup row was cut off for the same reason; it now shows the full time.'''),
]

MEADOW_WIDGETS = [
    ("u", "Add a small home screen widget that shows today's harvest total."),
    ("write", "MeadowWidget/HarvestWidget.swift", '''struct HarvestWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "harvest", provider: HarvestProvider()) { entry in
            HarvestView(entry: entry)
        }
        .supportedFamilies([.systemSmall])
    }
}'''),
    ("bash", "xcodebuild -scheme MeadowWidget -destination 'generic/platform=iOS Simulator' build 2>&1 | tail -1", "** BUILD SUCCEEDED **"),
    ("a", "The widget builds. Next: a timeline provider that refreshes every 30 minutes from the shared app group."),
    ("write", "MeadowWidget/HarvestProvider.swift", '''struct HarvestProvider: TimelineProvider {
    let store = HarvestStore(group: "group.meadow.shared")

    func getTimeline(in context: Context, completion: @escaping (Timeline<HarvestEntry>) -> Void) {
        let entry = HarvestEntry(date: .now, total: store.today().total)
        let next = Calendar.current.date(byAdding: .minute, value: 30, to: .now)!
        completion(Timeline(entries: [entry], policy: .after(next)))
    }
}'''),
    ("a", "The provider reads today's total from the shared store and asks for a new timeline every 30 minutes. "
          "The app already writes the store after each pickup, so the widget is never more than half an hour behind. "
          "Next I'll run the widget tests on the simulator."),
]

MEADOW_CRASH = [
    ("u", "Crash reports show a crash on launch for users upgrading from 2.3. Find it."),
    ("bash", "grep -A4 'Thread 0 Crashed' crashes/2.4.0-launch.crash | head -6", '''Thread 0 Crashed:
0   Meadow    HarvestStore.migrate(from:) + 212
1   Meadow    HarvestStore.init() + 96
2   Meadow    MeadowApp.init() + 48'''),
    ("read", "Meadow/Store/HarvestStore.swift", '''func migrate(from version: Int) {
    if version < 4 {
        let old = try! decoder.decode([LegacyHarvest].self, from: data)
        save(old.map(Harvest.init))
    }
}'''),
    ("a", "The 2.3 store can hold an empty file, and `try!` crashes on it. I'll decode with `try?` and treat an empty or unreadable store as no harvests."),
    ("edit", "Meadow/Store/HarvestStore.swift", "        let old = try! decoder.decode([LegacyHarvest].self, from: data)", "        let old = (try? decoder.decode([LegacyHarvest].self, from: data)) ?? []"),
    ("bash", "xcodebuild test -scheme Meadow -only-testing MeadowTests/MigrationTests 2>&1 | tail -1", "** TEST SUCCEEDED **"),
    ("a", "Fixed. An empty 2.3 store no longer crashes the migration; it migrates to an empty list. I added a test with an empty legacy file."),
]

MEADOW_L10N = [
    ("u", "Find strings in the onboarding screens that are not localized."),
    ("bash", "rg -n 'Text\\(\"' Meadow/Onboarding", '''Meadow/Onboarding/WelcomePage.swift:12:        Text("Welcome to Meadow")
Meadow/Onboarding/PermissionsPage.swift:20:        Text("Allow notifications")'''),
    ("a", "Two hard-coded strings. Both are `Text` with a literal, which SwiftUI does localize, but neither key is in `Localizable.xcstrings` yet. I'll add them."),
]

# ------------------------------------------------------------------ Beacon: notification service (Go)

BEACON_RETRY = [
    ("u", "Push notifications that fail with a 429 are dropped. Add retries with backoff, and test it against a real Redis queue."),
    ("read", "internal/push/send.go", '''func (s *Sender) Send(ctx context.Context, n Notification) error {
	resp, err := s.client.Do(req(n))
	if err != nil {
		return err
	}
	if resp.StatusCode >= 400 {
		return fmt.Errorf("push failed: %d", resp.StatusCode)
	}
	return nil
}'''),
    ("edit", "internal/push/send.go", '''	if resp.StatusCode >= 400 {
		return fmt.Errorf("push failed: %d", resp.StatusCode)
	}''', '''	if resp.StatusCode == http.StatusTooManyRequests {
		return s.queue.Retry(ctx, n, backoff(n.Attempt, resp.Header.Get("Retry-After")))
	}
	if resp.StatusCode >= 400 {
		return fmt.Errorf("push failed: %d", resp.StatusCode)
	}'''),
    ("a", "Retries go back on the Redis queue with exponential backoff, and `Retry-After` wins when the server sends it. The integration test needs a local Redis, so I'll start one with Docker."),
    ("bash", "docker compose up -d redis", None),
]

BEACON_RETRY_ALLOW = {"output": '''[+] Running 2/2
 ✔ Network beacon_default  Created
 ✔ Container beacon-redis  Started''', "verb": "Running integration tests", "then": [
    ("a", '''Redis is up and the integration test passes.

| Case | Result |
|---|---|
| 429 without `Retry-After` | retried after 1 s, 2 s, 4 s |
| 429 with `Retry-After: 10` | retried after 10 s |
| 5 failed attempts | moved to the dead-letter list |'''),
]}

BEACON_DIGEST = [
    ("u", "Batch email notifications into one daily digest per user."),
    ("write", "internal/digest/digest.go", '''func Build(user string, items []Item) Email {
	sort.Slice(items, func(i, j int) bool { return items[i].At.Before(items[j].At) })
	return Email{To: user, Subject: fmt.Sprintf("%d updates today", len(items)), Items: items}
}'''),
    ("bash", "go test ./internal/digest/...", "ok  	beacon/internal/digest	0.214s"),
    ("a", "Digest builder done: one email per user per day, items in time order, subject with the count. Tests pass."),
]

BEACON_LOAD = [
    ("u", "Load test the send path at 2,000 notifications per second for five minutes."),
    ("bash", "k6 run --vus 200 --duration 5m load/send.js 2>&1 | tail -4", None),
]

# ------------------------------------------------------------------ Ledger: billing

LEDGER_INVOICE = [
    ("u", "Invoices for multi-currency accounts show the wrong totals. Look at how we convert and round."),
    ("read", "ledger/invoice.py", '''def invoice_total(lines, currency):
    return sum(convert(l.amount, l.currency, currency) for l in lines)'''),
    ("bash", "python -m ledger.explain INV-2026-0918", '''line  amount      from  rate      converted
1     120.00      EUR   1.0841    130.092
2     89.50       GBP   1.2710    113.7545
total (unrounded)                 243.8465'''),
    ("a", "Each line is converted at full precision and the total is never rounded, so the PDF shows 243.85 while the ledger stores 243.8465. Before I fix it: rounding per line or once at the end gives different totals, and that is a billing policy decision."),
]

LEDGER_INVOICE_Q = {
    "header": "Rounding",
    "question": "Where should currency conversion round on invoices?",
    "options": [
        {"label": "Per line (Recommended)", "description": "Round each converted line to cents, then add. Every line on the PDF adds up to the total, which is what customers check."},
        {"label": "Once at the end", "description": "Add at full precision, round the total. Slightly more exact, but lines may not add up to the total on the PDF."},
    ],
}

LEDGER_INVOICE_A = {
    "Per line": "Per line. I'll round each converted line with `ROUND_HALF_EVEN` to the currency's minor unit, sum the rounded lines, and add a test that the PDF lines add up to the total.",
    "Once at the end": "Once at the end. I'll keep lines at full precision, round only the total, and add a note on the PDF that lines are shown rounded.",
}

LEDGER_RECON = [
    ("u", "Reconcile September payouts against the bank export."),
    ("bash", "python -m ledger.reconcile payouts-2026-09.csv bank-2026-09.csv", '''matched     1,482
unmatched      3  (see reconcile-2026-09.csv)'''),
    ("a", "1,482 payouts match the bank export. Three do not; all three are the same partner with a one-day settlement delay, so they will match in the October export."),
]

# ------------------------------------------------------------------ layout

WORKSPACES = [
    {"label": "Orchard", "dir": "orchard", "about": "Storefront web app: cart, checkout and pricing.", "tabs": [
        {"label": "pricing", "panes": [
            {"name": "orchard-api-dev", "kind": "claude", "status": "idle", "title": "Fix rounding in order totals",
             "start": 160, "end": 95, "turns": ORCHARD_API},
            {"name": "orchard-api-review", "kind": "codex", "status": "done", "title": "Review pricing fix",
             "start": 40, "end": 6, "turns": ORCHARD_REVIEW, "split": "right"},
        ]},
        {"label": "cart drawer", "panes": [
            {"name": "orchard-ui-dev", "kind": "claude", "status": "working", "title": "Cart drawer component",
             "start": 35, "end": 0.5, "turns": ORCHARD_UI, "verb": "Running tests", "elapsed": "42s"},
        ]},
        {"label": "e2e", "panes": [
            {"name": "orchard-e2e-dev", "kind": "codex", "status": "done", "title": "Flaky checkout test",
             "start": 70, "end": 12, "turns": ORCHARD_E2E},
        ]},
        {"label": "a11y", "panes": [
            {"name": "orchard-a11y-dev", "kind": "claude", "status": "approval", "title": "Cart drawer accessibility",
             "start": 14, "end": 1, "turns": ORCHARD_A11Y, "approval_why": "Regenerate the a11y snapshot baselines",
             "allow": ORCHARD_A11Y_ALLOW},
        ]},
        {"label": "promo codes", "panes": [
            {"name": "orchard-promo-dev", "kind": "claude", "status": "question", "title": "Promo codes at checkout",
             "start": 9, "end": 2, "turns": ORCHARD_PROMO, "question": ORCHARD_PROMO_Q, "answers": ORCHARD_PROMO_A},
        ]},
    ]},
    {"label": "Atlas", "dir": "atlas", "about": "Harvest data pipeline: CSV imports into the warehouse.", "tabs": [
        {"label": "importer", "panes": [
            {"name": "atlas-import-dev", "kind": "claude", "status": "done", "title": "Skip malformed harvest rows",
             "start": 90, "end": 18, "turns": ATLAS_IMPORT},
        ]},
        {"label": "backfill", "panes": [
            {"name": "atlas-backfill-dev", "kind": "pi", "status": "working", "title": "September backfill",
             "start": 50, "end": 1, "turns": ATLAS_BACKFILL},
        ]},
        {"label": "schema", "panes": [
            {"name": "atlas-schema-dev", "kind": "claude", "status": "idle", "title": "Add variety column",
             "start": 200, "end": 150, "turns": ATLAS_SCHEMA},
            {"name": "atlas-schema-review", "kind": "codex", "status": "working", "title": "Check index lock",
             "start": 15, "end": 0.3, "turns": ATLAS_SCHEMA_CHECK, "split": "down", "elapsed": "1m 03s"},
        ]},
    ]},
    {"label": "Harbor", "dir": "harbor", "about": "Deploy tooling for the co-op services.", "tabs": [
        {"label": "deploy", "panes": [
            {"name": "harbor-deploy-ops", "kind": "claude", "status": "approval", "title": "Staging dry-run deploy",
             "start": 8, "end": 2, "turns": HARBOR_DEPLOY, "approval_why": "Dry-run deploy of harbor to staging",
             "allow": HARBOR_DEPLOY_ALLOW},
        ]},
        {"label": "ci", "panes": [
            {"name": "harbor-ci-dev", "kind": "codex", "status": "done", "title": "Fix locked build on main",
             "start": 55, "end": 22, "turns": HARBOR_CI},
        ]},
        {"label": "runbook", "panes": [
            {"name": "harbor-runbook-docs", "kind": "claude", "status": "idle", "title": "Rollback runbook",
             "start": 260, "end": 240, "turns": HARBOR_RUNBOOK, "focus": True},
        ]},
        {"label": "secrets", "panes": [
            {"name": "harbor-secrets-ops", "kind": "claude", "status": "approval", "title": "Rotate staging DB password",
             "start": 11, "end": 3, "turns": HARBOR_SECRETS, "approval_why": "Rotate db-password in staging and restart its pods",
             "allow": HARBOR_SECRETS_ALLOW},
        ]},
        {"label": "canary", "panes": [
            {"name": "harbor-canary-ops", "kind": "codex", "status": "working", "title": "Canary deploy step",
             "start": 24, "end": 0.5, "turns": HARBOR_CANARY, "elapsed": "1m 47s"},
        ]},
    ]},
    {"label": "Lighthouse", "dir": "lighthouse", "about": "Docs site for the co-op services.", "tabs": [
        {"label": "getting started", "panes": [
            {"name": "lighthouse-start-docs", "kind": "claude", "status": "question", "title": "Getting Started page",
             "start": 6, "end": 1, "turns": LIGHTHOUSE_START, "question": LIGHTHOUSE_START_Q,
             "answers": LIGHTHOUSE_START_A},
        ]},
        {"label": "api ref", "panes": [
            {"name": "lighthouse-api-docs", "kind": "pi", "status": "idle", "title": "Orchard API reference",
             "start": 180, "end": 140, "turns": LIGHTHOUSE_API},
            {"name": "lighthouse-links-check", "kind": "codex", "status": "idle", "title": "Dead link check",
             "start": 130, "end": 125, "turns": LIGHTHOUSE_LINKS, "split": "right"},
        ]},
        {"label": "docs audit", "panes": [
            # The Resources demo: every group, Markdown past 30 rows (scripts/demo/resdemo.py).
            {"name": "lighthouse-audit-docs", "kind": "claude", "status": "done", "title": "Docs audit before 2.0",
             "start": 150, "end": 6, "turns": resdemo.TURNS, "replies": resdemo.REPLIES},
        ]},
    ]},
    {"label": "Tidepool", "dir": "tidepool", "about": "A small Rust CLI that tails and filters log files.", "tabs": [
        {"label": "parser", "panes": [
            {"name": "tidepool-parse-dev", "kind": "claude", "status": "working", "title": "Add --since flag",
             "start": 45, "end": 0.2, "turns": TIDEPOOL_PARSER, "verb": "Compiling", "elapsed": "2m 31s"},
            {"name": "tidepool-fuzz-test", "kind": "codex", "status": "done", "title": "Fuzz the parser",
             "start": 25, "end": 9, "turns": TIDEPOOL_FUZZ, "split": "right"},
            {"name": "tidepool-readme-docs", "kind": "pi", "status": "idle", "title": "README usage",
             "start": 100, "end": 96, "turns": TIDEPOOL_DOCS, "split": "down"},
        ]},
        {"label": "release", "panes": [
            {"name": "tidepool-release-dev", "kind": "claude", "status": "idle", "title": "0.6.0 release notes",
             "start": 300, "end": 290, "turns": TIDEPOOL_RELEASE},
        ]},
    ]},
    {"label": "Quarry", "dir": "quarry", "about": "Search service for produce listings.", "tabs": [
        {"label": "ranking", "panes": [
            {"name": "quarry-rank-dev", "kind": "claude", "status": "working", "title": "Freshness in ranking",
             "start": 65, "end": 0.4, "turns": QUARRY_RANK, "verb": "Replaying queries", "elapsed": "3m 08s"},
        ]},
        {"label": "bench", "panes": [
            {"name": "quarry-bench-perf", "kind": "codex", "status": "working", "title": "Index build benchmark",
             "start": 20, "end": 0.6, "turns": QUARRY_BENCH, "elapsed": "2m 12s"},
        ]},
        {"label": "notes", "panes": [
            {"name": "quarry-notes-docs", "kind": "claude", "status": "done", "title": "Weekly update",
             "start": 30, "end": 4, "turns": QUARRY_NOTES},
        ]},
    ]},
    {"label": "Meadow", "dir": "meadow", "about": "iOS app for co-op members: harvest totals, pickups and news.", "tabs": [
        {"label": "onboarding", "panes": [
            {"name": "meadow-onboard-dev", "kind": "claude", "status": "question", "title": "Onboarding rework",
             "start": 12, "end": 3, "turns": MEADOW_ONBOARD, "question": MEADOW_ONBOARD_Q, "answers": MEADOW_ONBOARD_A},
        ]},
        {"label": "harvest card", "panes": [
            {"name": "meadow-card-dev", "kind": "claude", "status": "done", "title": "Harvest card on small phones",
             "start": 24, "end": 5, "turns": MEADOW_CARD},
        ]},
        {"label": "widgets", "panes": [
            {"name": "meadow-widget-dev", "kind": "claude", "status": "working", "title": "Harvest widget",
             "start": 28, "end": 0.4, "turns": MEADOW_WIDGETS, "elapsed": "58s",
             "queued": ["Also add a medium size that shows the week chart.", "Use the app's green for today's bar."]},
        ]},
        {"label": "crash", "panes": [
            {"name": "meadow-crash-dev", "kind": "claude", "status": "done", "title": "Launch crash after upgrade",
             "start": 75, "end": 8, "turns": MEADOW_CRASH},
        ]},
        {"label": "l10n", "panes": [
            {"name": "meadow-l10n-docs", "kind": "pi", "status": "idle", "title": "Unlocalized strings",
             "start": 140, "end": 136, "turns": MEADOW_L10N},
        ]},
    ]},
    {"label": "Beacon", "dir": "beacon", "about": "Notification service in Go: push, email and digests.", "tabs": [
        {"label": "retries", "panes": [
            {"name": "beacon-retry-dev", "kind": "claude", "status": "approval", "title": "Retry 429s with backoff",
             "start": 22, "end": 2, "turns": BEACON_RETRY, "approval_why": "Start a local Redis for the integration test",
             "allow": BEACON_RETRY_ALLOW},
        ]},
        {"label": "digest", "panes": [
            {"name": "beacon-digest-dev", "kind": "codex", "status": "done", "title": "Daily email digest",
             "start": 48, "end": 15, "turns": BEACON_DIGEST},
        ]},
        {"label": "load test", "panes": [
            {"name": "beacon-load-perf", "kind": "pi", "status": "working", "title": "Send path load test",
             "start": 6, "end": 0.3, "turns": BEACON_LOAD},
        ]},
    ]},
    {"label": "Ledger", "dir": "ledger", "about": "Billing: invoices, payouts and reconciliation.", "tabs": [
        {"label": "invoices", "panes": [
            {"name": "ledger-invoice-dev", "kind": "claude", "status": "question", "title": "Multi-currency invoice totals",
             "start": 18, "end": 2, "turns": LEDGER_INVOICE, "question": LEDGER_INVOICE_Q, "answers": LEDGER_INVOICE_A},
        ]},
        {"label": "reconcile", "panes": [
            {"name": "ledger-recon-ops", "kind": "codex", "status": "idle", "title": "September reconciliation",
             "start": 210, "end": 200, "turns": LEDGER_RECON},
        ]},
    ]},
]

# Resources for the other main agents (each gets a few, on its own project).
resdemo.extend(WORKSPACES)
