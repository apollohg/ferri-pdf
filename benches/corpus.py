"""Deterministic synthetic business documents for the render benchmark."""
import argparse
import html
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = ROOT / "target/bench-corpus"
FONTS = ROOT / "tests/fixtures/fonts"
IMAGES = ROOT / "tests/oracle/assets"
SEED = 20261101
LCG_MULTIPLIER = 6364136223846793005
LCG_INCREMENT = 1442695040888963407
LCG_MASK = (1 << 64) - 1
LCG_SHIFT = 33

WORDS = (
    "account allocation analysis annual approval assessment balance baseline budget capacity "
    "commitment compliance consolidated contract coordination customer delivery department "
    "depreciation distribution efficiency estimate expenditure facility forecast governance "
    "implementation infrastructure initiative inventory investment logistics maintenance "
    "management measurement milestone objective operational organisation performance portfolio "
    "procurement programme projection quarterly reconciliation regional reporting requirement "
    "resource revenue schedule stakeholder strategy supplier sustainability transformation "
    "utilisation variance venue workforce the of and to in for with on by from across within"
).split()
SECTION_TITLES = (
    "Executive summary", "Market conditions", "Operational performance", "Financial results",
    "Customer outcomes", "Supply chain", "Workforce and culture", "Risk and compliance",
    "Capital programme", "Outlook",
)
REGIONS = ("North", "South", "East", "West", "Central", "Coastal", "Highlands", "Metro")
PRODUCTS = ("Standard plan", "Premium plan", "Field service", "Annual licence", "Support hours",
            "Training day", "Hardware kit", "Consulting block")
EVENT_TRACKS = (("Keynote", "#1f4e79"), ("Workshop", "#2e7d32"), ("Panel", "#8e24aa"),
                ("Networking", "#ef6c00"), ("Break", "#546e7a"))
ARABIC = "تقرير الأداء السنوي للعمليات الإقليمية والمالية"
HEBREW = "סיכום הפעילות השנתית של המחלקה והתקציב"
CJK = "漢字あア「水」、漢字。（ア）水あ〔字〕"

REPORT_SECTIONS = 10
REPORT_PARAGRAPHS = 6
REPORT_TABLE_ROWS = 36
WIDE_TABLE_ROWS = 1200
EVENT_DAYS = 4
EVENT_SESSIONS_PER_DAY = 18
PROSE_CHAPTERS = 6
PROSE_PARAGRAPHS = 22
PROSE_FOOTNOTE_EVERY = 3
INVOICE_LINES = 12


class Words:
    """A fixed-seed linear congruential stream, so every run writes identical documents."""

    def __init__(self, seed):
        self.state = seed

    def index(self, bound):
        self.state = (self.state * LCG_MULTIPLIER + LCG_INCREMENT) & LCG_MASK
        return (self.state >> LCG_SHIFT) % bound

    def sentence(self, low=8, high=18):
        words = [WORDS[self.index(len(WORDS))] for _ in range(low + self.index(high - low))]
        return words[0].capitalize() + " " + " ".join(words[1:]) + "."

    def paragraph(self, sentences=5):
        return " ".join(self.sentence() for _ in range(sentences))


def amount(value):
    return f"{value // 100:,}.{value % 100:02d}"


def document(title, css, body, lang="en"):
    return (f'<!doctype html><html lang="{lang}"><head><meta charset="utf-8">'
            f"<title>{html.escape(title)}</title><style>{css}</style></head>"
            f"<body>{body}</body></html>\n")


def report(words, assets):
    css = """
@page{size:A4;margin:28mm 20mm 24mm;
 @top-left{content:string(section);font:9pt sans-serif;color:#555}
 @top-right{content:'Annual operations report';font:9pt sans-serif;color:#555}
 @bottom-left{content:'Confidential draft';font:8pt sans-serif;color:#888}
 @bottom-right{content:'Page ' counter(page) ' of ' counter(pages);font:9pt sans-serif}}
@page:first{@top-left{content:none}@top-right{content:none}}
body{font:10.5pt/1.45 serif;color:#222}
h1{font:bold 24pt sans-serif;margin:0 0 12pt}
h2{font:bold 16pt sans-serif;color:#1f4e79;string-set:section content();break-before:page;margin:0 0 8pt}
h3{font:bold 12pt sans-serif;margin:14pt 0 4pt}
nav ol{list-style:none;padding:0}
nav a{color:inherit;text-decoration:none}
nav a::after{content:leader('.') target-counter(attr(href),page)}
ul,ol{margin:4pt 0 8pt}
table{border-collapse:collapse;width:100%;font:9pt sans-serif;margin:8pt 0}
th{background:#1f4e79;color:#fff;text-align:left;padding:3pt 5pt}
td{padding:3pt 5pt;border-bottom:0.5pt solid #ccc}
td.num,th.num{text-align:right}
tbody tr:nth-child(even){background:#eef3f8}
tr{break-inside:avoid}
"""
    toc = "".join(f'<li><a href="#s{index}">{index + 1}. {title}</a></li>'
                  for index, title in enumerate(SECTION_TITLES[:REPORT_SECTIONS]))
    body = [f"<h1>Annual operations report</h1><p>{words.paragraph()}</p>"
            f"<nav><h3>Contents</h3><ol>{toc}</ol></nav>"]
    row = 0
    for index, title in enumerate(SECTION_TITLES[:REPORT_SECTIONS]):
        body.append(f'<h2 id="s{index}">{index + 1}. {title}</h2>')
        for paragraph in range(REPORT_PARAGRAPHS):
            body.append(f"<p>{words.paragraph()}</p>")
            if paragraph == 1:
                body.append("<ul>" + "".join(f"<li>{words.sentence(5, 10)}</li>" for _ in range(4))
                            + "</ul>")
            if paragraph == 3:
                body.append("<ol>" + "".join(f"<li>{words.sentence(4, 8)}</li>" for _ in range(3))
                            + "</ol>")
        body.append(f"<h3>{title}: regional detail</h3><table><thead><tr><th>Region</th>"
                    "<th>Product</th><th class=num>Units</th><th class=num>Revenue</th>"
                    "<th class=num>Change</th></tr></thead><tbody>")
        for _ in range(REPORT_TABLE_ROWS):
            row += 1
            change = words.index(4000) - 2000
            body.append(f"<tr><td>{REGIONS[row % len(REGIONS)]}</td>"
                        f"<td>{PRODUCTS[words.index(len(PRODUCTS))]}</td>"
                        f"<td class=num>{words.index(9000) + 100:,}</td>"
                        f"<td class=num>{amount(words.index(90_000_000))}</td>"
                        f"<td class=num>{change / 100:+.2f}%</td></tr>")
        body.append("</tbody></table>")
    return document("Annual operations report", css, "".join(body))


def wide_table(words, assets):
    css = """
@page{size:A4 landscape;margin:16mm 12mm;
 @bottom-center{content:'Ledger page ' counter(page);font:8pt sans-serif}}
body{font:8pt/1.3 sans-serif}
h1{font-size:14pt;margin:0 0 6pt}
h1+p{margin:0 0 6pt}
table{width:100%;margin-bottom:12pt}
table.collapse{border-collapse:collapse;border:1pt solid #333}
table.separate{border-collapse:separate;border-spacing:1pt;border:0.75pt solid #333}
th{background:#ddd;border:0.75pt solid #333;padding:2pt 4pt;text-align:left}
td{border:0.5pt solid #999;padding:1.5pt 4pt}
td.num,th.num{text-align:right;font-variant-numeric:tabular-nums}
.separate td{border-color:#bbb}
section+section{break-before:page}
"""
    columns = ("Entry", "Date", "Account", "Description", "Region", "Debit", "Credit", "Balance",
               "Tax", "Status")
    numeric = {"Debit", "Credit", "Balance", "Tax"}
    head = "".join(f'<th{" class=num" if name in numeric else ""}>{name}</th>' for name in columns)
    body = []
    for mode in ("collapse", "separate"):
        balance = 0
        rows = []
        for index in range(WIDE_TABLE_ROWS):
            debit, credit = words.index(500_000), words.index(500_000)
            balance += debit - credit
            rows.append(f"<tr><td>E{index + 1:06d}</td><td>2026-{index % 12 + 1:02d}-"
                        f"{index % 28 + 1:02d}</td><td>{4000 + words.index(900)}</td>"
                        f"<td>{words.sentence(3, 6)}</td><td>{REGIONS[index % len(REGIONS)]}</td>"
                        f"<td class=num>{amount(debit)}</td><td class=num>{amount(credit)}</td>"
                        f"<td class=num>{amount(abs(balance))}</td>"
                        f"<td class=num>{amount(debit // 10)}</td>"
                        f"<td>{'Posted' if index % 5 else 'Pending'}</td></tr>")
        body.append(f'<section><h1>General ledger ({mode} borders)</h1><p>{words.sentence()}</p>'
                    f'<table class="{mode}"><thead><tr>{head}</tr></thead><tbody>'
                    + "".join(rows) + "</tbody></table></section>")
    return document("General ledger", css, "".join(body))


def event_plan(words, assets):
    css = f"""
@font-face{{font-family:Brand;src:url('{assets["brand_regular"]}') format('woff2');font-weight:400}}
@font-face{{font-family:Brand;src:url('{assets["brand_bold"]}');font-weight:700}}
@font-face{{font-family:Display;src:url('{assets["display"]}')}}
@page{{size:A4;margin:18mm 14mm;@bottom-right{{content:counter(page);font:8pt Brand}}}}
body{{font:9.5pt/1.4 Brand,sans-serif;color:#263238;margin:0}}
header{{display:flex;justify-content:space-between;align-items:center;padding:10pt 14pt;
 background:linear-gradient(90deg,#1f4e79,#3f7fbf);color:#fff;border-radius:8pt}}
header h1{{font:24pt Display;margin:0}}
header img{{width:60pt;height:30pt;border-radius:4pt}}
h2{{font:18pt Display;margin:16pt 0 8pt;break-after:avoid}}
.day{{display:grid;grid-template-columns:repeat(3,1fr);gap:8pt}}
.card{{border:0.75pt solid #cfd8dc;border-radius:6pt;padding:8pt;background:#fff;
 box-shadow:0 1.5pt 4pt rgba(0,0,0,.25);break-inside:avoid;display:flex;flex-direction:column;gap:4pt}}
.card .top{{display:flex;align-items:center;gap:6pt}}
.card img{{width:28pt;height:28pt;border-radius:50%;object-fit:cover}}
.card h3{{font-size:10.5pt;margin:0;font-weight:700}}
.time{{font-style:italic;color:#607d8b}}
.badge{{display:inline-block;padding:1pt 5pt;border-radius:8pt;color:#fff;font-size:7.5pt;
 font-weight:700;text-transform:uppercase;letter-spacing:.4pt}}
.meta{{display:flex;flex-wrap:wrap;gap:3pt}}
.meta span{{border:0.5pt solid #b0bec5;border-radius:3pt;padding:0 3pt;font-size:7.5pt}}
.summary{{display:flex;gap:10pt;margin:10pt 0}}
.summary div{{flex:1;border-radius:6pt;padding:8pt;background:#eceff1;text-align:center}}
.summary strong{{display:block;font:bold 16pt Brand}}
"""
    images = (assets["png"], assets["jpeg"], assets["svg"])
    body = [f'<header><h1>Partner summit plan</h1><img src="{assets["svg"]}" alt="Logo"></header>'
            '<div class="summary">' + "".join(
                f"<div><strong>{value}</strong>{label}</div>" for value, label in
                (("4", "days"), ("72", "sessions"), ("1,250", "guests"), ("12", "venues")))
            + "</div>"]
    for day in range(EVENT_DAYS):
        body.append(f'<h2>Day {day + 1}</h2><div class="day">')
        for session in range(EVENT_SESSIONS_PER_DAY):
            track, colour = EVENT_TRACKS[(day + session) % len(EVENT_TRACKS)]
            hour = 8 + session // 2
            body.append(
                f'<div class="card"><div class="top"><img src="{images[session % len(images)]}" '
                f'alt="Host"><div><h3>{words.sentence(2, 5)[:-1]}</h3>'
                f'<span class="time">{hour:02d}:{30 * (session % 2):02d} &ndash; '
                f'{hour + 1:02d}:{30 * (session % 2):02d}</span></div></div>'
                f'<div><span class="badge" style="background:{colour}">{track}</span></div>'
                f"<p>{words.sentence(10, 20)} <b>{words.sentence(3, 5)}</b> "
                f"<i>{words.sentence(3, 6)}</i></p>"
                f'<div class="meta"><span>Room {1 + words.index(30)}</span>'
                f"<span>{words.index(200) + 20} seats</span>"
                f"<span>{REGIONS[session % len(REGIONS)]} hall</span></div></div>")
        body.append("</div>")
    return document("Partner summit plan", css, "".join(body))


def prose(words, assets):
    css = f"""
@font-face{{font-family:Cjk;src:url('{assets["cjk"]}')}}
@page{{size:A5;margin:18mm 16mm 20mm;
 @bottom-center{{content:counter(page);font:8pt serif}}
 @top-center{{content:string(chapter);font:italic 8pt serif}}}}
body{{font:10pt/1.4 serif;text-align:justify;hyphens:auto}}
h2{{font:bold 14pt serif;string-set:chapter content();break-before:page;text-align:left}}
p{{margin:0;text-indent:1.2em;orphans:2;widows:2}}
.fn{{float:footnote;font-size:8pt;text-indent:0}}
.rtl{{font-family:sans-serif}}
.cjk{{font-family:Cjk,serif;text-align:left}}
"""
    body = []
    note = 0
    for chapter in range(PROSE_CHAPTERS):
        body.append(f"<h2>Chapter {chapter + 1}: {SECTION_TITLES[chapter % len(SECTION_TITLES)]}</h2>")
        for paragraph in range(PROSE_PARAGRAPHS):
            text = words.paragraph(7)
            if paragraph % PROSE_FOOTNOTE_EVERY == 0:
                note += 1
                text += f'<span class="fn">Note {note}: {words.sentence(8, 16)}</span>'
            body.append(f"<p>{text}</p>")
            if paragraph == 8:
                body.append(f'<p class="rtl" lang="ar" dir="rtl">{ARABIC} '
                            f'<span dir="ltr">FY2026 Q{chapter % 4 + 1}</span> {ARABIC}</p>'
                            f'<p class="rtl" lang="he" dir="rtl">{HEBREW} 2026 {HEBREW}</p>')
            if paragraph == 15:
                body.append(f'<p class="cjk" lang="ja">{CJK * 4}</p>')
    return document("Operations handbook", css, "".join(body))


def invoice(words, assets):
    css = """
@page{size:A4;margin:16mm}
body{font:10pt/1.4 sans-serif;color:#222}
.head{display:flex;justify-content:space-between;align-items:flex-start;margin-bottom:16pt}
.head img{width:80pt;height:40pt}
.head h1{font-size:22pt;margin:0;color:#1f4e79}
.parties{display:flex;gap:24pt;margin-bottom:14pt}
.parties div{flex:1}
table{width:100%;border-collapse:collapse}
th{text-align:left;border-bottom:1pt solid #1f4e79;padding:4pt}
td{padding:4pt;border-bottom:0.5pt solid #ddd}
.num{text-align:right}
tfoot td{font-weight:bold;border-top:1pt solid #1f4e79;border-bottom:none}
.note{margin-top:18pt;font-size:8.5pt;color:#555}
"""
    lines = []
    subtotal = 0
    for index in range(INVOICE_LINES):
        quantity, price = 1 + words.index(9), 1000 + words.index(90_000)
        subtotal += quantity * price
        lines.append(f"<tr><td>{PRODUCTS[index % len(PRODUCTS)]}</td><td>{words.sentence(3, 6)}</td>"
                     f"<td class=num>{quantity}</td><td class=num>{amount(price)}</td>"
                     f"<td class=num>{amount(quantity * price)}</td></tr>")
    tax = subtotal // 10
    body = (f'<div class="head"><div><h1>Invoice</h1><p>No. INV-2026-0421<br>Issued 1 November 2026'
            f'</p></div><img src="{assets["svg"]}" alt="Logo"></div><div class="parties">'
            "<div><b>From</b><br>Example Services Pty Ltd<br>1 Sample Street<br>Exampletown</div>"
            "<div><b>Bill to</b><br>Placeholder Industries<br>99 Demo Road<br>Testville</div></div>"
            "<table><thead><tr><th>Item</th><th>Description</th><th class=num>Qty</th>"
            "<th class=num>Unit</th><th class=num>Amount</th></tr></thead><tbody>"
            + "".join(lines) + "</tbody><tfoot>"
            f"<tr><td colspan=4>Subtotal</td><td class=num>{amount(subtotal)}</td></tr>"
            f"<tr><td colspan=4>Tax</td><td class=num>{amount(tax)}</td></tr>"
            f"<tr><td colspan=4>Total due</td><td class=num>{amount(subtotal + tax)}</td></tr>"
            f'</tfoot></table><p class="note">{words.paragraph(2)}</p>')
    return document("Invoice", css, body)


DOCUMENTS = {
    "report": report,
    "wide-table": wide_table,
    "event-plan": event_plan,
    "prose": prose,
    "invoice": invoice,
}


def write_corpus(output):
    """Write every document to `output`; returns name -> path."""
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    relative = lambda path: Path(os.path.relpath(path, output)).as_posix()
    assets = {
        "brand_regular": relative(FONTS / "WoffTest-Regular.woff2"),
        "brand_bold": relative(FONTS / "WoffTest-Bold.ttf"),
        "display": relative(FONTS / "SourceSerif4-Regular.ttf"),
        "cjk": relative(FONTS / "CJKWritingTest.otf"),
        "png": relative(IMAGES / "image-pattern.png"),
        "jpeg": relative(IMAGES / "image-pattern.jpg"),
        "svg": relative(IMAGES / "image-pattern.svg"),
    }
    paths = {}
    for name, build in DOCUMENTS.items():
        path = output / f"{name}.html"
        path.write_text(build(Words(SEED), assets), encoding="utf-8")
        paths[name] = path
    return paths


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    for name, path in write_corpus(parser.parse_args().output).items():
        print(f"{name}: {path}")


if __name__ == "__main__":
    main()
