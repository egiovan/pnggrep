"""
smart_savefig.py - Reproducible Scientific Visualization Module

Drop-in replacement for `matplotlib.pyplot.savefig`.
Automatically embeds caller script source code, file system context,
runtime timestamp, Git commit SHA, Git commit message, sys.path entries,
fusion shot/pulse identifiers, local module dependencies, and database schemas
into image metadata for PNG, SVG, and PDF formats.

Default resolution: 300 DPI.

Usage examples:
    savefig("equilibrium.png", shot=92185, database=df_plasma)
    savefig("comparison.pdf", shots=[92185, 92186], databases=[df_mhd, ds_equil])
    savefig("analysis.svg", pulse=92185, databases={"mhd": df_mhd}, modules=[ge, gp])

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
    # Fusion-specific keys
    "shot",
    "shots",
    "pulse",
    "pulses",
    # Database and tabular metadata
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
            or norm_key.startswith("database/")
            or norm_key.startswith("database:")
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


def _normalize_int_or_str(val):
    """Convert single shot/pulse identifier into int if numeric, else trimmed str."""
    if val is None:
        return None
    if isinstance(val, int):
        return val
    s = str(val).strip()
    try:
        return int(s)
    except ValueError:
        return s


def _normalize_sequence(val):
    """Convert sequence of shot/pulse identifiers into a deduplicated list."""
    if val is None:
        return None
    if isinstance(val, (int, str)):
        items = [val]
    elif isinstance(val, (list, tuple, set)):
        items = list(val)
    else:
        items = [val]

    result = []
    for x in items:
        norm = _normalize_int_or_str(x)
        if norm is not None and norm not in result:
            result.append(norm)
    return result


def _extract_database_schema(db, label: str = None) -> dict:
    """
    Extract structural schema from a database object (Pandas DataFrame/Series,
    xarray Dataset/DataArray, file path, or dictionary) via duck typing.
    Never extracts raw numeric array values.
    """
    if db is None:
        return {}

    type_name = f"{db.__class__.__module__}.{db.__class__.__name__}"
    schema = {"type": type_name}
    if label:
        schema["name"] = str(label)

    # 1. Path or file string
    if isinstance(db, (str, Path)):
        schema["type"] = "file_path"
        schema["path"] = str(db)
        if not label:
            schema["name"] = Path(str(db)).name
        return schema

    # 2. Pandas DataFrame duck typing
    if hasattr(db, "columns") and hasattr(db, "shape") and hasattr(db, "dtypes"):
        schema["shape"] = list(db.shape)
        schema["columns"] = [str(c) for c in db.columns]
        idx_name = getattr(getattr(db, "index", None), "name", None)
        if idx_name:
            schema["index_name"] = str(idx_name)
        attrs = getattr(db, "attrs", None)
        if isinstance(attrs, dict) and attrs:
            schema["attrs"] = {str(k): str(v) for k, v in attrs.items()}
        return schema

    # 3. Pandas Series duck typing
    if hasattr(db, "index") and hasattr(db, "shape") and hasattr(db, "dtype") and not hasattr(db, "columns"):
        schema["shape"] = list(db.shape)
        series_name = getattr(db, "name", None)
        if series_name is not None and not label:
            schema["name"] = str(series_name)
        idx_name = getattr(getattr(db, "index", None), "name", None)
        if idx_name:
            schema["index_name"] = str(idx_name)
        attrs = getattr(db, "attrs", None)
        if isinstance(attrs, dict) and attrs:
            schema["attrs"] = {str(k): str(v) for k, v in attrs.items()}
        return schema

    # 4. xarray Dataset duck typing
    if hasattr(db, "data_vars") and hasattr(db, "coords") and hasattr(db, "dims"):
        schema["dims"] = {str(k): int(v) for k, v in db.dims.items()}
        schema["coords"] = [str(c) for c in db.coords]
        schema["data_vars"] = [str(v) for v in db.data_vars]
        attrs = getattr(db, "attrs", None)
        if isinstance(attrs, dict) and attrs:
            schema["attrs"] = {str(k): str(v) for k, v in attrs.items()}
        return schema

    # 5. xarray DataArray duck typing
    if hasattr(db, "coords") and hasattr(db, "dims") and (hasattr(db, "values") or hasattr(db, "data")) and not hasattr(db, "data_vars"):
        schema["shape"] = list(db.shape) if hasattr(db, "shape") else []
        schema["dims"] = [str(d) for d in db.dims]
        schema["coords"] = [str(c) for c in db.coords]
        arr_name = getattr(db, "name", None)
        if arr_name is not None and not label:
            schema["name"] = str(arr_name)
        attrs = getattr(db, "attrs", None)
        if isinstance(attrs, dict) and attrs:
            schema["attrs"] = {str(k): str(v) for k, v in attrs.items()}
        return schema

    # 6. Generic dictionary
    if isinstance(db, dict):
        schema["type"] = "dict"
        schema["keys"] = [str(k) for k in list(db.keys())[:50]]
        return schema

    # 7. Fallback
    schema["repr"] = repr(db)[:200]
    return schema


def _collect_database_schemas(database, databases) -> list:
    """Collect and normalize database schemas from database and databases arguments."""
    items = []
    if database is not None:
        items.append((None, database))
    if databases is not None:
        if isinstance(databases, dict):
            for k, v in databases.items():
                items.append((str(k), v))
        elif isinstance(databases, (list, tuple, set)):
            for v in databases:
                items.append((None, v))
        else:
            items.append((None, databases))

    schemas = []
    for label, db_obj in items:
        extracted = _extract_database_schema(db_obj, label=label)
        if extracted:
            schemas.append(extracted)
    return schemas


def _format_databases_text(schemas: list) -> str:
    """
    Format database schemas into a line-delimited stream:
        %%% Database: <name> [<type>]
        Columns: ...
        Shape: ...
    """
    if not schemas:
        return ""

    blocks = []
    for s in schemas:
        name_part = f" {s['name']}" if s.get("name") else ""
        type_part = s.get("type", "Unknown")
        lines = [f"%%% Database:{name_part} [{type_part}]"]

        if "shape" in s:
            lines.append(f"  Shape: {tuple(s['shape'])}")
        if "dims" in s:
            lines.append(f"  Dims: {s['dims']}")
        if "columns" in s:
            cols = s["columns"]
            cols_preview = ", ".join(cols[:25])
            if len(cols) > 25:
                cols_preview += f", ... ({len(cols)} total)"
            lines.append(f"  Columns: {cols_preview}")
        if "data_vars" in s:
            lines.append(f"  DataVars: {', '.join(s['data_vars'])}")
        if "coords" in s:
            lines.append(f"  Coords: {', '.join(s['coords'])}")
        if "index_name" in s:
            lines.append(f"  Index: {s['index_name']}")
        if "path" in s:
            lines.append(f"  Path: {s['path']}")
        if "keys" in s:
            lines.append(f"  Keys: {', '.join(s['keys'])}")
        if s.get("attrs"):
            lines.append(f"  Attrs: {s['attrs']}")

        blocks.append("\n".join(lines))

    return "\n\n".join(blocks)


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


def savefig(
    fname,
    fig=None,
    modules=None,
    shot=None,
    shots=None,
    pulse=None,
    pulses=None,
    database=None,
    databases=None,
    dpi=300,
    **kwargs,
):
    """
    Save a Matplotlib figure embedding caller source code, Git metadata,
    sys.path entries, fusion shot/pulse parameters, and database schemas.

    Parameters:
        fname: Output filename or path (.png, .svg, or .pdf).
        fig: matplotlib.figure.Figure instance (defaults to plt.gcf()).
        modules: Optional list/tuple of imported module objects whose source code
                 should be embedded (e.g. `modules=[ge, gp]`).
        shot: Single fusion shot identifier (int or str).
        shots: Sequence of fusion shot identifiers (list, tuple, set).
        pulse: Single fusion pulse identifier (int or str).
        pulses: Sequence of fusion pulse identifiers (list, tuple, set).
        database: Single database/table (DataFrame, Dataset, file path, dict).
        databases: Sequence or dict of databases/tables.
        dpi: Resolution in dots per inch [default: 300].
        **kwargs: Extra keyword arguments forwarded to matplotlib.pyplot.savefig.
    """
    if fig is None:
        fig = plt.gcf()

    # Fallback checks from kwargs
    if modules is None:
        modules = kwargs.pop("include_modules", None)
    if shot is None:
        shot = kwargs.pop("shot", None)
    if shots is None:
        shots = kwargs.pop("shots", None)
    if pulse is None:
        pulse = kwargs.pop("pulse", None)
    if pulses is None:
        pulses = kwargs.pop("pulses", None)
    if database is None:
        database = kwargs.pop("database", None)
    if databases is None:
        databases = kwargs.pop("databases", None)

    ext = Path(fname).suffix.lower()
    ctx = _get_caller_context(explicit_modules=modules)
    raw_user_metadata = dict(kwargs.pop("metadata", {}) or {})
    user_metadata = isolate_user_metadata(raw_user_metadata)

    # 1. Process fusion parameters
    fusion_ctx = {}
    if shot is not None:
        fusion_ctx["shot"] = _normalize_int_or_str(shot)
    if shots is not None:
        fusion_ctx["shots"] = _normalize_sequence(shots)
    if pulse is not None:
        fusion_ctx["pulse"] = _normalize_int_or_str(pulse)
    if pulses is not None:
        fusion_ctx["pulses"] = _normalize_sequence(pulses)

    ctx.update(fusion_ctx)

    # 2. Process database schemas
    db_schemas = _collect_database_schemas(database, databases)
    if db_schemas:
        ctx["databases"] = db_schemas

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

        # Embed fusion keys as standalone PNG chunks
        if "shot" in fusion_ctx:
            meta["Shot"] = str(fusion_ctx["shot"])
        if "shots" in fusion_ctx:
            meta["Shots"] = json.dumps(fusion_ctx["shots"])
        if "pulse" in fusion_ctx:
            meta["Pulse"] = str(fusion_ctx["pulse"])
        if "pulses" in fusion_ctx:
            meta["Pulses"] = json.dumps(fusion_ctx["pulses"])

        # Embed database schemas as line-delimited stream
        if db_schemas:
            meta["Databases"] = _format_databases_text(db_schemas)

        if "git_commit" in ctx:
            meta["GitCommit"] = ctx["git_commit"]
            meta["GitCommitMessage"] = ctx.get("git_commit_message", "")
            meta["GitDirty"] = str(ctx["git_dirty"])
            meta["GitScriptTracked"] = str(ctx.get("git_script_tracked", False))

        fig.savefig(fname, metadata=meta, dpi=dpi, **kwargs)

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

        fig.savefig(fname, metadata=svg_meta, dpi=dpi, **kwargs)

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

        fig.savefig(fname, metadata=native_pdf_meta, dpi=dpi, **kwargs)

    # -------------------------------------------------------------------------
    # 4. Other Backends
    # -------------------------------------------------------------------------
    else:
        fig.savefig(fname, dpi=dpi, **kwargs)


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

        for f_key in ["Shot", "shot", "Pulse", "pulse"]:
            if meta.get(f_key):
                print(f"# {f_key.capitalize()}:     {meta[f_key]}")
        for f_key in ["Shots", "shots", "Pulses", "pulses"]:
            if meta.get(f_key):
                print(f"# {f_key.capitalize()}:    {meta[f_key]}")

        local_mods_raw = meta.get("LocalModules") or meta.get("local_modules", "")
        if local_mods_raw:
            mod_names = re.findall(r"^%%%\s*(\S+)\s*%", local_mods_raw, flags=re.MULTILINE)
            if mod_names:
                print(f"# Local Modules: {', '.join(mod_names)}")

        db_raw = meta.get("Databases") or meta.get("databases")
        if isinstance(db_raw, list):
            db_names = [d.get("name") or d.get("type") for d in db_raw if isinstance(d, dict)]
            if db_names:
                print(f"# Databases:     {', '.join(db_names)}")
        elif isinstance(db_raw, str):
            db_names = re.findall(r"^%%%\s*Database:\s*(\S+)", db_raw, flags=re.MULTILINE)
            if db_names:
                print(f"# Databases:     {', '.join(db_names)}")

        print("-" * 60)
        print(source)