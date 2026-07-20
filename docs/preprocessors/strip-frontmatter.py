#!/usr/bin/env python3
"""mdBook preprocessor that strips leading YAML frontmatter from chapters.

Repository convention requires YAML frontmatter (title, last_updated, tags)
in every `.md` file under `docs/`, but mdBook 0.4.x renders that block as
visible page content. This preprocessor removes a leading `---` ... `---`
block from every chapter before rendering, so source files keep their
metadata while the published site stays clean.

Protocol: https://rust-lang.github.io/mdBook/for_developers/preprocessors.html
"""

import json
import re
import sys

# A frontmatter block at the very start of the file: an opening `---` line,
# any lines, a closing `---` line, plus any blank lines that follow it.
FRONTMATTER = re.compile(r"\A---[ \t]*\n.*?\n---[ \t]*\n\s*", re.DOTALL)


def strip_frontmatter(content):
    return FRONTMATTER.sub("", content, count=1)


def walk(items):
    for item in items:
        if not isinstance(item, dict):
            continue  # "Separator"
        chapter = item.get("Chapter")
        if chapter is None:
            continue  # e.g. PartTitle
        chapter["content"] = strip_frontmatter(chapter["content"])
        walk(chapter["sub_items"])


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "supports":
        sys.exit(0)  # frontmatter must be stripped for every renderer

    _context, book = json.load(sys.stdin)
    walk(book["sections"])
    json.dump(book, sys.stdout)


if __name__ == "__main__":
    main()
