"""Source-addressed foundations coverage, separate from the complete release gate."""
from pathlib import Path

from conformance import BLOCKED, FAIL, PASS, combine, evaluate_requirements
from manifest import ID, SCHEMA_VERSION, ManifestError, _read

MANIFEST_NAME = "foundations.json"
GROUPS = {
    "syntax-acquisition", "cascade-defaulting", "rule-applicability",
    "font-declarations", "values-sizing", "html-foundations",
}
REVIEW_STATES = {"pending", "reviewed"}
IMPORT_STATES = {"required", "outside-scope"}
OBLIGATION_FIELDS = ("default_obligations", "invalid_obligations", "interaction_obligations")


def load_foundations(bundle: dict) -> dict:
    foundations = _read(Path(bundle["root"]) / MANIFEST_NAME)
    validate_foundations(bundle, foundations)
    return foundations


def validate_foundations(bundle: dict, foundations: dict) -> None:
    path = Path(bundle["root"]) / MANIFEST_NAME

    def fail(pointer, reason):
        raise ManifestError(path, pointer, reason)

    def identifiers(value, pointer, *, nonempty=True, stable=True):
        if not isinstance(value, list) or (nonempty and not value):
            fail(pointer, "expected a nonempty ID list" if nonempty else "expected an ID list")
        if any(not isinstance(item, str) or not item or (stable and not ID.fullmatch(item)) for item in value):
            fail(pointer, "expected stable IDs")
        if len(set(value)) != len(value):
            fail(pointer, "duplicate IDs")
        return set(value)

    def section_reference(value, pointer):
        if not isinstance(value, dict):
            fail(pointer, "expected source section reference")
        source_id = value.get("source_id")
        section_id = value.get("section_id")
        if not isinstance(source_id, str) or source_id not in bundle["sources"]:
            fail(pointer, "unknown source")
        if not any(section["id"] == section_id for section in bundle["sections"][source_id]):
            fail(pointer, "unknown source section")
        return source_id, section_id

    def proof(value, pointer):
        source_id, _ = section_reference(value, pointer)
        if value.get("sha256") != bundle["sources"][source_id]["sha256"]:
            fail(pointer, "source proof differs from content hash")

    if not isinstance(foundations, dict) or type(foundations.get("schema_version")) is not int or foundations["schema_version"] != SCHEMA_VERSION:
        fail("/schema_version", "unsupported or absent schema version")
    if not isinstance(foundations.get("id"), str) or not ID.fullmatch(foundations["id"]):
        fail("/id", "scope needs a stable ID")
    groups = identifiers(foundations.get("required_group_ids"), "/required_group_ids")
    if groups - GROUPS:
        fail("/required_group_ids", "unknown foundations group")
    required = identifiers(foundations.get("required_behavior_ids"), "/required_behavior_ids")
    required_requirements = identifiers(foundations.get("required_requirement_ids"), "/required_requirement_ids")
    behaviors = foundations.get("behaviors")
    if not isinstance(behaviors, list):
        fail("/behaviors", "expected behavior coverage list")
    seen = set()
    covered_groups = set()
    covered_requirements = set()
    for index, behavior in enumerate(behaviors):
        pointer = f"/behaviors/{index}"
        if not isinstance(behavior, dict):
            fail(pointer, "expected behavior object")
        identity = behavior.get("id")
        if not isinstance(identity, str) or not ID.fullmatch(identity) or identity in seen:
            fail(pointer + "/id", "invalid or duplicate behavior ID")
        seen.add(identity)
        if behavior.get("handoff") is not None:
            fail(pointer + "/handoff", "included foundations behavior cannot become a later handoff")
        group = behavior.get("group")
        if not isinstance(group, str) or group not in groups:
            fail(pointer + "/group", "behavior is outside the frozen group coverage")
        covered_groups.add(group)
        source_id = behavior.get("source_id")
        if not isinstance(source_id, str) or source_id not in bundle["sources"]:
            fail(pointer + "/source_id", "unknown source")
        sections = identifiers(behavior.get("section_ids"), pointer + "/section_ids", stable=False)
        known_sections = {section["id"] for section in bundle["sections"][source_id]}
        if sections - known_sections:
            fail(pointer + "/section_ids", "unknown source section")
        requirements = identifiers(behavior.get("requirement_ids"), pointer + "/requirement_ids")
        for requirement_id in requirements:
            requirement = bundle["requirements"].get(requirement_id)
            if requirement is None or requirement["applicability"]["status"] != "applicable":
                fail(pointer + "/requirement_ids", "included behavior needs applicable requirement IDs")
            if requirement["source_id"] == source_id and not sections.intersection(requirement["sections"]):
                fail(pointer + "/requirement_ids", "requirement does not cover this behavior section")
        covered_requirements.update(requirements)
        if type(behavior.get("owner_task")) is not int or behavior["owner_task"] <= 0:
            fail(pointer + "/owner_task", "behavior needs an implementation task")
        files = behavior.get("implementation_files")
        if not isinstance(files, list) or not files or any(
            not isinstance(file, str) or not file or Path(file).is_absolute() or ".." in Path(file).parts
            for file in files
        ):
            fail(pointer + "/implementation_files", "expected repository-relative implementation paths")
        for field in OBLIGATION_FIELDS:
            obligations = behavior.get(field)
            if not isinstance(obligations, list) or not obligations or any(not isinstance(item, str) or not item.strip() for item in obligations):
                fail(pointer + "/" + field, "expected explicit behavior obligations")
        cases = identifiers(behavior.get("case_ids"), pointer + "/case_ids", nonempty=False)
        if bundle.get("scope", "full") in ("cases", "full"):
            for case_id in cases:
                case = bundle["cases"].get(case_id)
                if case is None or not requirements.intersection(case["requirement_ids"]):
                    fail(pointer + "/case_ids", "case evidence does not cover an included requirement")
    if covered_requirements != required_requirements:
        fail("/behaviors", "missing or extra frozen requirement coverage")
    if seen != required or covered_groups != groups:
        fail("/behaviors", "missing or extra frozen behavior/group coverage")

    reviews = foundations.get("source_reviews")
    if not isinstance(reviews, list):
        fail("/source_reviews", "expected source review list")
    reviewed_sources = set()
    for index, review in enumerate(reviews):
        pointer = f"/source_reviews/{index}"
        if not isinstance(review, dict):
            fail(pointer, "expected source review object")
        source_id = review.get("source_id")
        if not isinstance(source_id, str) or source_id not in bundle["sources"] or source_id in reviewed_sources:
            fail(pointer + "/source_id", "unknown or duplicate source")
        reviewed_sources.add(source_id)
        if review.get("sha256") != bundle["sources"][source_id]["sha256"]:
            fail(pointer + "/sha256", "review differs from pinned source content")
        sections = identifiers(review.get("section_ids"), pointer + "/section_ids", stable=False)
        if sections - {section["id"] for section in bundle["sections"][source_id]}:
            fail(pointer + "/section_ids", "unknown reviewed source section")
        if not isinstance(review.get("review_state"), str) or review["review_state"] not in REVIEW_STATES:
            fail(pointer + "/review_state", "source must be pending or reviewed")
        imports = review.get("import_dispositions")
        overlaps = review.get("overlap_dispositions")
        if not isinstance(imports, list) or not isinstance(overlaps, list):
            fail(pointer, "expected import and overlap review lists")
        urls = set()
        for position, disposition in enumerate(imports):
            item_pointer = pointer + f"/import_dispositions/{position}"
            if not isinstance(disposition, dict) or not isinstance(disposition.get("url"), str) or not disposition["url"] or disposition["url"] in urls:
                fail(item_pointer, "import needs a unique URL")
            urls.add(disposition["url"])
            if not isinstance(disposition.get("status"), str) or disposition["status"] not in IMPORT_STATES or not disposition.get("reason"):
                fail(item_pointer, "import needs explicit scoped applicability and rationale")
            proof(disposition.get("basis"), item_pointer + "/basis")
            target = disposition.get("target_source_id")
            if target is not None and (not isinstance(target, str) or target not in bundle["sources"]):
                fail(item_pointer + "/target_source_id", "required import target must be pinned")
        other_sources = set()
        for position, disposition in enumerate(overlaps):
            item_pointer = pointer + f"/overlap_dispositions/{position}"
            if not isinstance(disposition, dict):
                fail(item_pointer, "expected overlap review")
            other = disposition.get("other_source_id")
            if not isinstance(other, str) or other not in bundle["sources"] or other in other_sources:
                fail(item_pointer, "overlap needs a unique pinned other source")
            other_sources.add(other)
            if disposition.get("status") != "reviewed" or not disposition.get("reason"):
                fail(item_pointer, "overlap needs reviewed controlling sections and rationale")
            proof(disposition.get("basis"), item_pointer + "/basis")
            controllers = disposition.get("controlling")
            if not isinstance(controllers, list) or not controllers:
                fail(item_pointer + "/controlling", "overlap needs controlling sections")
            for controller in controllers:
                section_reference(controller, item_pointer + "/controlling")

    handoffs = foundations.get("handoffs", [])
    if not isinstance(handoffs, list):
        fail("/handoffs", "expected explicit later-workstream handoffs")
    for index, handoff in enumerate(handoffs):
        pointer = f"/handoffs/{index}"
        if not isinstance(handoff, dict) or not handoff.get("workstream") or not handoff.get("reason"):
            fail(pointer, "handoff needs a workstream and rationale")
        requirements = identifiers(handoff.get("requirement_ids"), pointer + "/requirement_ids")
        if requirements - bundle["requirements"].keys() or requirements.intersection(covered_requirements):
            fail(pointer, "handoff cannot remove an included or unknown requirement")


def _requirement_closure(bundle, identities):
    closure = set(identities)
    pending = list(identities)
    while pending:
        identity = pending.pop()
        for dependency in bundle["requirements"][identity]["dependencies"]:
            if dependency not in closure:
                closure.add(dependency)
                pending.append(dependency)
    return closure


def evaluate_foundations(bundle: dict, foundations: dict, cases: dict) -> dict:
    validate_foundations(bundle, foundations)
    requirements = evaluate_requirements(bundle, cases)
    included = _requirement_closure(bundle, {
        identity for behavior in foundations["behaviors"] for identity in behavior["requirement_ids"]
    })
    reviews = {review["source_id"]: review for review in foundations["source_reviews"]}
    unresolved_imports = []
    unresolved_overlaps = []
    unreviewed_sections = set()
    required_sections = {
        (bundle["requirements"][identity]["source_id"], section)
        for identity in included for section in bundle["requirements"][identity]["sections"]
    }
    required_sections.update(
        (behavior["source_id"], section)
        for behavior in foundations["behaviors"] for section in behavior["section_ids"]
    )
    for source_id, section_id in required_sections:
        review = reviews.get(source_id)
        if review is None or review["review_state"] != "reviewed" or section_id not in review["section_ids"]:
            unreviewed_sections.add((source_id, section_id))
    required_sources = {source for source, _ in required_sections}
    pending = list(required_sources)
    visited = set()
    while pending:
        source_id = pending.pop()
        if source_id in visited:
            continue
        visited.add(source_id)
        source = bundle["sources"][source_id]
        review = reviews.get(source_id, {})
        dispositions = {item["url"]: item for item in review.get("import_dispositions", [])}
        for reference in source.get("reference_dispositions", []):
            disposition = dispositions.get(reference["url"])
            if disposition and disposition["status"] == "outside-scope":
                continue
            if disposition is None or reference.get("status") != "pinned":
                unresolved_imports.append({"source_id": source_id, **reference})
                continue
            target = disposition.get("target_source_id") or reference.get("source_id")
            if target not in reviews or reviews[target]["review_state"] != "reviewed":
                unresolved_imports.append({"source_id": source_id, **reference})
            elif target not in visited:
                pending.append(target)
        overlap_reviews = {item["other_source_id"]: item for item in review.get("overlap_dispositions", [])}
        for overlap in source.get("overlap_dispositions", []):
            disposition = overlap_reviews.get(overlap["other_source_id"])
            if disposition is None or any(
                controller["source_id"] not in reviews
                or reviews[controller["source_id"]]["review_state"] != "reviewed"
                or controller["section_id"] not in reviews[controller["source_id"]]["section_ids"]
                for controller in disposition["controlling"]
            ):
                unresolved_overlaps.append({"source_id": source_id, **overlap})

    by_id = {record.get("case_id", identity): record for identity, record in cases.items()}
    behavior_results = {}
    for behavior in foundations["behaviors"]:
        closure = _requirement_closure(bundle, behavior["requirement_ids"])
        statuses = [requirements[identity]["status"] for identity in closure]
        for identity in closure:
            for verification in bundle["requirements"][identity].get("verification", []):
                record = by_id.get(verification["case_id"], {})
                if record.get("warnings_unresolved") or record.get("input_lock_valid") is False or record.get("asset_lock_valid") is False:
                    statuses.append(BLOCKED)
        behavior_results[behavior["id"]] = {
            "status": combine(statuses),
            "requirement_ids": sorted(closure),
            "owner_task": behavior["owner_task"],
        }
    statuses = [result["status"] for result in behavior_results.values()]
    closure_complete = not unresolved_imports and not unresolved_overlaps and not unreviewed_sections
    if not closure_complete:
        statuses.append(BLOCKED)
    status = combine(statuses)
    return {
        "id": foundations["id"],
        "status": PASS if status == PASS else FAIL if status == FAIL else BLOCKED,
        "behaviors": behavior_results,
        "requirement_ids": sorted(included),
        "unverified_requirement_ids": sorted(identity for identity in included if requirements[identity]["status"] != PASS),
        "source_closure_complete": closure_complete,
        "unresolved_imports": unresolved_imports,
        "unresolved_overlaps": unresolved_overlaps,
        "unreviewed_sections": [{"source_id": source, "section_id": section} for source, section in sorted(unreviewed_sections)],
    }


def required_case_ids(bundle: dict, foundations: dict) -> list[str]:
    """Select all mandatory scoped evidence, including requirement dependencies."""
    included = _requirement_closure(bundle, {
        identity for behavior in foundations["behaviors"] for identity in behavior["requirement_ids"]
    })
    cases = {identity for behavior in foundations["behaviors"] for identity in behavior["case_ids"]}
    cases.update(reference["case_id"] for identity in included
                 for reference in bundle["requirements"][identity].get("verification", []))
    return sorted(cases)
