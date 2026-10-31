"""Render the benchmark corpus with the release build and print time, pages and peak RSS.

Each document runs in its own process, so peak RSS is that document's; the runs inside a
process share whatever the renderer keeps between renders, as an embedding service would.
"""
import argparse
import json
import platform
import re
import subprocess
from pathlib import Path

import corpus

ROOT = corpus.ROOT
RUNS = {"report": 5, "wide-table": 3, "event-plan": 5, "prose": 5, "invoice": 100}
TIME = "/usr/bin/time"
# BSD time reports bytes; GNU time reports kilobytes.
RSS_PATTERNS = {
    "Darwin": ("-l", re.compile(r"(\d+)\s+maximum resident set size"), 1),
    "Linux": ("-v", re.compile(r"Maximum resident set size \(kbytes\): (\d+)"), 1024),
}
BYTES_PER_MIB = 1 << 20


def build():
    """The release benchmark executable, built with `cargo bench --no-run`."""
    result = subprocess.run(
        ["cargo", "bench", "--locked", "--bench", "render", "--no-run", "--message-format=json"],
        cwd=ROOT, check=True, capture_output=True, text=True)
    for line in result.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message.get("executable") \
                and message["target"]["name"] == "render":
            return message["executable"]
    raise RuntimeError("cargo built no render benchmark executable")


def measure(executable, path, runs):
    command = [executable, "--runs", str(runs), str(path)]
    rss = RSS_PATTERNS.get(platform.system())
    if rss:
        command = [TIME, rss[0], *command]
    result = subprocess.run(command, check=True, capture_output=True, text=True)
    record = json.loads(result.stdout.splitlines()[-1])
    match = rss and rss[1].search(result.stderr)
    record["peak_rss_mib"] = int(match.group(1)) * rss[2] / BYTES_PER_MIB if match else None
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("documents", nargs="*",
                        help=f"documents to run (default: all of {', '.join(corpus.DOCUMENTS)})")
    parser.add_argument("--runs", type=int, help="runs per document (default: per document)")
    parser.add_argument("--output", type=Path, default=corpus.DEFAULT_OUTPUT,
                        help="directory for the generated corpus")
    parser.add_argument("--json", type=Path, help="also write the records to this file")
    args = parser.parse_args()
    unknown = set(args.documents) - set(corpus.DOCUMENTS)
    if unknown:
        parser.error(f"unknown documents: {', '.join(sorted(unknown))}")
    paths = corpus.write_corpus(args.output)
    executable = build()
    records = []
    print(f"{'document':<12} {'pages':>5} {'runs':>4} {'median ms':>10} {'first ms':>9} "
          f"{'peak RSS MiB':>12}  content lost")
    for name in args.documents or corpus.DOCUMENTS:
        record = measure(executable, paths[name], args.runs or RUNS[name])
        records.append(record)
        rss = "n/a" if record["peak_rss_mib"] is None else f"{record['peak_rss_mib']:.0f}"
        print(f"{name:<12} {record['pages']:>5} {record['runs']:>4} {record['median_ms']:>10.1f} "
              f"{record['first_ms']:>9.1f} {rss:>12}  {'; '.join(record['content_lost']) or '-'}",
              flush=True)
    if args.json:
        args.json.write_text(json.dumps(records, indent=2) + "\n")


if __name__ == "__main__":
    main()
