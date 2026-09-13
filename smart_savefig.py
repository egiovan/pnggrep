"""
smart_savefig.py - Save matplotlib figures embedding caller source code,
file paths, execution context, and Git metadata into image metadata.
"""

from datetime import datetime
import inspect
import json
import os
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET
import matplotlib.pyplot as plt
from PIL import Image

MAX_FILE_READ_BYTES = 32 * 1024 * 1024  # 32 MB safety limit

# Standard Dublin Core metadata elements accepted by Matplotlib's SVG backend
PERMITTED_SVG_METADATA_KEYS = {
    "creator", "date", "format", "identifier", "language", "relation",
    "source", "subject", "title", "type", "coverage", "description",
    "rights"
}

CANONICAL_SVG_KEYS = {
    k.lower(): k for k in [
        "Creator", "Date", "Format", "Identifier", "Language", "Relation",
        "Source", "Subject", "Title", "Type", "Coverage", "Description",
        "Rights"
    ]
}


def _get_git_info(directory: str, script_name: str = None) -> dict:
    """
    Retrieve Git commit and repository state:
    - dirty is True if tracked files are modified or if the script is untracked.
    - ignores unrelated untracked files in the working directory.
    """
    try:
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=directory,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()

        # Check only tracked files (-uno = --untracked-files=no)
        tracked_changes = subprocess.check_output(
            ["git", "status", "--porcelain", "-uno"],
            cwd=directory,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()

        # Verify that the calling script is tracked under version control
        is_script_tracked = False
        if script_name:
            res = subprocess.run(
                ["git", "ls-files", "--error-unmatch", script_name],
                cwd=directory,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            is_script_tracked = res.returncode == 0

        dirty = bool(tracked_changes) or not is_script_tracked

        return {
            "git_commit": commit,
            "git_dirty": dirty,
            "git_script_tracked": is_script_tracked,
        }
    except Exception:
        return {}


def _get_caller_context() -> dict:
    """Walk up the call stack to identify the calling module."""
    frame = inspect.currentframe()
    this_file = os.path.abspath(__file__)

    caller_frame = frame.f_back if frame else None
    while caller_frame:
        filename = caller_frame.f_code.co_filename
        if os.path.abspath(filename) != this_file:
            break
        caller_frame = caller_frame.f_back

    if not caller_frame:
        return {
            "directory": os.getcwd(),
            "filename": "<unknown>",
            "filepath": "<unknown>",
            "source_code": "",
            "caller_function": "<unknown>",
            "caller_line": 0,
            "timestamp": datetime.now().isoformat(),
        }

    caller_path = Path(caller_frame.f_code.co_filename).resolve()
    caller_dir = str(caller_path.parent)
    caller_name = caller_path.name

    if caller_path.is_file():
        try:
            source_code = caller_path.read_text(encoding="utf-8")
        except Exception as e:
            source_code = f"<Error reading source file: {e}>"
    else:
        source_code = f"<Not executed from a disk file: {caller_path}>"

    info = {
        "directory": caller_dir,
        "filename": caller_name,
        "filepath": str(caller_path),
        "source_code": source_code,
        "caller_function": caller_frame.f_code.co_name,
        "caller_line": caller_frame.f_lineno,
        "timestamp": datetime.now().isoformat(),
    }

    script_name = caller_name if caller_path.is_file() else None
    info.update(_get_git_info(caller_dir, script_name))
    return info


def savefig(fname, fig=None, **kwargs):
    """
    Save matplotlib figure with embedded directory, filename, and source code.
    Any custom metadata keys not permitted by standard SVG specifications are
    automatically embedded into the JSON payload within <dc:description>.

    Parameters:
        fname: Output filename or path (.png or .svg).
        fig: matplotlib.figure.Figure instance (defaults to plt.gcf()).
        **kwargs: Extra arguments passed to matplotlib.pyplot.savefig.
    """
    if fig is None:
        fig = plt.gcf()

    ext = Path(fname).suffix.lower()
    ctx = _get_caller_context()
    user_metadata = dict(kwargs.pop("metadata", {}) or {})

    if ext == ".png":
        meta = {
            "SourceDirectory": ctx["directory"],
            "SourceFile": ctx["filename"],
            "SourcePath": ctx["filepath"],
            "CallerFunction": ctx["caller_function"],
            "Timestamp": ctx["timestamp"],
            "SourceCode": ctx["source_code"],
            "Comment": ctx["source_code"],
            "Description": f"Generated by {ctx['filename']} ({ctx['caller_function']})",
        }
        if "git_commit" in ctx:
            meta["GitCommit"] = ctx["git_commit"]
            meta["GitDirty"] = str(ctx["git_dirty"])
            meta["GitScriptTracked"] = str(ctx.get("git_script_tracked", False))

        # PNG tEXt/iTXt chunks accept arbitrary keys
        meta.update({str(k): str(v) for k, v in user_metadata.items()})
        fig.savefig(fname, metadata=meta, **kwargs)

    elif ext == ".svg":
        svg_meta = {
            "Title": ctx["filename"],
            "Date": ctx["timestamp"],
            "Type": "Scientific Visualization",
        }

        # Separate permitted Dublin Core keys from custom entries
        custom_json_metadata = {}
        for key, val in user_metadata.items():
            key_lower = str(key).strip().lower()
            if key_lower in PERMITTED_SVG_METADATA_KEYS and key_lower != "description":
                canonical_key = CANONICAL_SVG_KEYS[key_lower]
                svg_meta[canonical_key] = str(val)
            else:
                # Custom keys and user description go into the JSON payload
                custom_json_metadata[key] = val

        ctx.update(custom_json_metadata)

        payload = json.dumps(ctx, indent=2, ensure_ascii=False, default=str)
        svg_meta["Description"] = payload

        fig.savefig(fname, metadata=svg_meta, **kwargs)

    else:
        fig.savefig(fname, **kwargs)


def read_metadata(image_path: str) -> dict:
    """Extract metadata and source code from a PNG or SVG file."""
    path = Path(image_path)
    if not path.is_file():
        raise FileNotFoundError(f"File not found: {image_path}")

    if path.stat().st_size > MAX_FILE_READ_BYTES:
        raise ValueError(f"File too large for inspection (> 32MB): {image_path}")

    ext = path.suffix.lower()

    if ext == ".png":
        with Image.open(path) as img:
            return dict(img.info)

    elif ext == ".svg":
        parser = ET.XMLParser()
        tree = ET.parse(path, parser=parser)
        root = tree.getroot()
        ns = {"dc": "http://purl.org/dc/elements/1.1/"}

        metadata = {}
        # Collect top-level Dublin Core fields
        for elem in root.findall(".//dc:*", ns):
            tag = elem.tag.split("}")[-1].capitalize()
            if elem.text:
                metadata[tag] = elem.text.strip()

        # Unpack JSON payload if present
        desc_elem = root.find(".//dc:description", ns)
        if desc_elem is not None and desc_elem.text:
            try:
                json_data = json.loads(desc_elem.text)
                metadata.update(json_data)
            except json.JSONDecodeError:
                metadata["Description"] = desc_elem.text
        return metadata

    return {}


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(
        description="Extract source code and metadata from PNG/SVG figures."
    )
    parser.add_argument("image", help="Path to PNG or SVG image")
    parser.add_argument(
        "-o", "--output", help="Save extracted source code into a file"
    )
    args = parser.parse_args()

    meta = read_metadata(args.image)
    source = meta.get("SourceCode") or meta.get("source_code") or meta.get("Comment")

    if not source:
        print("No source code found in metadata.", file=sys.stderr)
        sys.exit(1)

    if args.output:
        Path(args.output).write_text(source, encoding="utf-8")
        print(f"Source code successfully extracted to: {args.output}")
    else:
        print(f"# File: {meta.get('SourceFile') or meta.get('filename')}")
        print(f"# Dir:  {meta.get('SourceDirectory') or meta.get('directory')}")
        print(f"# Git:  {meta.get('GitCommit') or meta.get('git_commit')} (Dirty: {meta.get('GitDirty') or meta.get('git_dirty')})")
        print("-" * 60)
        print(source)