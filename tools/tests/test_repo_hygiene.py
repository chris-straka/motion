#!/usr/bin/env python3
"""Repository hygiene tests: documentation links, index coverage, and public privacy."""

import pathlib
import re
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
LINK_RE = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


class RepoHygieneTests(unittest.TestCase):
    def test_markdown_relative_links_resolve(self):
        md_files = [
            ROOT / "README.md",
            ROOT / "PLAN.md",
            ROOT / "AGENTS.md",
            *sorted((ROOT / "docs").rglob("*.md")),
        ]
        broken = []
        for md in md_files:
            text = md.read_text(encoding="utf-8")
            for lineno, line in enumerate(text.splitlines(), 1):
                for target in LINK_RE.findall(line):
                    if target.startswith(("http://", "https://", "mailto:", "#")):
                        continue
                    rel = target.split("#", 1)[0]
                    if not (md.parent / rel).resolve().exists():
                        broken.append(f"{md.relative_to(ROOT)}:{lineno} -> {target}")
        self.assertEqual(broken, [], "Broken markdown links:\n" + "\n".join(broken))

    def test_docs_readme_indexes_all_docs(self):
        index_md = ROOT / "docs" / "README.md"
        self.assertTrue(index_md.is_file(), "docs/README.md must exist")
        linked = {
            (index_md.parent / t.split("#", 1)[0]).resolve()
            for t in LINK_RE.findall(index_md.read_text(encoding="utf-8"))
            if not t.startswith(("http://", "https://", "#"))
        }
        all_docs = {
            p.resolve()
            for p in (ROOT / "docs").rglob("*.md")
            if p.resolve() != index_md.resolve()
        }
        missing = sorted(str(p.relative_to(ROOT)) for p in (all_docs - linked))
        self.assertEqual(missing, [], f"Unindexed doc files: {missing}")

    def test_no_personal_home_paths_or_private_refs_in_tracked_files(self):
        # Construct needles dynamically so this test file itself stays clean.
        forbidden = [
            "/" + "Users/c",
            "/" + "home/f",
            "~" + "/Games",
            "~" + "/rfwork",
            "~" + "/rigwork",
            "hlh" + "/bevy",
            "MON" + "STERS.md",
            "char" + "acter-line",
        ]
        res = subprocess.run(
            ["git", "ls-files", "-z"],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if res.returncode == 0:
            files = [ROOT / p for p in res.stdout.decode("utf-8").split("\x00") if p]
        else:
            files = [
                p for p in ROOT.rglob("*")
                if p.is_file() and ".git" not in p.parts and "target" not in p.parts
            ]
        hits = []
        for path in files:
            if not path.is_file():
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for pat in forbidden:
                if pat in text:
                    hits.append(f"{path.relative_to(ROOT)} contains {pat!r}")
        self.assertEqual(hits, [], "Private paths/refs found:\n" + "\n".join(hits))


if __name__ == "__main__":
    unittest.main()
