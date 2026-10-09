"""
smart_savefig.py - Reproducible Scientific Visualization Module

Drop-in replacement for `matplotlib.pyplot.savefig`.
Automatically embeds caller script source code, file system context,
runtime timestamp, Git commit SHA, Git commit message, and sys.path entries
into image metadata for PNG, SVG, and PDF formats.

Optionally embeds full source code of specified local module objects via `modules=[...]`:
    savefig("fig.png", modules=[ge, gp])

Local modules are formatted as a line-delimited stream:
    %%% <modulename> % <relpath>
    <source code...>

Alerts the user on stderr if saving from a dirty or untracked repository state.
Guarantees immutability of system provenance keys against user metadata collisions.
"""

from datetime import datetime
import inspect
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import types
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

# Normalized system reserved keys (checked lowercase without hyphens/underscores)
RESERVED_SYSTEM_KEYS = {
    # System context & provenance
    "directory",
    "sourcedirectory",
    "filename",
    "sourcefile",
    "filepath",
    "sourcepath",
    "sourcecode",
    "comment",
    "callerfunction",
    "callerline",
    "timestamp",
    "description",
    "gitcommit",
    "gitcommitmessage",
    "gitdirty",
    "gitscripttracked",
    "gitdirtyreason",
    "pythonversion",
    # Environment and module hierarchy
    "syspath",
    "localmodules",
    "modules",
    # Reserved for upcoming extensions
    "shot",
    "shots",
    "pulse",
    "pulses",
    "database",
    "databases",
}


def _normalize_key(key: str) -> str:
    """Normalize a key name for collision testing."""
    return str(key).lower().replace("_", "").replace("-", "")


def isolate_user_metadata(user_metadata: dict) -> dict:
    """
    Sanitizes user-provided metadata.
    If a key collides with a reserved system key or dynamic module prefix,
    it is remapped to 'User/<Key>' and a warning is emitted to stderr.
    """
    if not isinstance(user_metadata, dict):
        return {}

    sanitized = {}
    for key, value in user_metadata.items():
        norm_key = _normalize_key(key)
        is_conflict = (
            norm_key in RESERVED_SYSTEM_KEYS
            or norm_key.startswith("sourcecode/")
            or norm_key.startswith("sourcecode:")
            or norm_key.startswith("localmodule/")
            or norm_key.startswith("localmodule:")
        )

        if is_conflict:
            safe_key = f"User/{key}"
            sys.stderr.write(
                f"\x1b[33m[smart_savefig warning]\x1b[0m User metadata key '{key}' "
                f"conflicts with system reserved key. Remapped to '{safe_key}'.\n"
            )
            sanitized[safe_key] = value
        else:
            sanitized[key] = value

    return sanitized


def _get_sys_path() -> list:
    """Retrieve normalized, deduplicated list of sys.path directory entries."""
    seen = set()
    normalized = []
    for p in sys.path:
        if not p:
            abs_p = os.path.abspath(os.getcwd())
        else:
            abs_p = os.path.abspath(p)
        if abs_p not in seen:
            seen.add(abs_p)
            normalized.append(abs_p)
    return normalized


def _get_explicit_modules(modules_list, caller_dir: Path, caller_path: Path) -> dict:
    """
    Extract source code and relative paths for an explicitly provided list
    of Python module objects.

    Excludes:
      - The caller script itself
      - smart_savefig.py itself (even if aliased or symlinked)
    """
    if not modules_list:
        return {}

    if not isinstance(modules_list, (list, tuple, set)):
        modules_list = [modules_list]

    local_mods = {}
    try:
        caller_dir_res = caller_dir.resolve()
        caller_file_res = caller_path.resolve() if caller_path.is_file() else None
        this_file_res = Path(__file__).resolve()
    except Exception:
        return {}

    this_module = sys.modules.get(__name__)

    for mod in modules_list:
        if mod is None or mod is this_module:
            continue

        if not isinstance(mod, types.ModuleType):
            sys.stderr.write(
                f"\x1b[33m[smart_savefig warning]\x1b[0m Argument in 'modules' is not a module object: {repr(mod)}. Skipped.\n"
            )
            continue

        name = getattr(mod, "__name__", None)
        mod_file = getattr(mod, "__file__", None)

        if not name or not mod_file:
            continue

        try:
            raw_mod_path = Path(mod_file)
            resolved_mod_path = raw_mod_path.resolve()

            # 1. Skip smart_savefig.py under any circumstance
            if (
                resolved_mod_path == this_file_res
                or raw_mod_path.name.startswith("smart_savefig")
                or resolved_mod_path.name.startswith("smart_savefig")
                or name.endswith("smart_savefig")
            ):
                continue

            # 2. Skip the caller script itself (already under SourceCode)
            if caller_file_res and (
                resolved_mod_path == caller_file_res or raw_mod_path == caller_path
            ):
                continue

            # 3. Locate source .py if imported via bytecode cache (__pycache__)
            if resolved_mod_path.suffix == ".pyc":
                py_candidate = resolved_mod_path.with_suffix(".py")
                if py_candidate.is_file():
                    resolved_mod_path = py_candidate
                elif resolved_mod_path.parent.name == "__pycache__":
                    parent_py = resolved_mod_path.parent.parent / (
                        resolved_mod_path.stem.split(".")[0] + ".py"
                    )
                    if parent_py.is_file():
                        resolved_mod_path = parent_py

            if not resolved_mod_path.is_file():
                continue

            try:
                rel_path = resolved_mod_path.relative_to(caller_dir_res)
            except ValueError:
                rel_path = raw_mod_path.name

            try:
                source_code = resolved_mod_path.read_text(
                    encoding="utf-8", errors="replace"
                )
            except Exception as e:
                source_code = f"<Error reading source file: {e}>"

            local_mods[name] = {
                "relpath": str(rel_path).replace("\\", "/"),
                "source_code": source_code,
            }
        except Exception:
            continue

    return dict(sorted(local_mods.items()))


def _format_local_modules(local_mods: dict) -> str:
    """
    Format local modules into a single line-delimited string:
        %%% <modulename> % <relpath>
        <source_code>
    """
    if not local_mods:
        return ""

    blocks = []
    for name, info in local_mods.items():
        relpath = info.get("relpath", "")
        code = info.get("source_code", "").rstrip("\n")
        blocks.append(f"%%% {name} % {relpath}\n{code}")

    return "\n\n".join(blocks)


def _get_git_info(directory: str, script_name: str = None) -> dict:
    """Query Git for commit hash, commit message, and repository status."""
    try:
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


def _get_caller_context(explicit_modules=None) -> dict:
    """Inspect the Python call stack to identify caller script and environment."""
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
            "python_version": sys.version.split()[0],
            "sys_path": "\n".join(_get_sys_path()),
            "local_modules": "",
        }

    caller_path = Path(caller_frame.f_code.co_filename).resolve()
    caller_dir = caller_path.parent
    caller_name = caller_path.name

    if caller_path.is_file():
        try:
            source_code = caller_path.read_text(encoding="utf-8")
        except Exception as e:
            source_code = f"<Error reading source file: {e}>"
    else:
        source_code = f"<Not executed from a disk file: {caller_path}>"

    # Only process modules if explicitly passed
    if explicit_modules:
        local_mods_dict = _get_explicit_modules(explicit_modules, caller_dir, caller_path)
        local_mods_str = _format_local_modules(local_mods_dict)
    else:
        local_mods_str = ""

    sys_path_str = "\n".join(_get_sys_path())

    info = {
        "directory": str(caller_dir),
        "filename": caller_name,
        "filepath": str(caller_path),
        "source_code": source_code,
        "caller_function": caller_frame.f_code.co_name,
        "caller_line": caller_frame.f_lineno,
        "timestamp": datetime.now().isoformat(),
        "python_version": sys.version.split()[0],
        "sys_path": sys_path_str,
        "local_modules": local_mods_str,
    }

    script_name = caller_name if caller_path.is_file() else None
    info.update(_get_git_info(str(caller_dir), script_name))
    return info


def savefig(fname, fig=None, modules=None, **kwargs):
    """
    Save a Matplotlib figure embedding caller source code, Git metadata,
    and sys.path entries.

    Parameters:
        fname: Output filename or path (.png, .svg, or .pdf).
        fig: matplotlib.figure.Figure instance (defaults to plt.gcf()).
        modules: Optional list/tuple of imported module objects whose source code
                 should be embedded (e.g. `modules=[ge, gp]`).
        **kwargs: Extra keyword arguments forwarded to matplotlib.pyplot.savefig.
    """
    if fig is None:
        fig = plt.gcf()

    # Support passing modules via kwargs as well
    if modules is None:
        modules = kwargs.pop("include_modules", None)

    ext = Path(fname).suffix.lower()
    ctx = _get_caller_context(explicit_modules=modules)
    raw_user_metadata = dict(kwargs.pop("metadata", {}) or {})
    user_metadata = isolate_user_metadata(raw_user_metadata)

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
        # Safe user metadata populated first
        meta = {str(k): str(v) for k, v in user_metadata.items()}

        # Authoritative system metadata overwrites to guarantee provenance
        meta.update({
            "SourceDirectory": ctx["directory"],
            "SourceFile": ctx["filename"],
            "SourcePath": ctx["filepath"],
            "CallerFunction": ctx["caller_function"],
            "Timestamp": ctx["timestamp"],
            "PythonVersion": ctx["python_version"],
            "SourceCode": ctx["source_code"],
            "Comment": ctx["source_code"],
            "Description": f"Generated by {ctx['filename']} ({ctx['caller_function']})",
            "SysPath": ctx["sys_path"],
        })

        # Only embed LocalModules if modules were explicitly requested and found
        if ctx["local_modules"]:
            meta["LocalModules"] = ctx["local_modules"]

        if "git_commit" in ctx:
            meta["GitCommit"] = ctx["git_commit"]
            meta["GitCommitMessage"] = ctx.get("git_commit_message", "")
            meta["GitDirty"] = str(ctx["git_dirty"])
            meta["GitScriptTracked"] = str(ctx.get("git_script_tracked", False))

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

        # System provenance strictly overrides user values
        final_ctx = {**custom_json_metadata, **ctx}
        if not final_ctx.get("local_modules"):
            final_ctx.pop("local_modules", None)

        payload = json.dumps(final_ctx, indent=2, ensure_ascii=False, default=str)
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

        # System provenance strictly overrides user values
        final_ctx = {**custom_json_metadata, **ctx}
        if not final_ctx.get("local_modules"):
            final_ctx.pop("local_modules", None)

        payload = json.dumps(final_ctx, indent=2, ensure_ascii=False, default=str)
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
                raw_val.replace(b'\\(', b'(')                 .replace(b'\\)', b')')
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
        print(
            f"# Commit:  {meta.get('GitCommit') or meta.get('git_commit')} "
            f"(Dirty: {meta.get('GitDirty') or meta.get('git_dirty')})"
        )
        if meta.get("GitCommitMessage") or meta.get("git_commit_message"):
            print(f"# Message: {meta.get('GitCommitMessage') or meta.get('git_commit_message')}")

        local_mods_raw = meta.get("LocalModules") or meta.get("local_modules", "")
        if local_mods_raw:
            mod_names = re.findall(r"^%%%\s*(\S+)\s*%", local_mods_raw, flags=re.MULTILINE)
            if mod_names:
                print(f"# Local Modules: {', '.join(mod_names)}")

        print("-" * 60)
        print(source)