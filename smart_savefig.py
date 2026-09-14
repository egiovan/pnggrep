"""
smart_savefig.py - Reproducible Scientific Visualization Module

Drop-in replacement for `matplotlib.pyplot.savefig`.
Automatically embeds caller script source code, file system context,
runtime timestamp, Git commit SHA, and Git commit message into image metadata
for PNG, SVG, and PDF formats.

Alerts the user on stderr if saving from a dirty or untracked repository state.
"""

from datetime import datetime
import inspect
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
import matplotlib
import matplotlib.pyplot as plt
from PIL import Image

# Maximum memory allocation guard against oversized files during inspection
MAX_FILE_READ_BYTES = 32 * 1024 * 1024  # 32 MB

# Standard Dublin Core metadata elements accepted by Matplotlib's SVG backend
PERMITTED_SVG_KEYS = {
    "creator", "date", "format", "identifier", "language", "relation",
    "source", "subject", "title", "type", "coverage", "description",
    "rights",
}

CANONICAL_SVG_KEYS = {
    k.lower(): k for k in [
        "Creator", "Date", "Format", "Identifier", "Language", "Relation",
        "Source", "Subject", "Title", "Type", "Coverage", "Description",
        "Rights",
    ]
}

# Standard PDF /Info dictionary keys accepted by Matplotlib's PDF backend
PERMITTED_PDF_KEYS = {
    "title", "author", "subject", "keywords", "creator", "producer",
    "creationdate", "moddate", "trapped",
}

CANONICAL_PDF_KEYS = {
    k.lower(): k for k in [
        "Title", "Author", "Subject", "Keywords", "Creator", "Producer",
        "CreationDate", "ModDate", "Trapped",
    ]
}


def _get_git_info(directory: str, script_name: str = None) -> dict:
    """
    Query Git to determine the commit hash, commit message, and repository status.

    Parameters:
        directory: The working directory of the caller script.
        script_name: The filename of the caller script to verify tracking.

    Returns:
        dict containing:
            - 'git_commit': Full 40-character commit SHA of HEAD.
            - 'git_commit_message': Subject line of the latest commit.
            - 'git_dirty': Boolean, True if tracked files are modified or
                           if the caller script itself is untracked.
            - 'git_script_tracked': Boolean, True if script is tracked.
            - 'git_dirty_reason': Explanation of why the state is dirty.
    """
    try:
        # 1. Retrieve the latest commit hash
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=directory,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()

        # 2. Retrieve the latest commit subject line
        commit_msg = subprocess.check_output(
            ["git", "log", "-1", "--format=%s"],
            cwd=directory,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()

        # 3. Check for uncommitted modifications to tracked files (-uno)
        tracked_changes = subprocess.check_output(
            ["git", "status", "--porcelain", "-uno"],
            cwd=directory,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()

        # 4. Verify that the calling Python script itself is tracked in Git
        is_script_tracked = False
        if script_name:
            res = subprocess.run(
                ["git", "ls-files", "--error-unmatch", script_name],
                cwd=directory,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            is_script_tracked = (res.returncode == 0)

        dirty = bool(tracked_changes) or not is_script_tracked

        dirty_reasons = []
        if tracked_changes:
            dirty_reasons.append("uncommitted changes in tracked files")
        if not is_script_tracked:
            dirty_reasons.append(f"script '{script_name}' is not tracked in Git")

        return {
            "git_commit": commit,
            "git_commit_message": commit_msg,
            "git_dirty": dirty,
            "git_script_tracked": is_script_tracked,
            "git_dirty_reason": "; ".join(dirty_reasons) if dirty else "clean",
        }
    except Exception:
        return {}


def _get_caller_context() -> dict:
    """Inspect the Python call stack to identify the caller script and location."""
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
    Save a Matplotlib figure embedding the caller's source code and Git metadata.
    Prints an alert to stderr if the repository contains uncommitted changes.

    Parameters:
        fname: Output filename or path (.png, .svg, or .pdf).
        fig: matplotlib.figure.Figure instance (defaults to plt.gcf()).
        **kwargs: Extra keyword arguments forwarded to matplotlib.pyplot.savefig.
    """
    if fig is None:
        fig = plt.gcf()

    ext = Path(fname).suffix.lower()
    ctx = _get_caller_context()
    user_metadata = dict(kwargs.pop("metadata", {}) or {})

    # Alert the user if the repository state is dirty
    if ctx.get("git_dirty"):
        reason = ctx.get("git_dirty_reason", "uncommitted changes detected")
        sys.stderr.write(
            f"\n\x1b[33m[smart_savefig ALERT]\x1b[0m Saving '\x1b[1m{fname}\x1b[0m' with a dirty Git state:\n"
            f"  Reason: \x1b[31m{reason}\x1b[0m\n"
            f"  Commit: {ctx.get('git_commit', 'unknown')[:8]} (\"{ctx.get('git_commit_message', '')}\")\n"
            f"  \x1b[90m-> Consider committing your changes for strict reproducibility.\x1b[0m\n\n"
        )

    # -------------------------------------------------------------------------
    # 1. PNG Backend
    # -------------------------------------------------------------------------
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
            meta["GitCommitMessage"] = ctx.get("git_commit_message", "")
            meta["GitDirty"] = str(ctx["git_dirty"])
            meta["GitScriptTracked"] = str(ctx.get("git_script_tracked", False))

        meta.update({str(k): str(v) for k, v in user_metadata.items()})
        fig.savefig(fname, metadata=meta, **kwargs)

    # -------------------------------------------------------------------------
    # 2. SVG Backend (Dublin Core validation)
    # -------------------------------------------------------------------------
    elif ext == ".svg":
        svg_meta = {
            "Title": ctx["filename"],
            "Date": ctx["timestamp"],
            "Type": "Scientific Visualization",
        }

        custom_json_metadata = {}
        for key, val in user_metadata.items():
            key_lower = str(key).strip().lower()
            if key_lower in PERMITTED_SVG_KEYS and key_lower != "description":
                canonical_key = CANONICAL_SVG_KEYS[key_lower]
                svg_meta[canonical_key] = str(val)
            else:
                custom_json_metadata[key] = val

        ctx.update(custom_json_metadata)
        payload = json.dumps(ctx, indent=2, ensure_ascii=False, default=str)
        svg_meta["Description"] = payload

        fig.savefig(fname, metadata=svg_meta, **kwargs)

    # -------------------------------------------------------------------------
    # 3. PDF Backend (PDF /Info dictionary validation)
    # -------------------------------------------------------------------------
    elif ext == ".pdf":
        native_pdf_meta = {
            "Title": str(user_metadata.get("Title", ctx["filename"])),
            "Author": str(user_metadata.get("Author", ctx.get("caller_function", ""))),
            "Subject": str(user_metadata.get("Subject", f"smart_savefig: {ctx['filename']}")),
            "Creator": f"matplotlib {matplotlib.__version__} (smart_savefig)",
        }

        custom_json_metadata = {}
        for key, val in user_metadata.items():
            key_lower = str(key).strip().lower()
            if key_lower in PERMITTED_PDF_KEYS and key_lower != "keywords":
                canonical_key = CANONICAL_PDF_KEYS[key_lower]
                native_pdf_meta[canonical_key] = str(val)
            else:
                custom_json_metadata[key] = val

        ctx.update(custom_json_metadata)
        payload = json.dumps(ctx, indent=2, ensure_ascii=False, default=str)
        native_pdf_meta["Keywords"] = payload

        fig.savefig(fname, metadata=native_pdf_meta, **kwargs)

    # -------------------------------------------------------------------------
    # 4. Other Backends
    # -------------------------------------------------------------------------
    else:
        fig.savefig(fname, **kwargs)


def read_metadata(image_path: str) -> dict:
    """Extract metadata and source code from a PNG, SVG, or PDF figure."""
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
        for elem in root.findall(".//dc:*", ns):
            tag = elem.tag.split("}")[-1].capitalize()
            if elem.text:
                metadata[tag] = elem.text.strip()

        desc_elem = root.find(".//dc:description", ns)
        if desc_elem is not None and desc_elem.text:
            try:
                json_data = json.loads(desc_elem.text)
                metadata.update(json_data)
            except json.JSONDecodeError:
                metadata["Description"] = desc_elem.text
        return metadata

    elif ext == ".pdf":
        content = path.read_bytes()
        lower_content = content.lower()
        if b"matplotlib" not in lower_content and b"smart_savefig" not in lower_content:
            return {}

        match = re.search(rb'/Keywords\s*\((.*?)\)(?:\s*/|\s*>>)', content, re.DOTALL)
        if match:
            raw_val = match.group(1)
            unescaped = (
                raw_val.replace(b'\\(', b'(')
                .replace(b'\\)', b')')
                .replace(b'\\\\', b'\\')
                .replace(b'\\n', b'\n')
                .replace(b'\\r', b'\r')
                .replace(b'\\t', b'\t')
            )
            try:
                if unescaped.startswith(b'\xfe\xff'):
                    text = unescaped[2:].decode('utf-16-be', errors='replace')
                else:
                    text = unescaped.decode('utf-8', errors='replace')
                return json.loads(text)
            except Exception:
                pass
        return {}

    return {}


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(
        description="Extract source code and metadata from PNG, SVG, or PDF figures."
    )
    parser.add_argument("image", help="Path to PNG, SVG, or PDF figure file")
    parser.add_argument("-o", "--output", help="Save extracted source code to a file")
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
        print(f"# File:    {meta.get('SourceFile') or meta.get('filename')}")
        print(f"# Dir:     {meta.get('SourceDirectory') or meta.get('directory')}")
        print(f"# Commit:  {meta.get('GitCommit') or meta.get('git_commit')} (Dirty: {meta.get('GitDirty') or meta.get('git_dirty')})")
        if meta.get("GitCommitMessage") or meta.get("git_commit_message"):
            print(f"# Message: {meta.get('GitCommitMessage') or meta.get('git_commit_message')}")
        print("-" * 60)
        print(source)