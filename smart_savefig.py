#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Created on Wed Sep  9 15:08:09 2026

@author: egio

smart_savefig.py - Salva figure matplotlib incorporando nei metadati
il file, la directory e il listato sorgente completo del modulo chiamante.
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


def _get_git_info(directory: str) -> dict:
    """Recupera commit e stato del repository Git se presente."""
    try:
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=directory,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
        dirty = (
            subprocess.check_output(
                ["git", "status", "--porcelain"],
                cwd=directory,
                stderr=subprocess.DEVNULL,
                text=True,
            ).strip()
            != ""
        )
        return {"git_commit": commit, "git_dirty": dirty}
    except Exception:
        return {}


def _get_caller_context() -> dict:
    """Risale lo stack frame per individuare il modulo chiamante."""
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
            source_code = f"<Errore lettura sorgente: {e}>"
    else:
        # Sessione interattiva o eval
        source_code = f"<Non eseguito da file su disco: {caller_path}>"

    info = {
        "directory": caller_dir,
        "filename": caller_name,
        "filepath": str(caller_path),
        "source_code": source_code,
        "caller_function": caller_frame.f_code.co_name,
        "caller_line": caller_frame.f_lineno,
        "timestamp": datetime.now().isoformat(),
    }

    info.update(_get_git_info(caller_dir))
    return info


def savefig(fname, fig=None, **kwargs):
    """
    Salva la figura inserendo nei metadati directory, nome file e sorgente del chiamante.

    Parametri:
        fname: percorso o nome file (.png o .svg).
        fig: oggetto matplotlib.figure.Figure (default: plt.gcf()).
        **kwargs: argomenti passati a matplotlib.pyplot.savefig.
    """
    if fig is None:
        fig = plt.gcf()

    ext = Path(fname).suffix.lower()
    ctx = _get_caller_context()
    user_metadata = kwargs.pop("metadata", {}) or {}

    if ext == ".png":
        # Pillow e Matplotlib supportano chiavi arbitrarie nei chunk testuali tEXt/zTXt
        meta = {
            "SourceDirectory": ctx["directory"],
            "SourceFile": ctx["filename"],
            "SourcePath": ctx["filepath"],
            "CallerFunction": ctx["caller_function"],
            "Timestamp": ctx["timestamp"],
            "SourceCode": ctx["source_code"],
            # Comment è standard e leggibile da quasi tutti i visualizzatori/exiftool
            "Comment": ctx["source_code"],
            "Description": f"Generato da {ctx['filename']} ({ctx['caller_function']})",
        }
        if "git_commit" in ctx:
            meta["GitCommit"] = ctx["git_commit"]
            meta["GitDirty"] = str(ctx["git_dirty"])

        meta.update(user_metadata)
        fig.savefig(fname, metadata=meta, **kwargs)

    elif ext == ".svg":
        # Matplotlib supporta in RDF: Creator, Date, Format, Type, Description, Title
        # Serializziamo il contesto completo in JSON dentro 'Description'
        payload = json.dumps(ctx, indent=2, ensure_ascii=False)
        meta = {
            "Title": ctx["filename"],
            "Description": payload,
            "Date": ctx["timestamp"],
            "Type": "Scientific Visualization",
        }
        meta.update(user_metadata)
        fig.savefig(fname, metadata=meta, **kwargs)

    else:
        # Fallback per altri formati (es. PDF)
        fig.savefig(fname, **kwargs)


def read_metadata(image_path: str) -> dict:
    """Estrae i metadati e il sorgente da un file PNG o SVG salvato con smart_savefig."""
    path = Path(image_path)
    if not path.is_file():
        raise FileNotFoundError(f"File non trovato: {image_path}")

    ext = path.suffix.lower()

    if ext == ".png":
        with Image.open(path) as img:
            return dict(img.info)

    elif ext == ".svg":
        tree = ET.parse(path)
        root = tree.getroot()
        # Cerca il tag dc:description nei namespace RDF/Dublin Core
        ns = {"dc": "http://purl.org/dc/elements/1.1/"}
        desc_elem = root.find(".//dc:description", ns)

        if desc_elem is not None and desc_elem.text:
            try:
                return json.loads(desc_elem.text)
            except json.JSONDecodeError:
                return {"Description": desc_elem.text}
        return {}

    return {}


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(
        description="Estrae il codice sorgente e i metadati da figure PNG/SVG."
    )
    parser.add_argument("image", help="Percorso dell'immagine PNG o SVG")
    parser.add_argument(
        "-o", "--output", help="Salva il codice estratto su un file .py"
    )
    args = parser.parse_args()

    meta = read_metadata(args.image)
    source = meta.get("SourceCode") or meta.get("source_code") or meta.get("Comment")

    if not source:
        print("Nessun codice sorgente trovato nei metadati.", file=sys.stderr)
        sys.exit(1)

    if args.output:
        Path(args.output).write_text(source, encoding="utf-8")
        print(f"Sorgente estratto con successo in: {args.output}")
    else:
        print(
            f"# File di origine: {meta.get('SourceFile') or meta.get('filename')}"
        )
        print(
            f"# Directory: {meta.get('SourceDirectory') or meta.get('directory')}"
        )
        print("-" * 60)
        print(source)