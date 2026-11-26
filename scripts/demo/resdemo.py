"""Demo content for the Resources window (herdr-app/mods/flockbook/resources.js): one Lighthouse agent whose
conversation points to many resources of every kind, and the files it names.

Imported by content.py (the agent's turns) and hc_seed.py (write_files). All names, files and data are made up.
The doc files really exist under the made-up HOME (HOME/code/lighthouse): Resources lists only docs whose file is
there, and opens them. The reply mixes the forms the helper reads (xplat/flockbook-modfeed/src/resources.rs):
absolute paths, ~/ paths, relative paths (they open under the log's cwd), Markdown links, code spans, a table,
fenced one-line shell commands, GitHub PR/issue/run links, claude.ai artifact links (also from Artifact publish calls),
local servers and other links. Tool calls (Read, Write, Edit, Bash, Glob) name the same files, as a real session does.
"""
import os
import struct
import uuid
import zlib

HOME = os.path.expanduser("~")
CWD = f"{HOME}/code/lighthouse"
TILDE = "~/code/lighthouse"
REPO = "github.com/fbdemo-coop/lighthouse"

# ------------------------------------------------------------------ the files (path under CWD, title)

MARKDOWN = [
    ("docs/getting-started.md", "Getting started"),
    ("docs/install.md", "Install the Lighthouse CLI"),
    ("docs/configuration.md", "Configuration"),
    ("docs/cli-reference.md", "CLI reference"),
    ("docs/api/orders.md", "Orders API"),
    ("docs/api/listings.md", "Listings API"),
    ("docs/api/pickups.md", "Pickups API"),
    ("docs/api/webhooks.md", "Webhooks"),
    ("docs/api/auth.md", "Authentication"),
    ("docs/api/errors.md", "Error codes"),
    ("docs/guides/first-deploy.md", "Your first deploy"),
    ("docs/guides/rollback.md", "Roll back a release"),
    ("docs/guides/backups.md", "Backups and restore"),
    ("docs/guides/alerts.md", "Alerts that matter"),
    ("docs/guides/rate-limits.md", "Rate limits"),
    ("docs/guides/migrations.md", "Database migrations"),
    ("docs/faq.md", "FAQ"),
    ("docs/glossary.md", "Glossary"),
    ("docs/changelog.md", "Changelog"),
    ("docs/contributing.md", "Contributing to the docs"),
    ("plans/launch-2.0-plan.md", "Docs 2.0 launch plan"),
    ("plans/nav-redesign.md", "Navigation redesign"),
    ("plans/search-index.md", "Search index rebuild"),
    ("plans/versioned-docs.md", "Versioned docs"),
    ("plans/i18n-plan.md", "Translations plan"),
    ("specs/style-guide.md", "Style guide"),
    ("specs/page-template.md", "Page template"),
    ("specs/api-page-spec.md", "API page spec"),
    ("specs/link-checker-spec.md", "Link checker spec"),
    ("specs/redirects-spec.md", "Redirects spec"),
    ("reports/audit-2026-11.md", "Docs audit, November 2026"),
    ("reports/broken-links.md", "Broken links"),
    ("reports/readability.md", "Readability scores"),
    ("reports/page-weight.md", "Page weight"),
    ("reports/search-misses.md", "Searches with no result"),
    ("reports/survey-results.md", "Reader survey results"),
    ("reports/accessibility.md", "Accessibility check"),
    ("notes/sync-2026-11-20.md", "Docs sync, 20 November"),
]
HTML = [
    ("site/_preview/index.html", "Lighthouse Docs"),
    ("site/_preview/getting-started.html", "Getting started · Lighthouse Docs"),
    ("site/_preview/api/orders.html", "Orders API · Lighthouse Docs"),
    ("site/_preview/guides/rollback.html", "Roll back a release · Lighthouse Docs"),
    ("reports/audit-dashboard.html", "Docs audit dashboard"),
    ("reports/link-report.html", "Link check report"),
    ("prototypes/nav-v2.html", "Navigation v2 prototype"),
    ("prototypes/search.htm", "Search prototype"),
]
PDF = [
    ("exports/lighthouse-docs-2.0.pdf", "Lighthouse Docs 2.0"),
    ("exports/api-reference.pdf", "API reference"),
    ("exports/style-guide.pdf", "Style guide"),
    ("reports/survey-results.pdf", "Reader survey results"),
    ("reports/audit-summary.pdf", "Docs audit summary"),
    ("handouts/onboarding-checklist.pdf", "Onboarding checklist"),
]
PNG = [
    ("screenshots/home-before.png", (196, 205, 214)),
    ("screenshots/home-after.png", (64, 132, 108)),
    ("screenshots/nav-mobile.png", (92, 116, 170)),
    ("screenshots/search-results.png", (214, 160, 72)),
    ("screenshots/api-page.png", (120, 96, 160)),
    ("screenshots/dark-mode.png", (40, 44, 52)),
]
SVG = [
    ("diagrams/site-map.svg", "Site map"),
    ("diagrams/deploy-flow.svg", "Deploy flow"),
    ("diagrams/docs-pipeline.svg", "Docs build pipeline"),
    ("diagrams/nav-tree.svg", "Navigation tree"),
]
# Mentioned only in the replies to a typed message (the badge counts them).
LATER = [
    ("reports/landscape-check.md", "Landscape check"),
    ("screenshots/landscape.png", (88, 150, 128)),
]

ARTIFACTS = {
    "dashboard": "https://claude.ai/artifact/5d0c7a2e-41b8-4c6f-9e3a-8b17f2a6c901",
    "nav": "https://claude.ai/artifact/9b3e1f40-7c2d-4a85-b6e1-2f4d8c0a7e35",
    "survey": "https://claude.ai/artifact/1e7a9c55-0b3f-4d21-a8c6-6f92d4e1b078",
    "plan": "https://claude.ai/artifact/c4f28b91-6a0e-4f37-9d15-3b7e2a8f0c46",
    "search": "https://claude.ai/artifact/7a61d0e3-28bc-4e94-b0f7-91c5e3d2a6f8",
}


def p(rel):
    return f"{CWD}/{rel}"


def t(rel):
    return f"{TILDE}/{rel}"


# ------------------------------------------------------------------ writing them

def _md(title, rel, project="Lighthouse"):
    return (f"# {title}\n\n"
            f"Part of the {project} docs (fictional, for the Flockbook demo). File: `{rel}`.\n\n"
            "## Summary\n\n"
            "- What this page covers, in one line.\n"
            "- Who reads it: co-op developers and operators.\n"
            "- Last reviewed: November 2026.\n")


def _html(title, project="Lighthouse"):
    return ("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n"
            f"<title>{title}</title>\n"
            "<style>body{font:16px/1.5 system-ui,sans-serif;max-width:40rem;margin:3rem auto;padding:0 1rem;color:#1d2a24}"
            "h1{color:#2f6b55}</style>\n</head>\n<body>\n"
            f"<h1>{title}</h1>\n<p>A page of the fictional {project} docs, for the Flockbook demo.</p>\n"
            "</body>\n</html>\n")


def _pdf(title, project="Lighthouse"):
    """A real one-page PDF (Helvetica text), with a correct xref table."""
    esc = title.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
    stream = (f"BT /F1 24 Tf 72 720 Td ({esc}) Tj ET\n"
              f"BT /F1 12 Tf 72 690 Td (Fictional {project} docs, for the Flockbook demo.) Tj ET\n").encode()
    objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"endstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Title (" + esc.encode() + b") /Producer (flockbook demo) >>",
    ]
    out = bytearray(b"%PDF-1.4\n")
    offs = []
    for i, o in enumerate(objs, 1):
        offs.append(len(out))
        out += f"{i} 0 obj\n".encode() + o + b"\nendobj\n"
    x = len(out)
    out += f"xref\n0 {len(objs) + 1}\n0000000000 65535 f \n".encode()
    out += b"".join(f"{o:010d} 00000 n \n".encode() for o in offs)
    out += f"trailer\n<< /Size {len(objs) + 1} /Root 1 0 R /Info 6 0 R >>\nstartxref\n{x}\n%%EOF\n".encode()
    return bytes(out)


def _png(color, w=480, h=300):
    """A real PNG: a page mock-up (a bar, a sidebar, text lines) in the given color."""
    r, g, b = color
    rows = []
    for y in range(h):
        row = bytearray([0])
        for x in range(w):
            if y < 36:
                px = (r, g, b)
            elif x < 110:
                px = (min(r + 40, 255), min(g + 40, 255), min(b + 40, 255))
            elif 130 <= x < 130 + 300 - (y // 22 % 4) * 50 and y % 22 in range(10, 16):
                px = (90, 98, 106)
            else:
                px = (250, 251, 250)
            row += bytes(px)
        rows.append(bytes(row))
    raw = zlib.compress(b"".join(rows), 9)

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + \
        chunk(b"IDAT", raw) + chunk(b"IEND", b"")


def _svg(title):
    boxes = "".join(f'<rect x="{20 + i * 150}" y="60" width="120" height="50" rx="8" fill="#e6f0eb" stroke="#2f6b55"/>'
                    f'<text x="{80 + i * 150}" y="90" text-anchor="middle" font-size="13">{name}</text>'
                    + (f'<path d="M{140 + i * 150} 85 h30" stroke="#2f6b55" marker-end="url(#a)"/>' if i < 3 else "")
                    for i, name in enumerate(["Source", "Build", "Check", "Publish"]))
    return ('<svg xmlns="http://www.w3.org/2000/svg" width="620" height="160" viewBox="0 0 620 160" '
            'font-family="system-ui, sans-serif">\n'
            f"<title>{title}</title>\n"
            '<defs><marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto">'
            '<path d="M0 0 L10 5 L0 10 z" fill="#2f6b55"/></marker></defs>\n'
            f'<rect width="620" height="160" fill="#fbfcfb"/><text x="20" y="34" font-size="16" font-weight="600">{title}</text>\n'
            f"{boxes}\n</svg>\n")


def write_files(cwd=CWD):
    """Writes every file the agent names (small but real), under cwd. Returns how many."""
    files = []
    for rel, title in MARKDOWN + [LATER[0]]:
        files.append((rel, _md(title, rel).encode()))
    files += [(rel, _html(title).encode()) for rel, title in HTML]
    files += [(rel, _pdf(title)) for rel, title in PDF]
    files += [(rel, _png(color)) for rel, color in PNG + [LATER[1]]]
    files += [(rel, _svg(title).encode()) for rel, title in SVG]
    files = [(f"{cwd}/{rel}", data) for rel, data in files] + extra_files()
    for path, data in files:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)
    return len(files)


# ------------------------------------------------------------------ the conversation

def _sh(lead, cmd):
    return f"{lead}\n\n```sh\n{cmd}\n```"


md = dict(MARKDOWN)

TURNS = [
    ("u", "Before the 2.0 launch, audit the docs site: broken links, stale pages, missing screenshots. "
          "Write up what you find and fix the small things."),
    ("a", "I'll start with the launch plan and the style guide, then walk every page."),
    ("read", "plans/launch-2.0-plan.md", _md(md["plans/launch-2.0-plan.md"], "plans/launch-2.0-plan.md")),
    ("read", "specs/style-guide.md", _md(md["specs/style-guide.md"], "specs/style-guide.md")),
    ("glob", "docs/**/*.md", "\n".join(p(r) for r, _ in MARKDOWN if r.startswith("docs/"))),
    ("a", "The plan is in [Docs 2.0 launch plan](" + t("plans/launch-2.0-plan.md") + ") and the rules in "
          "`" + t("specs/style-guide.md") + "`. There are 20 pages under `docs/`; these need the most work:\n\n"
          "- `docs/getting-started.md`: still shows the 1.x install steps\n"
          "- `docs/install.md`: no Linux section\n"
          "- `docs/configuration.md` and `docs/cli-reference.md`: flags renamed in 1.9\n"
          "- " + t("docs/api/orders.md") + " and " + t("docs/api/listings.md") + ": examples return the old fields\n"
          "- " + t("docs/api/pickups.md") + ", " + t("docs/api/webhooks.md") + ", " + t("docs/api/auth.md")
          + " and " + t("docs/api/errors.md") + ": fine, but no page template header"),
    ("bash", "cd " + TILDE + "/site && npm run build", "built 64 pages in 3.8s\nwarnings: 23 broken links"),
    ("a", _sh("The build is green but warns about links. To see the site as it is now:", "npm run preview -- --port 4321")
          + "\n\nIt serves on http://localhost:4321/ (the API pages on http://localhost:4321/api/orders)."),
    ("bash", "npx lychee --offline 'site/dist/**/*.html' --format markdown > reports/broken-links.md",
     "23 broken links in 11 pages"),
    ("write", "reports/broken-links.md", _md(md["reports/broken-links.md"], "reports/broken-links.md")),
    ("a", "23 broken links, listed in " + p("reports/broken-links.md") + ". Most point to the old guides:\n\n"
          "| Old page | New page |\n|---|---|\n"
          "| first-deploy | `docs/guides/first-deploy.md` |\n"
          "| rollback | " + t("docs/guides/rollback.md") + " |\n"
          "| backups | " + t("docs/guides/backups.md") + " |\n"
          "| alerts | " + t("docs/guides/alerts.md") + " |\n"
          "| rate limits | " + t("docs/guides/rate-limits.md") + " |\n"
          "| migrations | " + t("docs/guides/migrations.md") + " |\n\n"
          "The redirects belong in `specs/redirects-spec.md`; the link checker rules are in `specs/link-checker-spec.md`."),
    ("edit", "docs/faq.md", "## How do I install 1.x?", "## How do I install Lighthouse?"),
    ("edit", "docs/glossary.md", "**pickup slot**: a time", "**Pickup slot**: a 30-minute window"),
    ("a", "Fixed the FAQ heading in [FAQ](" + t("docs/faq.md") + ") and a definition in `docs/glossary.md`. "
          "The changelog (`docs/changelog.md`) and `docs/contributing.md` are current.\n\n"
          "Issues I opened for the bigger ones:\n\n"
          "- https://" + REPO + "/issues/212 (install steps for Linux)\n"
          "- https://" + REPO + "/issues/213 (API examples return old fields)\n"
          "- " + REPO + "/issues/215 (search finds nothing for \"pickup\")"),
    ("bash", "cd " + TILDE + " && npx playwright screenshot --full-page http://localhost:4321/ screenshots/home-after.png",
     "saved screenshots/home-after.png"),
    ("a", "Screenshots for the audit, before and after the new home page:\n\n"
          "- " + t("screenshots/home-before.png") + "\n- " + t("screenshots/home-after.png") + "\n"
          "- " + t("screenshots/nav-mobile.png") + " (the menu on a 375 px phone)\n"
          "- " + t("screenshots/search-results.png") + "\n- " + t("screenshots/api-page.png") + "\n"
          "- " + t("screenshots/dark-mode.png") + " (code blocks are unreadable in dark mode)"),
    ("write", "diagrams/site-map.svg", _svg("Site map")),
    ("a", "I drew the structure as diagrams: `diagrams/site-map.svg`, `diagrams/nav-tree.svg`, "
          "`diagrams/docs-pipeline.svg` and `diagrams/deploy-flow.svg`. The nav tree shows why readers get lost: "
          "the guides sit three levels deep."),
    ("read", "plans/nav-redesign.md", _md(md["plans/nav-redesign.md"], "plans/nav-redesign.md")),
    ("a", "The fix is already planned: " + t("plans/nav-redesign.md") + ", with " + t("plans/search-index.md")
          + " for search. " + t("plans/versioned-docs.md") + " and " + t("plans/i18n-plan.md")
          + " wait until after 2.0.\n\nI made two prototypes to compare: " + t("prototypes/nav-v2.html")
          + " and " + t("prototypes/search.htm") + "."),
    ("tool", "Artifact", {"action": "publish", "file_path": p("prototypes/nav-v2.html"), "title": "Navigation v2 prototype"},
     "Published. Link: " + ARTIFACTS["nav"]),
    ("tool", "Artifact", {"action": "publish", "file_path": p("reports/audit-dashboard.html"), "title": "Docs audit dashboard"},
     "Published. Link: " + ARTIFACTS["dashboard"]),
    ("a", "Published both so the team can try them without a checkout: the nav prototype and the "
          "[audit dashboard](" + ARTIFACTS["dashboard"] + "). The search prototype is at " + ARTIFACTS["search"]
          + ", from last week.\n\nLocal copies: " + t("reports/audit-dashboard.html") + " and " + t("reports/link-report.html") + "."),
    ("bash", "cd " + TILDE + "/site && npm run build -- --out _preview",
     "built 64 pages into site/_preview in 3.6s"),
    ("a", "Preview build for the pages I changed:\n\n"
          "- " + t("site/_preview/index.html") + "\n- " + t("site/_preview/getting-started.html") + "\n"
          "- " + t("site/_preview/api/orders.html") + "\n- " + t("site/_preview/guides/rollback.html") + "\n\n"
          + _sh("To serve that folder instead:", "python3 -m http.server 8787 --directory site/_preview")
          + "\n\nThen open http://127.0.0.1:8787/."),
    ("a", "Readers' side, from the survey and the logs:\n\n"
          "- `reports/survey-results.md` (and the board copy, " + t("reports/survey-results.pdf") + ")\n"
          "- `reports/search-misses.md`: 41 searches with no result last month\n"
          "- `reports/readability.md` and `reports/page-weight.md`: the API pages are the heaviest (1.8 MB)\n"
          "- `reports/accessibility.md`: 6 images without alt text\n\n"
          "Reference I used: https://www.w3.org/WAI/WCAG22/quickref/ and https://developer.mozilla.org/en-US/docs/Web/HTML/Element/img."),
    ("tool", "Artifact", {"action": "publish", "file_path": p("reports/survey-results.md"), "title": "Reader survey, at a glance"},
     "Published. Link: " + ARTIFACTS["survey"]),
    ("bash", "gh pr create --title 'docs: fix 23 broken links and the FAQ' --body-file reports/broken-links.md",
     "https://" + REPO + "/pull/148"),
    ("a", "Opened https://" + REPO + "/pull/148 with the link fixes. Related work in flight:\n\n"
          "- https://" + REPO + "/pull/141 (page template header, from `specs/page-template.md`)\n"
          "- https://" + REPO + "/pull/144 (API page layout, per `specs/api-page-spec.md`)\n"
          "- https://" + REPO + "/pull/146/files (dark mode code blocks)\n"
          "- " + REPO + "/pull/147 (redirects)\n\n"
          "CI on #148: https://" + REPO + "/actions/runs/18752231 passed; the link check job "
          "https://" + REPO + "/actions/runs/18752236 failed on one external link, and the nightly "
          "https://" + REPO + "/actions/runs/18749910 is still red from Tuesday.\n\n"
          + _sh("To try the branch locally:", "gh pr checkout 148")),
    ("bash", "gh run rerun 18752236 --failed", "✓ Requested rerun of the failed jobs"),
    ("a", "The external link was a timeout, so I asked for a rerun of the failed job.\n\n"
          + _sh("To watch it:", "gh run watch 18752236")
          + "\n\n" + _sh("If it fails again, run the checker on that page alone:",
                         "npx lychee --verbose site/dist/guides/alerts/index.html")),
    ("bash", "cd " + TILDE + " && make pdf", "wrote exports/lighthouse-docs-2.0.pdf (212 pages)"),
    ("a", "The print versions are rebuilt:\n\n"
          "1. " + t("exports/lighthouse-docs-2.0.pdf") + " (the whole site, 212 pages)\n"
          "2. " + t("exports/api-reference.pdf") + "\n3. " + t("exports/style-guide.pdf") + "\n"
          "4. " + t("handouts/onboarding-checklist.pdf") + " for new members\n\n"
          + _sh("To rebuild only the API reference:", "make pdf PAGES=docs/api")),
    ("write", "reports/audit-2026-11.md", _md(md["reports/audit-2026-11.md"], "reports/audit-2026-11.md")),
    ("bash", "cd " + TILDE + " && make pdf-report REPORT=reports/audit-2026-11.md", "wrote reports/audit-summary.pdf"),
    ("tool", "Artifact", {"action": "publish", "file_path": p("plans/launch-2.0-plan.md"), "title": "Docs 2.0: what is left"},
     "Published. Link: " + ARTIFACTS["plan"]),
    ("a", "## Audit done\n\n"
          "The full write-up is [Docs audit, November 2026](" + p("reports/audit-2026-11.md") + "), with a one-page "
          + p("reports/audit-summary.pdf") + " for the sync. Notes from the last sync are in "
          + t("notes/sync-2026-11-20.md") + ". What is left for 2.0 is on " + ARTIFACTS["plan"] + ".\n\n"
          "| Area | Found | Fixed |\n|---|---|---|\n"
          "| Broken links | 23 | 23 (#148) |\n| Stale pages | 9 | 2 |\n| Missing alt text | 6 | 0 |\n"
          "| Heavy pages | 4 | 0 |\n\n"
          "Open issues: https://" + REPO + "/issues/212, https://" + REPO + "/issues/213, https://" + REPO + "/issues/215. "
          "The style rules for the rest are in https://commonmark.org/help/ and our own style guide.\n\n"
          + _sh("To check everything again before the launch:", "make check-docs")
          + "\n\n" + _sh("And to publish the preview to staging (needs the deploy key):",
                         "./scripts/publish.sh --env staging --dry-run")
          + "\n\nThe live docs are https://docs.lighthouse.example/; the status page for the launch day is "
          "https://status.lighthouse.example/, and the staging site "
          "https://staging.docs.lighthouse.example/getting-started. The issue tracker board is "
          "https://" + REPO + "/projects/3."),
    ("a", _sh("One more thing: the search index is stale. To rebuild it:", "npm run search:index -- --full")
          + "\n\n" + _sh("And to compare the page weights after the image fixes:", "npx page-weight site/dist --budget 500kb")
          + "\n\nThe budget rule comes from https://web.dev/articles/performance-budgets-101."),
]

REPLIES = [
    "Done. I checked the pages in landscape too: two tables overflow on a phone. Notes in "
    + t(LATER[0][0]) + ", screenshot " + t(LATER[1][0]) + ".\n\n"
    + _sh("To see it yourself:", "npm run preview -- --port 4321 --host"),
    "Sure. I added the missing alt text to the six images (see `reports/accessibility.md`); "
    "it is on https://" + REPO + "/pull/149.",
    "Good idea. The redirects are in now; `specs/redirects-spec.md` lists all 23.",
    "On it. That's in now, and the build is green.",
]


# ------------------------------------------------------------------ the other main agents

# A few resources for each agent the recording and the shots open (owner 11-26), on that agent's own made-up project:
# docs (md, html, pdf, png, svg) under HOME/code/<space dir>, an Artifact publish, GitHub links on fbdemo-coop, other
# links and a command. extend() puts them right after the first user message, so the end of each conversation (a
# question, an approval) stays as it is. Per agent: docs [(path, title)], art (an Artifact title or None),
# gh [path on the repo], links [url], cmd (lead line, command).
EXTRA = {
    "orchard-api-dev": {"docs": [("docs/pricing/rounding-rules.md", "Rounding rules for order totals"),
                                 ("docs/pricing/tax-table.md", "Tax rates by county"),
                                 ("reports/cart-diff-2026-11.html", "Cart totals before and after"),
                                 ("exports/receipt-sample.pdf", "Sample receipt"),
                                 ("screenshots/cart-total-off-by-cent.png", "Cart total off by a cent")],
                        "art": "Rounding before and after", "gh": ["pull/212", "issues/208", "actions/runs/1187"],
                        "links": ["https://docs.python.org/3/library/decimal.html", "http://localhost:8000/cart"],
                        "cmd": ("To check the totals against the fixtures:", "pytest tests/test_pricing.py -q")},
    "orchard-promo-dev": {"docs": [("docs/checkout/promo-codes-spec.md", "Promo codes spec"),
                                   ("docs/checkout/discount-examples.md", "Discount examples"),
                                   ("diagrams/promo-stacking.svg", "How two codes stack"),
                                   ("exports/promo-terms.pdf", "Promo terms for the shop"),
                                   ("prototypes/promo-field.html", "Promo code field prototype")],
                          "art": "Promo stacking options", "gh": ["issues/215", "pull/219"],
                          "links": ["http://localhost:8000/checkout"],
                          "cmd": ("The codes in the test shop:", "python -m orchard.promo --list")},
    "atlas-import-dev": {"docs": [("docs/import/csv-format.md", "Harvest CSV format"),
                                  ("docs/import/malformed-rows.md", "Malformed rows we have seen"),
                                  ("reports/import-2026-11-24.html", "Import report, 24 November"),
                                  ("exports/import-errors.pdf", "Import errors by farm"),
                                  ("screenshots/import-errors.png", "Import errors in the admin")],
                         "art": "Import error summary", "gh": ["pull/88", "issues/84", "actions/runs/2291"],
                         "links": ["https://www.rfc-editor.org/rfc/rfc4180", "http://localhost:8080/imports"],
                         "cmd": ("A dry run on the November file:", "atlas import samples/harvest-2026-11.csv --dry-run")},
    "atlas-schema-dev": {"docs": [("docs/schema/variety-column.md", "Variety column"),
                                  ("docs/schema/migrations.md", "How we run migrations"),
                                  ("diagrams/harvest-schema.svg", "Harvest tables"),
                                  ("reports/index-lock-test.html", "Index lock test")],
                         "art": None, "gh": ["pull/91", "issues/90"],
                         "links": ["https://www.postgresql.org/docs/current/sql-altertable.html"],
                         "cmd": ("To see the SQL without running it:", "make migrate-dry-run")},
    "harbor-deploy-ops": {"docs": [("docs/deploy/staging.md", "Staging deploys"),
                                   ("docs/deploy/checklist.md", "Deploy checklist"),
                                   ("reports/dry-run-2026-11-25.html", "Dry run, 25 November"),
                                   ("exports/change-window.pdf", "Change window for staging"),
                                   ("diagrams/deploy-flow.svg", "Deploy flow")],
                          "art": "Staging deploy plan", "gh": ["pull/57", "actions/runs/4410", "actions/runs/4412"],
                          "links": ["http://localhost:9090/targets", "https://kubernetes.io/docs/concepts/workloads/controllers/deployment/"],
                          "cmd": ("The dry run on its own:", "sh scripts/deploy.sh staging --dry-run")},
    "harbor-runbook-docs": {"docs": [("docs/runbooks/incident-roles.md", "Incident roles"),
                                     ("docs/runbooks/rollback.md", "Rollback runbook"),
                                     ("docs/runbooks/db-restore.md", "Restore the database"),
                                     ("diagrams/rollback-steps.svg", "Rollback steps"),
                                     ("exports/rollback-runbook.pdf", "Rollback runbook (print)"),
                                     ("screenshots/error-rate.png", "Error rate during a rollback")],
                            "art": "Rollback runbook", "gh": ["issues/61"],
                            "links": ["http://localhost:3000/d/deploys"],
                            "cmd": ("To practise it on staging:", "harbor rollback --to previous --dry-run")},
    "lighthouse-start-docs": {"docs": [("docs/getting-started-draft.md", "Getting Started (draft)"),
                                       ("docs/local-setup.md", "Run the docs site locally"),
                                       ("screenshots/first-run.png", "The site on first run"),
                                       ("site/_preview/getting-started.html", "Getting Started preview")],
                              "art": None, "gh": ["pull/151", "issues/146"],
                              "links": ["http://localhost:4321/getting-started"],
                              "cmd": ("To see the draft:", "npm run dev")},
    "tidepool-parse-dev": {"docs": [("docs/since-flag.md", "The --since flag"),
                                    ("docs/time-formats.md", "Time formats we accept"),
                                    ("benches/parse-before-after.svg", "Parse time before and after"),
                                    ("reports/parse-bench.html", "Parser benchmark")],
                           "art": None, "gh": ["pull/34", "issues/31", "actions/runs/905"],
                           "links": ["https://docs.rs/chrono/latest/chrono/", "https://doc.rust-lang.org/book/"],
                           "cmd": ("Only the new tests:", "cargo test -p tidepool-parse since")},
    "tidepool-release-dev": {"docs": [("docs/release-0.6.0.md", "Release 0.6.0"),
                                      ("docs/upgrade-0.5-to-0.6.md", "Upgrade from 0.5"),
                                      ("screenshots/tail-color.png", "Colored output in 0.6"),
                                      ("exports/release-notes-0.6.0.pdf", "Release notes 0.6.0")],
                             "art": "0.6.0 release notes", "gh": ["pull/40", "actions/runs/951"],
                             "links": ["https://github.com/fbdemo-coop/tidepool/releases/tag/v0.6.0"],
                             "cmd": ("What the release would do:", "cargo release 0.6.0 --dry-run")},
    "quarry-rank-dev": {"docs": [("docs/ranking/freshness.md", "Freshness in ranking"),
                                 ("docs/ranking/bm25-notes.md", "BM25 notes"),
                                 ("reports/ndcg-freshness.html", "NDCG with freshness"),
                                 ("diagrams/decay-curve.svg", "Freshness decay curve"),
                                 ("exports/ranking-review.pdf", "Ranking review")],
                        "art": "Freshness decay curves", "gh": ["pull/73", "issues/70"],
                        "links": ["https://en.wikipedia.org/wiki/Okapi_BM25", "http://localhost:7700/search?q=apples"],
                        "cmd": ("The offline evaluation:", "quarry eval --set fresh-2026-11")},
    "quarry-notes-docs": {"docs": [("notes/weekly-2026-11-24.md", "Weekly update, 24 November"),
                                   ("notes/weekly-2026-11-17.md", "Weekly update, 17 November"),
                                   ("reports/latency-week-47.html", "Search latency, week 47"),
                                   ("screenshots/p95-latency.png", "p95 latency"),
                                   ("exports/weekly-2026-11-24.pdf", "Weekly update (PDF)")],
                          "art": "Weekly update 47", "gh": ["pull/76", "actions/runs/3302"],
                          "links": ["http://localhost:7700/stats"],
                          "cmd": ("The numbers come from:", "quarry report --week 47")},
    "meadow-card-dev": {"docs": [("docs/design/harvest-card.md", "Harvest card design"),
                                 ("screenshots/card-small.png", "Card on a small phone"),
                                 ("screenshots/card-large.png", "Card on a large phone"),
                                 ("diagrams/card-layout.svg", "Card layout"),
                                 ("prototypes/card-sizes.html", "Card sizes prototype")],
                        "art": "Harvest card sizes", "gh": ["pull/128", "issues/125", "actions/runs/7710"],
                        "links": ["https://www.w3.org/WAI/WCAG22/Understanding/reflow.html"],
                        "cmd": ("The UI tests on the small simulator:", "make ui-test DEVICE=small")},
    "meadow-onboard-dev": {"docs": [("docs/onboarding/flow.md", "Onboarding flow"),
                                    ("docs/onboarding/copy.md", "Onboarding copy"),
                                    ("screenshots/onboard-step1.png", "Onboarding, step 1"),
                                    ("diagrams/onboarding-flow.svg", "Sign-in choices"),
                                    ("exports/sign-in-options.pdf", "Sign-in options")],
                           "art": "Onboarding flow", "gh": ["issues/131", "pull/133"],
                           "links": ["http://localhost:8090/auth/health"],
                           "cmd": ("The sign-in tests:", "make test SUITE=onboarding")},
    "beacon-retry-dev": {"docs": [("docs/push/retries.md", "Push retries"),
                                  ("docs/push/backoff.md", "Backoff and Retry-After"),
                                  ("diagrams/retry-backoff.svg", "Retry with backoff"),
                                  ("reports/429-rates.html", "429 rates by provider")],
                         "art": None, "gh": ["pull/64", "issues/60", "actions/runs/1530"],
                         "links": ["https://www.rfc-editor.org/rfc/rfc6585#section-4", "http://localhost:8025/"],
                         "cmd": ("Only the retry tests:", "go test ./internal/push/... -run Retry")},
    "ledger-invoice-dev": {"docs": [("docs/invoices/multi-currency.md", "Multi-currency invoices"),
                                    ("docs/invoices/rounding.md", "Rounding on invoices"),
                                    ("exports/invoice-sample-eur.pdf", "Sample invoice (EUR)"),
                                    ("exports/invoice-sample-jpy.pdf", "Sample invoice (JPY)"),
                                    ("screenshots/invoice-totals.png", "Invoice totals"),
                                    ("reports/fx-rates-2026-11.html", "Exchange rates, November")],
                           "art": "Invoice rounding options", "gh": ["pull/45", "issues/42"],
                           "links": ["https://en.wikipedia.org/wiki/ISO_4217"],
                           "cmd": ("A sample invoice:", "ledger invoice render --sample eur")},
}

SPACE = {"orchard": "Orchard", "atlas": "Atlas", "harbor": "Harbor", "lighthouse": "Lighthouse", "tidepool": "Tidepool",
         "quarry": "Quarry", "meadow": "Meadow", "beacon": "Beacon", "ledger": "Ledger"}
_COLORS = [(47, 107, 85), (58, 96, 140), (140, 88, 52), (110, 70, 130), (60, 120, 120)]


def _space(name):
    return name.split("-")[0]


def extra_files():
    """[(absolute path, bytes)] of every EXTRA doc, under HOME/code/<space dir>."""
    out = []
    for name, spec in EXTRA.items():
        d = _space(name)
        for i, (rel, title) in enumerate(spec["docs"]):
            ext = rel.rsplit(".", 1)[1]
            data = {"md": lambda: _md(title, rel, SPACE[d]).encode(), "html": lambda: _html(title, SPACE[d]).encode(),
                    "pdf": lambda: _pdf(title, SPACE[d]), "png": lambda: _png(_COLORS[i % len(_COLORS)]),
                    "svg": lambda: _svg(title).encode()}[ext]()
            out.append((f"{HOME}/code/{d}/{rel}", data))
    return out


def extra_turns(name):
    spec = EXTRA[name]
    d = _space(name)
    tilde, repo = f"~/code/{d}", f"https://github.com/fbdemo-coop/{d}"
    docs = spec["docs"]
    turns = [("read", docs[0][0], _md(docs[0][1], docs[0][0], SPACE[d])),
             ("a", "Background first. What is already written:\n\n"
                   + "\n".join(f"- [{title}]({tilde}/{rel})" for rel, title in docs))]
    if spec["art"]:
        page = next((rel for rel, _ in docs if rel.endswith(".html")), docs[0][0])
        url = "https://claude.ai/artifact/" + str(uuid.uuid5(uuid.NAMESPACE_URL, f"fbdemo-{name}"))
        turns.append(("tool", "Artifact", {"action": "publish", "file_path": f"{HOME}/code/{d}/{page}", "title": spec["art"]},
                      f"Published. Link: {url}"))
    lead, cmd = spec["cmd"]
    turns.append(("a", "Related work:\n\n" + "\n".join(f"- {repo}/{g}" for g in spec["gh"])
                  + "\n" + "\n".join(f"- {u}" for u in spec["links"]) + "\n\n" + _sh(lead, cmd)))
    return turns


def extend(workspaces):
    """Adds the EXTRA resources to those agents, right after their first (user) turn."""
    seen = set()
    for ws in workspaces:
        for tab in ws["tabs"]:
            for pane in tab["panes"]:
                if pane["name"] in EXTRA:
                    pane["turns"] = pane["turns"][:1] + extra_turns(pane["name"]) + pane["turns"][1:]
                    seen.add(pane["name"])
    missing = set(EXTRA) - seen
    if missing:
        raise SystemExit(f"resdemo.EXTRA: no such agent: {', '.join(sorted(missing))}")
