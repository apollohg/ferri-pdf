"""Scoped coverage must not turn missing source or output evidence into a pass."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from manifest import ManifestError, load_bundle
from test_support import TEST_REQUIREMENT_ID, TEST_SOURCE_ID, valid_bundle, write_json


class FoundationsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bundle = valid_bundle(self.root)
        self.case_id = "invoices-small"
        self.behavior_id = "foundations.explicit-width"
        self.contract = {
            "schema_version": 1,
            "id": "test-foundations",
            "required_group_ids": ["values-sizing"],
            "required_behavior_ids": [self.behavior_id],
            "required_requirement_ids": [TEST_REQUIREMENT_ID],
            "behaviors": [{
                "id": self.behavior_id,
                "group": "values-sizing",
                "requirement_ids": [TEST_REQUIREMENT_ID],
                "source_id": TEST_SOURCE_ID,
                "section_ids": ["width"],
                "owner_task": 21,
                "implementation_files": ["src/layout/geometry.rs"],
                "default_obligations": ["Auto widths use their containing block."],
                "invalid_obligations": ["Negative specified widths are discarded."],
                "interaction_obligations": ["Border-box changes the content width."],
                "case_ids": [],
                "handoff": None,
            }],
            "source_reviews": [{
                "source_id": TEST_SOURCE_ID,
                "sha256": "a" * 64,
                "section_ids": ["width"],
                "review_state": "reviewed",
                "import_dispositions": [],
                "overlap_dispositions": [],
            }],
        }

    def api(self):
        import foundations
        return foundations

    def verified_record(self):
        requirement = self.bundle["requirements"][TEST_REQUIREMENT_ID]
        requirement["review_state"] = "reviewed"
        requirement["case_ids"] = [self.case_id]
        requirement["verification"] = [{"case_id": self.case_id, "assertion_id": "width"}]
        self.contract["behaviors"][0]["case_ids"] = [self.case_id]
        self.bundle["cases"][self.case_id]["assertions"] = [{
            "id": "width",
            "kind": "page-boxes",
            "expected": {"width_pt": 100, "height_pt": 200},
            "basis": {
                "kind": "specification", "source_id": TEST_SOURCE_ID,
                "sections": ["width"], "sha256": "a" * 64,
                "reason": "Independent numeric width contract.",
            },
        }]
        return {self.case_id: {
            "case_id": self.case_id, "status": "rendered",
            "assertions": [{"id": "width", "status": "pass"}],
        }}

    def evaluate(self, cases=None):
        return self.api().evaluate_foundations(self.bundle, self.contract, cases or {})

    def test_missing_behavior_blocks_scope(self):
        # Dropping evidence must never make an implemented-looking property pass.
        self.bundle["requirements"][TEST_REQUIREMENT_ID]["implementation"]["status"] = "present"
        result = self.evaluate()
        self.assertEqual(result["status"], "blocked")
        self.assertIn(TEST_REQUIREMENT_ID, result["unverified_requirement_ids"])
        self.assertEqual(result["behaviors"][self.behavior_id]["status"], "unverified")

    def test_unresolved_controlling_import_blocks_scope(self):
        records = self.verified_record()
        self.bundle["sources"][TEST_SOURCE_ID]["reference_dispositions"] = [{
            "url": "https://example.test/unpinned", "status": "unresolved",
            "section_id": "width", "label": "Required width definition",
        }]
        result = self.evaluate(records)
        self.assertEqual(result["status"], "blocked")
        self.assertEqual(result["unresolved_imports"][0]["url"], "https://example.test/unpinned")

    def test_later_handoff_cannot_erase_foundation(self):
        self.contract["behaviors"][0]["handoff"] = {
            "workstream": "later-layout", "reason": "Not implemented yet."
        }
        with self.assertRaisesRegex(ManifestError, "included|handoff"):
            self.api().validate_foundations(self.bundle, self.contract)
        self.contract["behaviors"] = []
        with self.assertRaisesRegex(ManifestError, "missing|coverage"):
            self.api().validate_foundations(self.bundle, self.contract)

    def test_focused_evidence_does_not_pass_global_profile(self):
        records = self.verified_record()
        other = copy.deepcopy(self.bundle["requirements"][TEST_REQUIREMENT_ID])
        other.update(id="books.footnotes", review_state="unreviewed", case_ids=[], verification=[])
        self.bundle["requirements"][other["id"]] = other
        from conformance import evaluate_requirements
        self.assertEqual(self.evaluate(records)["status"], "pass")
        self.assertEqual(evaluate_requirements(self.bundle, records)["books.footnotes"]["status"], "unverified")

    def test_duplicate_ids_missing_hashes_and_unknown_sections_are_rejected(self):
        mutations = [
            lambda c: c["behaviors"].append(copy.deepcopy(c["behaviors"][0])),
            lambda c: c["source_reviews"][0].pop("sha256"),
            lambda c: c["source_reviews"][0].update(sha256="b" * 64),
            lambda c: c["behaviors"][0].update(section_ids=["missing"]),
            lambda c: c["required_behavior_ids"].append(self.behavior_id),
            lambda c: c["required_group_ids"].append("missing-group"),
            lambda c: c["behaviors"][0].update(owner_task=True),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                contract = copy.deepcopy(self.contract)
                mutate(contract)
                with self.assertRaises(ManifestError):
                    self.api().validate_foundations(self.bundle, contract)

    def test_evidence_for_an_unrelated_requirement_is_rejected(self):
        self.verified_record()
        self.bundle["cases"][self.case_id]["requirement_ids"] = []
        with self.assertRaisesRegex(ManifestError, "requirement|evidence"):
            self.api().validate_foundations(self.bundle, self.contract)

    def test_missing_source_review_and_unreviewed_overlap_block_scope(self):
        records = self.verified_record()
        self.contract["source_reviews"][0]["review_state"] = "pending"
        self.assertEqual(self.evaluate(records)["status"], "blocked")
        self.contract["source_reviews"][0]["review_state"] = "reviewed"
        other = copy.deepcopy(self.bundle["sources"][TEST_SOURCE_ID])
        other["id"] = "css-sizing-4"
        self.bundle["sources"][other["id"]] = other
        self.bundle["sections"][other["id"]] = copy.deepcopy(self.bundle["sections"][TEST_SOURCE_ID])
        self.bundle["sources"][TEST_SOURCE_ID]["overlap_dispositions"] = [{
            "status": "unresolved", "other_source_id": other["id"],
            "reason": "Cross-level algorithm not reviewed.",
        }]
        result = self.evaluate(records)
        self.assertEqual(result["status"], "blocked")
        self.assertEqual(result["unresolved_overlaps"][0]["other_source_id"], "css-sizing-4")

    def test_warning_failed_output_and_missing_dependency_never_pass(self):
        records = self.verified_record()
        records[self.case_id]["warnings_unresolved"] = True
        self.assertEqual(self.evaluate(records)["status"], "blocked")
        records[self.case_id].pop("warnings_unresolved")
        records[self.case_id]["assertions"][0]["status"] = "fail"
        self.assertEqual(self.evaluate(records)["status"], "fail")
        dependency = copy.deepcopy(self.bundle["requirements"][TEST_REQUIREMENT_ID])
        dependency.update(id="sizing.percentage-basis", verification=[], case_ids=[], review_state="unreviewed")
        self.bundle["requirements"][dependency["id"]] = dependency
        self.bundle["requirements"][TEST_REQUIREMENT_ID]["dependencies"] = [dependency["id"]]
        records[self.case_id]["assertions"][0]["status"] = "pass"
        self.assertEqual(self.evaluate(records)["status"], "blocked")

    def test_scoped_import_disposition_needs_pinned_proof_and_cannot_hide_required_import(self):
        self.bundle["sources"][TEST_SOURCE_ID]["reference_dispositions"] = [{
            "url": "https://example.test/interactive", "status": "unresolved",
            "section_id": "width",
        }]
        disposition = {
            "url": "https://example.test/interactive", "status": "outside-scope",
            "reason": "Only live event dispatch uses this reference.",
            "basis": {"source_id": TEST_SOURCE_ID, "section_id": "width", "sha256": "a" * 64},
        }
        self.contract["source_reviews"][0]["import_dispositions"] = [disposition]
        self.assertEqual(self.evaluate(self.verified_record())["status"], "pass")
        disposition["basis"]["sha256"] = "b" * 64
        with self.assertRaises(ManifestError):
            self.api().validate_foundations(self.bundle, self.contract)
        disposition["basis"]["sha256"] = "a" * 64
        disposition["status"] = "required"
        self.assertEqual(self.evaluate(self.verified_record())["status"], "blocked")

    def test_cli_validates_scope_without_renderer_or_poppler(self):
        write_json(self.root / "foundations.json", self.contract)
        command = [
            sys.executable, str(Path(__file__).with_name("run.py")),
            "--manifest-dir", str(self.root), "--foundations", "--validate-manifests",
        ]
        completed = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("foundations", completed.stdout.lower())
        self.contract["behaviors"] = []
        write_json(self.root / "foundations.json", self.contract)
        rejected = subprocess.run(command, capture_output=True, text=True)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("coverage", rejected.stderr.lower())

    def test_source_anchor_punctuation_is_not_rejected_as_a_behavior_id(self):
        anchor = "number-state-(type=number)"
        self.bundle["sections"][TEST_SOURCE_ID][0]["id"] = anchor
        self.bundle["requirements"][TEST_REQUIREMENT_ID]["sections"] = [anchor]
        self.contract["behaviors"][0]["section_ids"] = [anchor]
        self.contract["source_reviews"][0]["section_ids"] = [anchor]
        self.api().validate_foundations(self.bundle, self.contract)
        self.assertEqual(self.evaluate()["status"], "blocked")

    def test_malformed_nested_scope_data_reports_manifest_location(self):
        mutations = [
            lambda c: c["source_reviews"][0].update(review_state=[]),
            lambda c: c["behaviors"][0].update(group=[]),
            lambda c: c["behaviors"][0].update(requirement_ids=[{}]),
            lambda c: c["source_reviews"][0].update(import_dispositions=[{
                "url": "https://example.test/required", "status": [],
                "reason": "Required definition", "basis": {},
            }]),
            lambda c: c.update(handoffs=[{"workstream": "books", "reason": "Later", "requirement_ids": [{}]}]),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                contract = copy.deepcopy(self.contract)
                mutate(contract)
                with self.assertRaises(ManifestError) as caught:
                    self.api().validate_foundations(self.bundle, contract)
                self.assertEqual(caught.exception.file, self.root / "foundations.json")
                self.assertTrue(caught.exception.pointer.startswith("/"))

    def test_scoped_selection_includes_dependency_evidence_with_distinct_case_ids(self):
        self.verified_record()
        dependency = copy.deepcopy(self.bundle["requirements"][TEST_REQUIREMENT_ID])
        dependency.update(id="sizing.minimum", case_ids=["tables-small"],
                          verification=[{"case_id": "tables-small", "assertion_id": "minimum"}])
        self.bundle["requirements"][dependency["id"]] = dependency
        self.bundle["requirements"][TEST_REQUIREMENT_ID]["dependencies"] = [dependency["id"]]
        self.assertEqual(self.api().required_case_ids(self.bundle, self.contract),
                         ["invoices-small", "tables-small"])

    def test_scope_contract_changes_invalidate_recorded_profile(self):
        from reporting import profile_fingerprint
        self.bundle["foundations"] = self.contract
        fingerprint = profile_fingerprint(self.bundle)
        self.contract["behaviors"][0]["owner_task"] = 22
        self.assertNotEqual(profile_fingerprint(self.bundle), fingerprint)

    def test_cli_without_declared_evidence_blocks_without_rendering(self):
        write_json(self.root / "foundations.json", self.contract)
        output = self.root / "output"
        completed = subprocess.run([
            sys.executable, str(Path(__file__).with_name("run.py")),
            "--manifest-dir", str(self.root), "--foundations", "--require-conformance",
            "--output-dir", str(output),
        ], capture_output=True, text=True)
        self.assertEqual(completed.returncode, 3, completed.stdout + completed.stderr)
        report = json.loads((output / "report.json").read_text())
        self.assertEqual(report["scope"], "foundations")
        self.assertEqual(report["cases"], {})
        self.assertEqual(report["gates"]["foundations"], "blocked")
        self.assertEqual(report["infrastructure_errors"], [])
        self.assertFalse(report["provenance"]["build"]["verified"])

    def test_removing_a_requirement_cannot_shrink_frozen_behavior_coverage(self):
        other = copy.deepcopy(self.bundle["requirements"][TEST_REQUIREMENT_ID])
        other["id"] = "sizing.minimum"
        self.bundle["requirements"][other["id"]] = other
        self.contract["required_requirement_ids"].append(other["id"])
        self.contract["behaviors"][0]["requirement_ids"].append(other["id"])
        self.api().validate_foundations(self.bundle, self.contract)
        self.contract["behaviors"][0]["requirement_ids"].remove(other["id"])
        with self.assertRaisesRegex(ManifestError, "coverage"):
            self.api().validate_foundations(self.bundle, self.contract)

    def test_cli_inventory_reports_scoped_and_global_coverage_separately(self):
        write_json(self.root / "foundations.json", self.contract)
        completed = subprocess.run([
            sys.executable, str(Path(__file__).with_name("run.py")),
            "--manifest-dir", str(self.root), "--foundations", "--inventory",
        ], capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        summary = json.loads(completed.stdout)
        self.assertIn("foundations", summary)
        self.assertEqual(summary["foundations"]["status"], "blocked")
        self.assertEqual(summary["foundations"]["requirement_ids"], [TEST_REQUIREMENT_ID])
        self.assertEqual(summary["requirements_total"], 1)
        self.assertEqual(summary["verified"], 0)

    def test_load_scope_keeps_source_hashes_and_rejects_duplicate_json_keys(self):
        write_json(self.root / "foundations.json", self.contract)
        loaded = self.api().load_foundations(load_bundle(self.root))
        self.assertEqual(loaded["source_reviews"][0]["sha256"], "a" * 64)
        path = self.root / "foundations.json"
        path.write_text('{"schema_version":1,"schema_version":1}')
        with self.assertRaisesRegex(ManifestError, "duplicate"):
            self.api().load_foundations(self.bundle)


if __name__ == "__main__":
    unittest.main()
