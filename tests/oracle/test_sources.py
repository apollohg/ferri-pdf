import hashlib
from pathlib import Path
import tempfile
import unittest

from test_support import valid_bundle


class SourceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def test_heading_container_ids_are_preserved(self):
        from sources import index_sections
        source = self.root / "source.html"
        source.write_text('<h1 id="intro">Introduction</h1><section id="layout"><h2>Layout &amp; sizing</h2><p>A box must fit.</p><h3 id="layout-child">Layout</h3><p>Child boxes.</p></section>')
        sections = index_sections(source)
        self.assertEqual([s["id"] for s in sections], ["intro", "layout", "layout-child"])
        self.assertEqual(sections[1]["title"], "Layout & sizing")
        self.assertIn("must fit", sections[1]["text"])
        self.assertNotIn("Child boxes", sections[1]["text"])

    def test_microsoft_article_navigation_does_not_own_normative_body(self):
        from sources import index_sections
        source=self.root/'microsoft.html'
        source.write_text('<h1 id="head--font-header-table">head — Font Header Table</h1><h2 id="ms--in-this-article">In this article</h2><div class="content"><p>The unitsPerEm value must be in the required range.</p></div><h2 id="ms--feedback">Feedback</h2><p>Send feedback.</p>')
        sections={section['id']:section for section in index_sections(source)}
        self.assertIn('unitsPerEm',sections['head--font-header-table']['text'])
        self.assertNotIn('unitsPerEm',sections['ms--in-this-article']['text'])
        self.assertTrue(any('unitsPerEm' in item['text'] for item in sections['head--font-header-table']['clauses']))

    def test_nested_markup_and_duplicate_heading_titles_do_not_lose_sections(self):
        from sources import index_sections
        source = self.root / "source.html"
        source.write_text('<h1 id="title">Spec</h1><h2 id="one"><span class="secno">1.</span> Layout</h2><p>α text</p><h2 id="two">Layout</h2><p>β text</p>')
        sections = index_sections(source)
        self.assertEqual([s["id"] for s in sections], ["title", "one", "two"])
        self.assertIn("α", sections[1]["text"])
        self.assertIn("β", sections[2]["text"])

    def test_declared_legacy_encoding_preserves_normative_text(self):
        from sources import index_sections
        source = self.root / 'latin.html'
        source.write_bytes('<meta http-equiv="Content-Type" content="text/html; charset=iso-8859-1"><h1 id="intro">Règles</h1><p>Une boîte.</p>'.encode('iso-8859-1'))
        section = index_sections(source)[0]
        self.assertEqual(section['title'], 'Règles')
        self.assertIn('boîte', section['text'])

    def test_http_charset_is_used_when_a_legacy_page_has_no_meta(self):
        import json
        from sources import index_sections
        source = self.root / 'transport.html'
        source.write_bytes('<h1 id="intro">Règles</h1><p>Une boîte.</p>'.encode('iso-8859-1'))
        source.with_suffix('.headers.json').write_text(json.dumps({'charset': 'iso-8859-1'}))
        self.assertEqual(index_sections(source)[0]['title'], 'Règles')

    def test_markdown_specification_ignores_headings_inside_code_examples(self):
        from sources import index_sections
        source = self.root / 'font.md'
        source.write_text('# Font format\n\nA font has tables.\n\n```text\n# Not a section\n```\n\n## Metrics\n\nA glyph has an advance.\n')
        sections = index_sections(source)
        self.assertEqual([s['id'] for s in sections], ['font-format', 'metrics'])
        self.assertIn('advance', sections[1]['text'])

    def test_legacy_rfc_heading_spans_preserve_numbered_fragments(self):
        from sources import index_sections
        source = self.root / 'rfc.html'
        source.write_text('<pre><span class="h1">URI</span>\n<span class="h2"><a class="selflink" id="section-1" href="#section-1">1</a>. Syntax</span>\nA URI has a scheme.\n<span class="h3"><a class="selflink" id="section-1.1">1.1</a>. Names</span>\nNames are strings.</pre>')
        sections = index_sections(source)
        self.assertEqual([s['id'] for s in sections][1:], ['section-1', 'section-1.1'])
        self.assertIn('scheme', sections[1]['text'])

    def test_html_normative_reference_imports_are_discovered_without_a_normative_heading(self):
        from sources import index_sections, source_references
        source = self.root / "html-style-spec.html"
        source.write_text(
            '<h2 id="styles">Styles</h2><p>Decode using <a href="#refsENCODING">[ENCODING]</a>.</p>'
            '<h2 id="references">References</h2>'
            '<p>All references are normative unless marked "Non-normative".</p>'
            '<dl><dt id="refsENCODING">[ENCODING]</dt>'
            '<dd><a href="https://encoding.spec.whatwg.org/">Encoding</a>. WHATWG.</dd>'
            '<dt id="refsEXAMPLE">[EXAMPLE]</dt>'
            '<dd>(Non-normative) <a href="https://example.test/guide">Guide</a>.</dd></dl>'
        )
        references = source_references(index_sections(source))
        self.assertEqual(references, [{
            "url": "https://encoding.spec.whatwg.org/",
            "label": "ENCODING", "reference_id": "refsENCODING",
            "section_id": "references", "normative": True,
        }, {
            "url": "https://example.test/guide",
            "label": "EXAMPLE", "reference_id": "refsEXAMPLE",
            "section_id": "references", "normative": False,
        }])

    def test_css_normative_and_informative_references_keep_their_own_classification(self):
        from sources import index_sections, source_references
        source = self.root / "css-spec.html"
        source.write_text(
            '<h3 id="normative">Normative References</h3>'
            '<dl><dt id="biblio-css-syntax-3">[CSS-SYNTAX-3]</dt>'
            '<dd><a href="https://www.w3.org/TR/css-syntax-3/">CSS Syntax</a>.</dd></dl>'
            '<h3 id="informative">Informative References</h3>'
            '<dl><dt id="biblio-guide">[GUIDE]</dt><dd><a href="https://example.test/guide">Guide</a>.</dd></dl>'
        )
        references = source_references(index_sections(source))
        self.assertEqual([(r["label"], r["normative"]) for r in references],
                         [("CSS-SYNTAX-3", True), ("GUIDE", False)])
        self.assertEqual(references[0]["reference_id"], "biblio-css-syntax-3")

    def test_csswg_revision_metadata_is_verified_in_either_attribute_order(self):
        from sources import lock_source
        revision = "a" * 40
        for metadata in (
            f'<meta content="{revision}" name="revision">',
            f'<meta name="revision" content="{revision}">',
        ):
            with self.subTest(metadata=metadata):
                source = self.root / "draft.html"
                source.write_text(metadata + '<h1 id="names">Extension names</h1><p>Names start with two dashes.</p>')
                record = lock_source(source.as_uri(), "css-extensions-1", self.root / "cache", revision=revision)
                self.assertEqual(record["revision"], revision)
                self.assertEqual(record["sha256"], hashlib.sha256(source.read_bytes()).hexdigest())

    def test_rendered_editor_draft_pin_hashes_primary_and_rejects_mixed_revision(self):
        from sources import SourceError, lock_source
        revision = "a" * 40
        primary = self.root / "raw" / revision / "Overview.bs"
        primary.parent.mkdir(parents=True)
        primary.write_text("Status: ED\n# Extension names # {#names}\nNames start with two dashes.\n")
        rendered = self.root / "rendered.html"
        rendered.write_text(f'<meta content="{revision}" name="revision"><h1 id="names">Extension names</h1><p>Names start with two dashes.</p>')
        record = lock_source(primary.as_uri(), "extensions", self.root / "cache",
                             revision=revision, rendered_url=rendered.as_uri())
        self.assertEqual(record["url"], primary.as_uri())
        self.assertEqual(record["sha256"], hashlib.sha256(primary.read_bytes()).hexdigest())
        self.assertEqual(record["rendered"]["sha256"], hashlib.sha256(rendered.read_bytes()).hexdigest())
        self.assertNotEqual(record["sha256"], record["rendered"]["sha256"])
        self.assertEqual(record["sections"][0]["id"], "names")
        self.assertIn("two dashes", record["sections"][0]["text"])
        rendered.write_text(rendered.read_text().replace(revision, "b" * 40))
        with self.assertRaisesRegex(SourceError, "revision"):
            lock_source(primary.as_uri(), "extensions", self.root / "cache",
                        revision=revision, rendered_url=rendered.as_uri())

    def test_rendered_source_requires_matching_immutable_primary_revision(self):
        from sources import SourceError, lock_source
        revision = "a" * 40
        for url, selected in (
            ("https://raw.githubusercontent.com/w3c/csswg-drafts/main/Overview.bs", revision),
            ("https://raw.githubusercontent.com/w3c/csswg-drafts/" + revision + "/Overview.bs", "b" * 40),
        ):
            with self.subTest(url=url, selected=selected):
                with self.assertRaisesRegex(SourceError, "immutable|revision"):
                    lock_source(url, "extensions", self.root / "cache",
                                revision=selected, rendered_url="https://example.test/rendered")

    def test_rendered_source_manifest_rejects_mixed_revision_or_missing_hash(self):
        from manifest import ManifestError, validate_bundle
        bundle = valid_bundle(self.root)
        source = bundle["sources"]["css-sizing-3"]
        revision = "a" * 40
        source.update(url="https://raw.githubusercontent.com/w3c/csswg-drafts/" + revision + "/Overview.bs",
                      revision=revision, rendered={
                          "url": "https://drafts.csswg.org/css-sizing-3/",
                          "sha256": "b" * 64, "revision": revision,
                      })
        validate_bundle(bundle)
        source["rendered"]["revision"] = "c" * 40
        with self.assertRaisesRegex(ManifestError, "rendered|revision"):
            validate_bundle(bundle)
        source["rendered"]["revision"] = revision
        source["rendered"].pop("sha256")
        with self.assertRaisesRegex(ManifestError, "rendered|hash"):
            validate_bundle(bundle)

    def test_source_lock_command_exports_discovered_imports_for_review(self):
        import json
        import subprocess
        import sys
        source = self.root / "TR" / "2026" / "WD-example-20260101" / "index.html"
        source.parent.mkdir(parents=True)
        source.write_text(
            '<h1 id="rules">Rules</h1><p>A declaration uses CSS syntax.</p>'
            '<h2 id="normative">Normative References</h2><dl>'
            '<dt id="biblio-css-syntax-3">[CSS-SYNTAX-3]</dt>'
            '<dd><a href="https://www.w3.org/TR/css-syntax-3/">CSS Syntax</a>.</dd></dl>'
        )
        completed = subprocess.run([
            sys.executable, str(Path(__file__).with_name("sources.py")),
            "--lock", source.as_uri(), "--source-id", "example",
            "--cache-dir", str(self.root / "cache"),
        ], capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        record = json.loads(completed.stdout)
        self.assertEqual(record.get("reference_discovery"), [{
            "url": "https://www.w3.org/TR/css-syntax-3/",
            "label": "CSS-SYNTAX-3", "reference_id": "biblio-css-syntax-3",
            "section_id": "normative", "normative": True,
        }])

    def test_changed_section_index_is_not_accepted_under_old_source_lock(self):
        from manifest import ManifestError, load_bundle
        valid_bundle(self.root)
        index = self.root / "sections/css-sizing-3.json"
        index.write_text(index.read_text() + " ")
        with self.assertRaisesRegex(ManifestError, "index hash"):
            load_bundle(self.root)

    def test_moving_url_is_rejected_before_any_download(self):
        from sources import SourceError, lock_source
        with self.assertRaisesRegex(SourceError, "immutable"):
            lock_source("https://www.w3.org/TR/css-sizing-3/", "sizing", self.root)
        with self.assertRaisesRegex(SourceError, 'source ID'):
            lock_source('https://www.w3.org/TR/2026/NOTE-css-2026-20260622/', '../escape', self.root / 'cache')

    def test_versioned_unicode_and_rfc_sources_are_valid_immutable_pins(self):
        from manifest import validate_bundle
        bundle = valid_bundle(self.root)
        source = bundle['sources']['css-sizing-3']
        for url in ('https://www.unicode.org/reports/tr9/tr9-51.html', 'https://www.unicode.org/versions/Unicode17.0.0/core-spec/chapter-3/', 'https://www.rfc-editor.org/rfc/rfc3986.html', 'https://www.w3.org/TR/2010/REC-xml-stylesheet-20101028'):
            with self.subTest(url=url):
                source['url'] = url
                validate_bundle(bundle)

    def test_claimed_revision_must_match_published_source_metadata(self):
        from sources import SourceError, lock_source
        source = self.root / 'published.html'
        revision = 'a' * 40
        source.write_text(f'<meta name="git_commit_id" content="{revision}"><h1 id="metrics">Metrics</h1><p>Font metrics.</p>')
        with self.assertRaisesRegex(SourceError, 'revision'):
            lock_source(source.as_uri(), 'metrics', self.root/'cache', revision='b'*40)
        result = lock_source(source.as_uri(), 'metrics', self.root/'cache', revision=revision)
        self.assertEqual(result['revision'], revision)
        self.assertEqual(result['sha256'], hashlib.sha256(source.read_bytes()).hexdigest())

    def test_legacy_named_heading_anchor_retains_its_specification_fragment(self):
        from sources import index_sections
        source = self.root / "source.html"
        source.write_text('<h2><a name="box-model">8 Box model</a></h2><p>A box has margins.</p>')
        self.assertEqual(index_sections(source)[0]['id'], 'box-model')

    def test_omitted_paragraph_and_list_end_tags_do_not_merge_clauses(self):
        from sources import index_sections
        source = self.root / "source.html"
        source.write_text('<h1 id="intro">Spec</h1><p>First rule.<p>Second rule.<ul><li>One<li>Two</ul><h2 id="next">Next</h2><p>Next rule.')
        sections = index_sections(source)
        clauses = [c['text'] for c in sections[0]['clauses']]
        self.assertIn('First rule.', clauses)
        self.assertIn('Second rule.', clauses)
        self.assertIn('One', clauses)
        self.assertIn('Two', clauses)
        self.assertEqual(sections[1]['clauses'][0]['text'], 'Next rule.')

    def test_normative_source_cycles_are_allowed_unlike_verification_cycles(self):
        from manifest import validate_bundle
        bundle = valid_bundle(self.root)
        original = bundle["sources"]["css-sizing-3"]
        second = dict(original, id="css-values-3", dependencies=["css-sizing-3"])
        original["dependencies"] = ["css-values-3"]
        bundle["sources"][second["id"]] = second
        bundle["sections"][second["id"]] = [{"id": "intro", "title": "Introduction", "disposition": {"kind": "informative", "reason": "Informative introduction."}}]
        validate_bundle(bundle)


if __name__ == "__main__":
    unittest.main()
