#!/usr/bin/env python3
"""Build assets/fluent-catalog.json from upstream Microsoft Fluent emoji repos.

Sparse-clones `microsoft/fluentui-emoji` for `metadata.json` content and walks
`microsoft/fluentui-emoji-animated`'s file tree to mark which emoji have an
animated APNG. Emits a single sorted JSON catalog committed to the repo and
bundled into the daemon at compile time via include_str!.

Run via `just rebuild-catalog`.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

STATIC_REPO = "https://github.com/microsoft/fluentui-emoji.git"
ANIMATED_REPO = "https://github.com/microsoft/fluentui-emoji-animated.git"


def run(*args: str, cwd: Path | None = None) -> str:
    res = subprocess.run(
        list(args),
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return res.stdout


def slugify(name: str) -> str:
    """Microsoft's stub convention: lowercase; ' ' and '-' → '_'; drop the rest."""
    s = name.lower()
    s = re.sub(r"[\s\-]+", "_", s)
    s = re.sub(r"[^a-z0-9_]", "", s)
    s = re.sub(r"_+", "_", s).strip("_")
    return s


def collect_paths(tree: list[str], style_segment: str) -> dict[str, str]:
    """Return {emoji_folder: full_path} for `assets/<folder>/<style>/*.png` or
    `assets/<folder>/Default/<style>/*.png` (skin-toned variants).

    Tone-less paths are preferred when both exist; for toned emoji the
    Default tone stands in as the catalog preview.
    """
    out: dict[str, str] = {}
    toned: dict[str, str] = {}
    for p in tree:
        if not p.endswith(".png"):
            continue
        parts = p.split("/")
        if len(parts) == 4 and parts[0] == "assets" and parts[2] == style_segment:
            out.setdefault(parts[1], p)
        elif (
            len(parts) == 5
            and parts[0] == "assets"
            and parts[2] == "Default"
            and parts[3] == style_segment
        ):
            toned.setdefault(parts[1], p)
    for folder, path in toned.items():
        out.setdefault(folder, path)
    return out


def main(repo_root: Path) -> int:
    with tempfile.TemporaryDirectory() as tmp_str:
        tmp = Path(tmp_str)
        static = tmp / "static"
        animated = tmp / "animated"

        print(f"[catalog] sparse-cloning {STATIC_REPO}", file=sys.stderr)
        run(
            "git", "clone", "--depth=1", "--filter=blob:none", "--sparse",
            STATIC_REPO, str(static),
        )
        run(
            "git", "-C", str(static),
            "sparse-checkout", "set", "--no-cone",
            "assets/*/metadata.json",
        )
        static_tree = run(
            "git", "-C", str(static),
            "ls-tree", "-r", "--name-only", "HEAD",
        ).splitlines()

        print(f"[catalog] tree-only clone {ANIMATED_REPO}", file=sys.stderr)
        run(
            "git", "clone", "--depth=1", "--filter=blob:none", "--no-checkout",
            ANIMATED_REPO, str(animated),
        )
        animated_tree = run(
            "git", "-C", str(animated),
            "ls-tree", "-r", "--name-only", "HEAD",
        ).splitlines()

        static_3d = collect_paths(static_tree, "3D")
        animated_paths = collect_paths(animated_tree, "animated")

        catalog: list[dict] = []
        seen_ids: dict[str, str] = {}
        for meta_path in sorted((static / "assets").glob("*/metadata.json")):
            folder = meta_path.parent.name
            try:
                meta = json.loads(meta_path.read_text(encoding="utf-8"))
            except json.JSONDecodeError as e:
                print(f"[catalog] skip {folder}: {e}", file=sys.stderr)
                continue
            id_ = slugify(meta.get("cldr") or folder)
            if id_ in seen_ids:
                # Two different folders mapped to the same id; suffix to disambiguate.
                # We've seen this before with skin-tone variants of the same base name.
                print(
                    f"[catalog] duplicate id '{id_}' from '{folder}' "
                    f"(first: '{seen_ids[id_]}'), suffixing",
                    file=sys.stderr,
                )
                id_ = f"{id_}_{slugify(folder)}"
            seen_ids[id_] = folder

            entry = {
                "id": id_,
                "name": folder,
                "glyph": meta.get("glyph", ""),
                "group": meta.get("group", ""),
                "keywords": meta.get("keywords", []),
                "unicode": meta.get("unicode", ""),
                "has_animated": folder in animated_paths,
                "static_path": static_3d.get(folder, ""),
                "animated_path": animated_paths.get(folder, ""),
            }
            if not entry["static_path"]:
                # No 3D PNG at all → skip (we render previews from the 3D variant).
                print(f"[catalog] skip {folder}: no 3D PNG", file=sys.stderr)
                continue
            catalog.append(entry)

        catalog.sort(key=lambda e: e["id"])
        out = repo_root / "assets" / "fluent-catalog.json"
        out.parent.mkdir(exist_ok=True)
        out.write_text(
            json.dumps(catalog, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )

        animated_count = sum(1 for e in catalog if e["has_animated"])
        print(
            f"[catalog] wrote {len(catalog)} entries "
            f"({animated_count} animated) → {out.relative_to(repo_root)}",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    repo_root = Path(__file__).resolve().parent.parent
    sys.exit(main(repo_root))
