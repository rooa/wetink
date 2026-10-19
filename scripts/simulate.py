#!/usr/bin/env python3
"""Simulate an agent editing a Markdown file, for watching it in wetink.

    python3 scripts/simulate.py demo.md          # in one pane
    wetink demo.md                               # in another

Each step prints what it changed. Writes are atomic (temp file + rename),
like most agent tools, except the step that tests half-written files.
"""

import argparse
import os
import time

INITIAL = """\
# Migration plan

Status: **drafting**

## Tasks

- [ ] Audit current schema
- [ ] Write migration script
- [ ] Run on staging

## Notes

The old service writes to `orders_v1`. We keep it read-only during the cutover.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Data loss | High | Backups before each step |
| Downtime | Medium | Run at night |
"""

APPENDIX = """
## Appendix: rollout log

Day 1 went as planned. The read replica lagged by about two seconds during the
backfill, which is within budget. No customer-facing errors were reported.

Day 2 switched writes to `orders_v2`. Old readers were left in place for a day
so dashboards could catch up.
"""


def replace(old, new):
    def edit(text):
        assert old in text, f"simulate.py: expected text not found: {old!r}"
        return text.replace(old, new, 1)

    return edit


def append(extra):
    return lambda text: text + extra


STEPS = [
    ("check a box: 'Audit current schema'", replace("- [ ] Audit", "- [x] Audit")),
    ("add a task at the end of the list", replace("- [ ] Run on staging\n", "- [ ] Run on staging\n- [ ] Announce the cutover window\n")),
    ("modify a line: status drafting → in progress", replace("**drafting**", "**in progress**")),
    (
        "insert a paragraph under Notes",
        replace("## Notes\n\n", "## Notes\n\nBackfill runs in batches of 10k rows so replication can keep up.\n\n"),
    ),
    (
        "insert a new section with a code block",
        replace(
            "## Risks",
            "## Script\n\n```python\nfor batch in batches(orders_v1, size=10_000):\n    orders_v2.insert(batch)\n```\n\n## Risks",
        ),
    ),
    ("edit a table cell: Downtime Medium → Low", replace("| Downtime | Medium |", "| Downtime | Low |")),
    ("delete a task: 'Run on staging'", replace("- [ ] Run on staging\n", "")),
    ("add a warning alert", replace("## Risks\n", "## Risks\n\n> [!WARNING]\n> Freeze schema changes until the cutover is done.\n")),
    ("delete a paragraph (the original Notes text)", replace("The old service writes to `orders_v1`. We keep it read-only during the cutover.\n\n", "")),
    ("burst: three quick edits", None),
    ("half-written file (should never show truncated)", None),
    ("append a long section at the end (follow scrolls to it)", append(APPENDIX)),
    ("rewrite a heading entirely (reads as remove + add)", replace("## Notes", "## Background")),
]

BURST = [
    replace("- [ ] Write migration", "- [x] Write migration"),
    replace("- [ ] Announce", "- [x] Announce"),
    replace("**in progress**", "**done**"),
]


def write_atomic(path, text):
    tmp = f"{path}.tmp-{os.getpid()}"
    with open(tmp, "w") as f:
        f.write(text)
    os.replace(tmp, path)


def write_slowly(path, text):
    """Write in place in two halves, like a non-atomic editor or a streamed write."""
    with open(path, "w") as f:
        f.write(text[: len(text) // 2])
        f.flush()
        time.sleep(0.05)
        f.write(text[len(text) // 2 :])


def main():
    p = argparse.ArgumentParser(description="Simulate an agent editing a Markdown file.")
    p.add_argument("file", help="Markdown file to create and edit")
    p.add_argument("-d", "--delay", type=float, default=2.5, help="seconds between steps (default 2.5)")
    p.add_argument("--loop", action="store_true", help="start over after the last step")
    args = p.parse_args()

    while True:
        text = INITIAL
        write_atomic(args.file, text)
        print(f"wrote initial {args.file}")
        for n, (label, edit) in enumerate(STEPS, 1):
            time.sleep(args.delay)
            print(f"[{n:2}/{len(STEPS)}] {label}")
            if label.startswith("burst"):
                for e in BURST:
                    text = e(text)
                    write_atomic(args.file, text)
                    time.sleep(0.15)
            elif label.startswith("half-written"):
                text = replace("| Data loss | High |", "| Data loss | Medium |")(text)
                write_slowly(args.file, text)
            else:
                text = edit(text)
                write_atomic(args.file, text)
        if not args.loop:
            break
        time.sleep(args.delay * 2)
        print("--- starting over (the reset itself shows as many changes)")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
