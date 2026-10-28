"""Explicit maintenance of immutable specification sources and section indexes."""
import argparse
from datetime import datetime, timezone
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import urllib.error
import urllib.request

from manifest import ID, REVISION, SCHEMA_VERSION, SOURCE_PIN

FETCH_TIMEOUT_S = 30
MAX_SOURCE_BYTES = 32 * 1024 * 1024
SOURCE_ENCODING_SCAN_BYTES = 8192
HEADING_TAGS = {f"h{level}" for level in range(1, 7)}
VOID_TAGS = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"}
CLAUSE_TAGS = {"p", "li", "dt", "dd", "pre", "td", "th"}
BLOCK_TAGS = {"address", "article", "aside", "blockquote", "div", "dl", "fieldset", "footer", "form", "header", "hr", "main", "nav", "ol", "p", "pre", "section", "table", "ul"} | HEADING_TAGS


class SourceError(ValueError):
    pass


class SectionParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.stack = []
        self.sections = []
        self.body_section = None
        self.heading = None
        self.heading_tag = None
        self.clauses = []
        self.link = None
        self.ignored = 0

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        rfc_heading = tag == 'span' and attrs.get('class') in HEADING_TAGS
        is_heading = tag in HEADING_TAGS or rfc_heading
        if tag in BLOCK_TAGS:
            self._finish_clause("p")
        if tag == "li":
            self._finish_clause("li")
        if tag in {"dt", "dd"}:
            self._finish_clause("dt")
            self._finish_clause("dd")
        if tag in {"td", "th", "tr"}:
            self._finish_clause("td")
            self._finish_clause("th")
        if is_heading:
            while self.clauses:
                self._finish_clause(self.clauses[-1][0])
        if tag in {"script", "style"}:
            self.ignored += 1
        if is_heading:
            identity = attrs.get("id") or next((values.get("id") for name, values in reversed(self.stack) if values.get("id")), None)
            self.heading_tag = tag
            level = attrs['class'][1] if rfc_heading else tag[1]
            self.heading = {"id": identity, "title": "", "text": "", "clauses": [], "links": [], "level": int(level)}
        if tag in CLAUSE_TAGS and not self.ignored:
            self.clauses.append((tag, [], attrs.get("id"), []))
        if tag == "a" and attrs.get("href"):
            self.link = {"url": attrs["href"], "text": ""}
        if tag == "a" and self.heading and attrs.get("name"):
            self.heading["id"] = attrs["name"]
        if tag == 'a' and self.heading and attrs.get('class') == 'selflink' and attrs.get('id'):
            self.heading['id'] = attrs['id']
        if tag not in VOID_TAGS:
            self.stack.append((tag, attrs))
        if tag in {"br", "p", "li", "dt", "dd", "td", "th"} and self.body_section:
            self.body_section["text"] += " "

    def handle_endtag(self, tag):
        if tag == self.heading_tag and self.heading:
            heading = self.heading
            heading["title"] = " ".join(heading["title"].split())
            if not heading["id"]:
                heading["id"] = "heading-" + hashlib.sha256(f"{len(self.sections)}:{heading['title']}".encode()).hexdigest()[:12]
            self.sections.append(heading)
            if heading["id"] not in {"ms--in-this-article","side-doc-outline"}:
                self.body_section = heading
            self.heading = None
        if tag in {"ul", "ol"}:
            self._finish_clause("li")
        if tag in CLAUSE_TAGS:
            self._finish_clause(tag)
        if tag == "a" and self.link:
            if self.body_section:
                self.link["text"] = " ".join(self.link["text"].split())
                self.body_section["links"].append(self.link)
            for _, _, _, links in self.clauses:
                links.append(self.link)
            self.link = None
        for index in range(len(self.stack) - 1, -1, -1):
            if self.stack[index][0] == tag:
                del self.stack[index:]
                break
        if tag in {"script", "style"} and self.ignored:
            self.ignored -= 1

    def _finish_clause(self, tag):
        for index in range(len(self.clauses) - 1, -1, -1):
            if self.clauses[index][0] == tag:
                _, parts, identity, links = self.clauses.pop(index)
                text = " ".join("".join(parts).split())
                if text and self.body_section:
                    clause = {"tag": tag, "text": text}
                    if identity:
                        clause["id"] = identity
                    if links:
                        clause["links"] = links
                    self.body_section["clauses"].append(clause)
                return

    def handle_data(self, data):
        if self.ignored:
            return
        if self.heading:
            self.heading["title"] += data
        elif self.body_section:
            self.body_section["text"] += data
        for _, parts, _, _ in self.clauses:
            parts.append(data)
        if self.link:
            self.link["text"] += data


def index_markdown(text: str) -> list[dict]:
    sections = []
    fenced = False
    for line in text.splitlines():
        if line.lstrip().startswith(('```', '~~~')):
            fenced = not fenced
        match = re.match(r'^(#{1,6})\s+(.+?)\s*#*$', line) if not fenced else None
        if match:
            title = match[2]
            identity = re.sub(r'[^\w\s-]', '', title).strip().lower().replace(' ', '-')
            suffix = sum(section['id'] == identity or section['id'].startswith(identity + '-') for section in sections)
            if suffix:
                identity += '-' + str(suffix)
            sections.append({'id': identity, 'title': title, 'text': '', 'clauses': [], 'links': [], 'level': len(match[1])})
        elif sections:
            sections[-1]['text'] += line + '\n'
            if line.strip() and not fenced:
                sections[-1]['clauses'].append({'tag': 'p', 'text': line.strip()})
                sections[-1]['links'].extend({'text': label, 'url': url} for label, url in re.findall(r'\[([^\]]+)\]\(([^)]+)\)', line))
    return sections


def index_sections(path: Path) -> list[dict]:
    parser = SectionParser()
    data = Path(path).read_bytes()
    declaration = re.search(rb'<meta\b[^>]*charset\s*=\s*[\x22\x27]?([a-zA-Z0-9_-]+)', data[:SOURCE_ENCODING_SCAN_BYTES], re.I)
    encoding = declaration[1].decode("ascii") if declaration else "utf-8-sig"
    headers_path = Path(path).with_suffix('.headers.json')
    if headers_path.exists():
        headers = json.loads(headers_path.read_text())
        encoding = headers.get('charset') or encoding
    try:
        text = data.decode(encoding)
        if Path(path).suffix == '.md':
            parser.sections = index_markdown(text)
        else:
            parser.feed(text)
    except (LookupError, UnicodeError) as error:
        raise SourceError(f"{path}: cannot decode declared encoding {encoding}: {error}") from error
    while parser.clauses:
        parser._finish_clause(parser.clauses[-1][0])
    identities = set()
    for section in parser.sections:
        if section["id"] in identities:
            raise SourceError(f"{path}: duplicate section ID {section['id']!r}")
        identities.add(section["id"])
        section["text"] = " ".join(section["text"].split())
    if not parser.sections:
        raise SourceError(f"{path}: no indexed headings")
    return parser.sections



def source_references(sections: list[dict]) -> list[dict]:
    """Retain normative status and citation identity from CSS/HTML bibliographies."""
    references = []
    for section in sections:
        identity = section["id"]
        html_bibliography = identity == "references" and (
            'All references are normative unless marked "Non-normative".' in section["text"]
        )
        if identity not in {"normative", "informative"} and not html_bibliography:
            continue
        label = None
        reference_id = None
        for clause in section["clauses"]:
            if clause["tag"] == "dt":
                label = clause["text"].strip("[]")
                reference_id = clause.get("id")
            elif clause["tag"] == "dd" and label:
                normative = identity != "informative" and not re.search(r"\bNon-normative\b", clause["text"], re.I)
                seen = set()
                for link in clause.get("links", []):
                    url = link["url"]
                    if not url.startswith(("https://", "http://")) or url in seen:
                        continue
                    seen.add(url)
                    references.append({
                        "url": url, "label": label, "reference_id": reference_id,
                        "section_id": identity, "normative": normative,
                    })
    return references


def fetch_source(url: str, path: Path) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "ferri-pdf-standards-inventory/1"})
    try:
        with urllib.request.urlopen(request, timeout=FETCH_TIMEOUT_S) as response:
            data = response.read(MAX_SOURCE_BYTES + 1)
            charset = response.headers.get_content_charset()
    except (OSError, urllib.error.URLError) as error:
        raise SourceError(f"{url}: {error}") from error
    if len(data) > MAX_SOURCE_BYTES:
        raise SourceError(f"{url}: source exceeds {MAX_SOURCE_BYTES} bytes")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    path.with_suffix('.headers.json').write_text(json.dumps({'charset': charset, 'url': url}) + '\n')
    return data


def lock_source(url: str, source_id: str, cache_dir: Path, *, revision: str | None = None,
                rendered_url: str | None = None) -> dict:
    if not ID.fullmatch(source_id):
        raise SourceError(f'{source_id!r}: invalid source ID')
    if revision is not None and not REVISION.fullmatch(revision):
        raise SourceError(f"{url}: invalid fixed revision")
    if not SOURCE_PIN.search(url) and not revision:
        raise SourceError(f"{url}: an immutable source URL is required")
    if rendered_url and (not revision or not SOURCE_PIN.search(url) or f"/{revision}/" not in url):
        raise SourceError(f"{url}: rendered source requires a matching immutable primary revision")
    location = url.split('?', 1)[0]
    suffix = '.bs' if location.endswith('.bs') else '.md' if location.endswith('.md') else '.html'
    path = Path(cache_dir) / f"{source_id}{suffix}"
    data = fetch_source(url, path)
    indexed_path = path
    metadata_data = data
    metadata_url = url
    rendered = None
    if rendered_url:
        indexed_path = Path(cache_dir) / f"{source_id}.rendered.html"
        metadata_data = fetch_source(rendered_url, indexed_path)
        metadata_url = rendered_url
        rendered = {
            "url": rendered_url, "cache_path": str(indexed_path),
            "sha256": hashlib.sha256(metadata_data).hexdigest(), "revision": revision,
        }
    if revision:
        declared = re.search(
            rb'<meta\b(?=[^>]*\sname=["\'](?:git_commit_id|revision)["\'])'
            rb'(?=[^>]*\scontent=["\']([0-9a-f]{40})["\'])[^>]*>',
            metadata_data, re.I,
        )
        if not declared or declared[1].decode('ascii') != revision:
            raise SourceError(f"{metadata_url}: published source revision does not match {revision}")
    sections = index_sections(indexed_path)
    record = {
        "id": source_id, "url": url, "sha256": hashlib.sha256(data).hexdigest(),
        "retrieved_at": datetime.now(timezone.utc).isoformat(),
        "cache_path": str(path), "section_count": len(sections),
        "sections": sections,
        "reference_discovery": source_references(sections),
    }
    if revision:
        record['revision'] = revision
    if rendered:
        record['rendered'] = rendered
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lock", required=True)
    parser.add_argument("--source-id", required=True)
    parser.add_argument("--cache-dir", type=Path, required=True)
    parser.add_argument("--revision", help="upstream revision declared by the published source")
    parser.add_argument("--rendered-url", help="rendered editor draft declaring the immutable primary source revision")
    args = parser.parse_args()
    try:
        record = lock_source(args.lock, args.source_id, args.cache_dir, revision=args.revision, rendered_url=args.rendered_url)
    except SourceError as error:
        parser.error(str(error))
    print(json.dumps({"schema_version": SCHEMA_VERSION, **record}, indent=2))


if __name__ == "__main__":
    main()
