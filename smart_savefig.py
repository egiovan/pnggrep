"""
smart_savefig.py - Reproducible Scientific Visualization Module

Drop-in replacement for `matplotlib.pyplot.savefig`.
Automatically embeds caller script source code, file system context,
runtime timestamp, and Git revision status into image metadata
for PNG, SVG, and PDF formats.

Custom user-defined metadata keys are automatically routed to native
fields when permitted by the underlying file format, or packed into
a JSON payload when restricted by standard format schemas.
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
    Query Git to determine the current repository revision and cleanliness.

    Parameters:
        directory: The working directory of the caller script.
        script_name: The filename of the caller script to check for tracking.

    Returns:
        dict containing:
            - 'git_commit': Full 40-character SHA1 hash of HEAD.
            - 'git_dirty': Boolean, True if tracked files are modified or
                           if the caller script itself is untracked.
            - 'git_script_tracked': Boolean, True if script is committed/tracked.
    """
    try:
        # 1. Retrieve the latest commit hash from HEAD
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=directory,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()

        # 2. Check for modifications to tracked files only.
        # The '-uno' (--untracked-files=no) flag prevents scratch or unignored
        # files in the directory from falsely flagging the repository as dirty.
        tracked_changes = subprocess.check_output(
            ["git", "status", "--porcelain", "-uno"],
            cwd=directory,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()

        # 3. Verify that the calling Python script is under version control
        is_script_tracked = False
        if script_name:
            res = subprocess.run(
                ["git", "ls-files", "--error-unmatch", script_name],
                cwd=directory,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            is_script_tracked = (res.returncode == 0)

        # The state is considered dirty if tracked files have uncommitted changes
        # OR if the caller script itself has never been staged/committed.
        dirty = bool(tracked_changes) or not is_script_tracked

        return {
            "git_commit": commit,
            "git_dirty": dirty,
            "git_script_tracked": is_script_tracked,
        }
    except Exception:
        # Return empty dictionary if not in a Git repository or Git is not installed
        return {}


def _get_caller_context() -> dict:
    """
    Inspect the Python call stack to determine the original caller module,
    extracting its file path, directory, calling function, and raw source code.
    """
    frame = inspect.currentframe()
    this_file = os.path.abspath(__file__)

    # Walk up the call stack until reaching a frame outside this file
    caller_frame = frame.f_back if frame else None
    while caller_frame:
        filename = caller_frame.f_code.co_filename
        if os.path.abspath(filename) != this_file:
            break
        caller_frame = caller_frame.f_back

    # Fallback if executed in an interactive environment without a file stack
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

    # Read the full source code directly from the disk file
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

    # Query Git metadata using the caller's directory and filename
    script_name = caller_name if caller_path.is_file() else None
    info.update(_get_git_info(caller_dir, script_name))
    return info


def savefig(fname, fig=None, **kwargs):
    """
    Save a Matplotlib figure embedding the caller's source code and Git metadata.

    Supports PNG, SVG, and PDF formats:
      - PNG: Metadata stored directly in tEXt/iTXt chunks.
      - SVG: Dublin Core keys mapped natively; custom keys packed in Description JSON.
      - PDF: Standard keys mapped to /Info dict; custom keys packed in Keywords JSON.

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
            meta["GitDirty"] = str(ctx["git_dirty"])
            meta["GitScriptTracked"] = str(ctx.get("git_script_tracked", False))

        # PNG tEXt/iTXt chunks accept arbitrary key strings
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

        # Separate permitted Dublin Core keys from custom research keys
        custom_json_metadata = {}
        for key, val in user_metadata.items():
            key_lower = str(key).strip().lower()
            if key_lower in PERMITTED_SVG_KEYS and key_lower != "description":
                canonical_key = CANONICAL_SVG_KEYS[key_lower]
                svg_meta[canonical_key] = str(val)
            else:
                custom_json_metadata[key] = val

        # Pack caller context and all non-standard keys into the JSON payload
        ctx.update(custom_json_metadata)
        payload = json.dumps(ctx, indent=2, ensure_ascii=False, default=str)
        svg_meta["Description"] = payload

        fig.savefig(fname, metadata=svg_meta, **kwargs)

    # -------------------------------------------------------------------------
    # 3. PDF Backend (PDF /Info dictionary validation)
    # -------------------------------------------------------------------------
    elif ext == ".pdf":
        # Separate permitted PDF keys (Title, Author, Subject, etc.) from custom keys
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

        # Merge caller context with custom keys into the Keywords JSON payload
        ctx.update(custom_json_metadata)
        payload = json.dumps(ctx, indent=2, ensure_ascii=False, default=str)
        native_pdf_meta["Keywords"] = payload

        fig.savefig(fname, metadata=native_pdf_meta, **kwargs)

    # -------------------------------------------------------------------------
    # 4. Other Backends (EPS, JPEG, etc.)
    # -------------------------------------------------------------------------
    else:
        fig.savefig(fname, **kwargs)


def read_metadata(image_path: str) -> dict:
    """
    Extract metadata and embedded source code from a PNG, SVG, or PDF figure.

    Returns:
        dict containing all extracted metadata keys and unpacked JSON fields.
    """
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
        print(f"# File: {meta.get('SourceFile') or meta.get('filename')}")
        print(f"# Dir:  {meta.get('SourceDirectory') or meta.get('directory')}")
        print(f"# Git:  {meta.get('GitCommit') or meta.get('git_commit')} (Dirty: {meta.get('GitDirty') or meta.get('git_dirty')})")
        print("-" * 60)
        print(source)