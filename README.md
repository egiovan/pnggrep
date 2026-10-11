# Table of Contents

* [1. Overview & Tool Architecture](https://www.google.com/search?q=%231-overview--tool-architecture)
* [2. `smart_savefig.py` (Visualization & Provenance Hook)](https://www.google.com/search?q=%232-smart_savefigpy-visualization--provenance-hook)
* [Function Signature & Parameter Reference](https://www.google.com/search?q=%23function-signature--parameter-reference)
* [Heuristic Vector Auto-Rasterization](https://www.google.com/search?q=%23heuristic-vector-auto-rasterization)
* [Format Packaging & Metadata Architecture](https://www.google.com/search?q=%23format-packaging--metadata-architecture)
* [System Metadata Reference Table](https://www.google.com/search?q=%23system-metadata-reference-table)
* [User Metadata Isolation & Collision Safety](https://www.google.com/search?q=%23user-metadata-isolation--collision-safety)
* [Domain Parameters & Module Bundling Reference](https://www.google.com/search?q=%23domain-parameters--module-bundling-reference)
* [Embedded Module Storage Format](https://www.google.com/search?q=%23embedded-module-storage-format)


* [3. `pnggrep` (Metadata Inspection CLI)](https://www.google.com/search?q=%233-pnggrep-metadata-inspection-cli)
* [Core Operating Modes](https://www.google.com/search?q=%23core-operating-modes)
* [CLI Flags & Options Reference](https://www.google.com/search?q=%23cli-flags--options-reference)
* [Hierarchical Display & JSON Flattening](https://www.google.com/search?q=%23hierarchical-display--json-flattening)
* [Intelligent Directory Traversal & Exclusion Engine](https://www.google.com/search?q=%23intelligent-directory-traversal--exclusion-engine)


* [4. Installation Guide](https://www.google.com/search?q=%234-installation-guide)
* [Linux & macOS (Native HPC & Workstation)](https://www.google.com/search?q=%23linux--macos-native-hpc--workstation)
* [Windows Installation](https://www.google.com/search?q=%23windows-installation)
* [Option A: Cross-Compilation via WSL (Recommended)](https://www.google.com/search?q=%23option-a-cross-compilation-via-wsl-recommended)
* [Option B: Native Windows Build](https://www.google.com/search?q=%23option-b-native-windows-build)


* [Python Integration](https://www.google.com/search?q=%23python-integration)


* [5. Practical Examples & Workflows](https://www.google.com/search?q=%235-practical-examples--workflows)
* [Generating Figures with Embedded Provenance](https://www.google.com/search?q=%23generating-figures-with-embedded-provenance)
* [Querying Metadata by Text, Pulse, and Date](https://www.google.com/search?q=%23querying-metadata-by-text-pulse-and-date)
* [Inspecting and Extracting Bundled Python Modules](https://www.google.com/search?q=%23inspecting-and-extracting-bundled-python-modules)
* [Restoring Scripts and Sanitizing Output](https://www.google.com/search?q=%23restoring-scripts-and-sanitizing-output)


* [6. Technical Architecture & Security Safeguards](https://www.google.com/search?q=%236-technical-architecture--security-safeguards)
* [Lazy Chunk Decompression](https://www.google.com/search?q=%23lazy-chunk-decompression)
* [Security Hardening & DoS Protections](https://www.google.com/search?q=%23security-hardening--dos-protections)


* [7. Regulatory AI Disclosure & Compliance](https://www.google.com/search?q=%237-regulatory-ai-disclosure--compliance)
* [8. License](https://www.google.com/search?q=%238-license)

---

# 1. Overview & Tool Architecture

In computational, experimental, and high-performance physics research, publication-ready plots and figures frequently become disconnected from the code, environment, and parameters that generated them. Weeks or months later, re-identifying an experimental shot number, reconstructing the exact analysis script, or verifying whether uncommitted Git changes were present requires manual record-keeping.

This repository provides an integrated, dual-component system designed to solve figure reproducibility without external databases or server infrastructure:

```
[ Matplotlib Plotting Workflow ]
              │
              ▼
    smart_savefig.py
   (Embeds source code, host, user, Git status,
    axis labels, legends, shot numbers, schemas,
    and imported helper modules into image headers)
              │
              ├──► Generates: .png  (Native chunks: tEXt, zTXt, iTXt)
              ├──► Generates: .svg  (Dublin Core dc:description JSON)
              └──► Generates: .pdf  (Native /Info dictionary Keywords JSON)
              │
              ▼
           pnggrep
    (High-performance Rust CLI:
     Bypasses heavy rendering decoders,
     lazily searches metadata, matches pulse numbers,
     filters by date windows, inspects bundled modules,
     and restores complete source scripts in milliseconds)

```

1. **`smart_savefig.py`**: A drop-in replacement for `matplotlib.pyplot.savefig`. It inspects the Python call stack, runtime environment, figure artist tree, and local modules to embed comprehensive provenance metadata directly into **PNG**, **SVG**, and **PDF** containers while intelligently managing vector plot rasterization.
2. **`pnggrep`**: A standalone, zero-dependency Rust CLI utility. It scans directory trees, reads figure headers without decoding raster bitmaps, evaluates date and pulse queries, lists metadata fields, and extracts embedded scripts or bundled modules directly to `stdout`.

---

# 2. `smart_savefig.py` (Visualization & Provenance Hook)

## Function Signature & Parameter Reference

`smart_savefig.savefig` wraps Matplotlib's native export engine. It accepts all keyword arguments supported by `matplotlib.pyplot.savefig`, augmenting them with provenance and data parameters:

```python
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
    auto_rasterize=True,
    **kwargs,
)

```

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `fname` | `str` or `Path` | *Required* | Output path (`.png`, `.svg`, or `.pdf`). |
| `fig` | `Figure` | `None` | Target Matplotlib figure instance (defaults to `plt.gcf()`). Supports subfigures (`fig.subfigs`). |
| `modules` | `list[Module]` | `None` | Explicit list of imported Python module objects to bundle directly into image metadata. |
| `shot` | `int` or `str` | `None` | Single fusion experimental shot identifier. |
| `shots` | `Sequence` | `None` | Collection of experimental shot identifiers. |
| `pulse` | `int` or `str` | `None` | Single fusion pulse identifier (synonym for `shot`). |
| `pulses` | `Sequence` | `None` | Collection of fusion pulse identifiers (synonym for `shots`). |
| `database` | `Any` | `None` | Single tabular dataset or schema object (Pandas DataFrame/Series, xarray Dataset/DataArray, dict, Path). |
| `databases` | `Sequence`/`dict` | `None` | Collection or keyed mapping of database schema objects. |
| `dpi` | `int` | `300` | Rasterization resolution in dots per inch. |
| `auto_rasterize` | `bool` | `True` | Automatically enables `rasterized=True` on complex collections during vector export. |
| `**kwargs` | `Any` | `None` | Arbitrary keyword arguments passed to `matplotlib.figure.Figure.savefig` (e.g. `bbox_inches`, `pad_inches`, `transparent`). |

---

## Heuristic Vector Auto-Rasterization

Vector graphics formats (`.pdf`, `.svg`) preserve sharp typography and line plots at arbitrary zoom levels. However, plotting dense colormeshes or large marker sets can create multi-megabyte files and viewer lag. Furthermore, vector renderers draw adjacent polygon cells with visible anti-aliased white seams ("stitching artifacts").

`smart_savefig` solves this using an automated heuristic rasterizer targeting complex vector artists:

```
Threshold Check:
├── QuadMesh (pcolormesh)     >= 2,000 cells   --> Set rasterized=True (prevents stitching & file bloat)
├── PathCollection (scatter)  >= 10,000 points --> Set rasterized=True (prevents DOM/PDF object bloat)
├── PolyCollection (contourf) >= 1,000 paths   --> Set rasterized=True (flattens contour polygons)
└── Line2D (continuous lines)                  --> KEPT PURE VECTOR (handled by Matplotlib path.simplify)

```

* **Preservation of Typography:** Figure axes, tick marks, spine borders, suptitles, axis labels, and text annotations remain 100% vector objects.
* **Continuous Line Optimization:** Continuous line plots (`Line2D`) are deliberately bypassed by the auto-rasterizer regardless of vertex count. Matplotlib's built-in Ramer–Douglas–Peucker (RDP) path simplifier (`path.simplify=True`) strips subpixel collinear points during vector generation, rendering 100,000-point curves into compact 8–15 KB vector streams. Rasterizing them into a 300 DPI bitmap would make the file larger.
* **Non-Destructive In-Memory Execution:** Artists are flagged as rasterized immediately before export and reverted back to their original state in a `try...finally` block. The in-memory figure instance (`fig`) remains unmodified for subsequent notebook execution or interactive display.

---

## Format Packaging & Metadata Architecture

Different image file formats have distinct structural metadata constraints:

* **PNG:** Implements the official W3C PNG chunk architecture. Metadata entries are stored as discrete key-value chunks (`tEXt` for raw strings, `zTXt` for Deflate-compressed streams like source listings, and `iTXt` for UTF-8 payloads).
* **SVG:** SVG specifications restrict header metadata to Dublin Core schema tags (`dc:title`, `dc:creator`, `dc:date`, `dc:description`, etc.). Matplotlib drops unknown custom tags. `smart_savefig` sets standard tags (`dc:title`, `dc:date`) natively and packs all structured provenance into an escaped JSON payload inside `<dc:description>`.
* **PDF:** PDF specifications restrict document metadata to the document `/Info` dictionary (`/Title`, `/Author`, `/Subject`, `/Keywords`, etc.). Arbitrary dictionary keys are dropped by PDF rendering engines. `smart_savefig` writes standard titles and authors, then encapsulates the complete provenance JSON dictionary inside the native PDF `/Keywords` stream.

`pnggrep` navigates these differences transparently. It flattens nested JSON payloads from SVG `<dc:description>` and PDF `/Keywords` into hierarchical slash-separated paths:

```
PNG Chunk:          SourceCode
SVG Equivalent:     Description/source_code
PDF Equivalent:     Keywords/source_code

```

---

## System Metadata Reference Table

The following system metadata fields are harvested automatically by `smart_savefig` and embedded during export:

| Field Identifier | Stored in PNG | Stored in SVG / PDF | Data Type | Description |
| --- | --- | --- | --- | --- |
| `SourceFile` | `SourceFile` | `filename` | `str` | Name of the calling Python script. |
| `SourceDirectory` | `SourceDirectory` | `directory` | `str` | Absolute path to the calling script's parent directory. |
| `SourcePath` | `SourcePath` | `filepath` | `str` | Fully resolved filesystem path to the script. |
| `SourceCode` | `SourceCode` | `source_code` | `str` | Complete text listing of the calling script. |
| `CallerFunction` | `CallerFunction` | `caller_function` | `str` | Name of the function executing `savefig`. |
| `CallerLine` | `CallerLine` | `caller_line` | `int` | Exact source line number where `savefig` was called. |
| `Timestamp` | `Timestamp` | `timestamp` | `str` | ISO 8601 execution time (`YYYY-MM-DDTHH:MM:SS.mmmmmm`). |
| `PythonVersion` | `PythonVersion` | `python_version` | `str` | Python interpreter version (e.g. `3.11.5`). |
| `Hostname` | `Hostname` | `hostname` | `str` | System network node hostname (e.g. `viz06`, `login01`). |
| `Username` | `Username` | `username` | `str` | System login user executing the process. |
| `Command` | `Command` | `command` | `str` | Full command-line invocation (`sys.argv` shell-escaped). |
| `SysPath` | `SysPath` | `sys_path` | `str` | Deduplicated list of active Python module search paths. |
| `GitCommit` | `GitCommit` | `git_commit` | `str` | Git commit hash (`HEAD`). |
| `GitCommitMessage` | `GitCommitMessage` | `git_commit_message` | `str` | First line of the latest Git commit message. |
| `GitDirty` | `GitDirty` | `git_dirty` | `bool` | `True` if tracked changes or uncommitted edits exist. |
| `GitScriptTracked` | `GitScriptTracked` | `git_script_tracked` | `bool` | `True` if calling script is tracked in Git. |
| `PlotTitles` | `PlotTitles` | `figure_semantics/titles` | `list[str]` | Harvested figure suptitles and subplot titles. |
| `AxisLabels` | `AxisLabels` | `figure_semantics/axis_labels` | `list[str]` | Harvested axis labels (x, y, z across all axes). |
| `LegendLabels` | `LegendLabels` | `figure_semantics/legends` | `list[str]` | Harvested legend entries and artist labels. |
| `LocalModules` | `LocalModules` | `local_modules` | `str` | Bundled source listings of imported local modules. |
| `Databases` | `Databases` | `databases` | `list[dict]` | Extracted structural schemas of passed datasets. |

---

## User Metadata Isolation & Collision Safety

Custom metadata dictionaries can be supplied via `metadata={...}`:

```python
savefig("run.png", metadata={"Campaign": "Autumn2026", "SourceFile": "override.py"})

```

To guarantee that system provenance cannot be forged, overwritten, or corrupted by user-supplied inputs, `smart_savefig` runs all custom keys through an isolation layer:

1. All keys are checked against `RESERVED_SYSTEM_KEYS` (ignoring case, hyphens, and underscores).
2. Any user key colliding with system provenance (e.g. `SourceFile`, `git_commit`, `hostname`, `shot`, `modules`) is intercepted.
3. The colliding key is rewritten to `User/<Key>` and a warning is printed to `stderr`:
```text
[smart_savefig warning] User metadata key 'SourceFile' conflicts with system reserved key. Remapped to 'User/SourceFile'.

```


4. Authoritative system provenance keys are always written to the metadata payload.

---

## Domain Parameters & Module Bundling Reference

`smart_savefig` handles domain-specific parameters and database schemas through specialized processing pipelines:

| Argument | Accepted Inputs | Stored Representation | Processing Mechanics |
| --- | --- | --- | --- |
| `shot` / `pulse` | `int` or `str` | `str` or `int` | Normalized into a single identifier (numeric values parsed to `int`). |
| `shots` / `pulses` | `list`, `tuple`, `set` | JSON array / lines | Deduplicated, sorted sequence of identifiers. |
| `modules` | `list[ModuleType]` | Delimited stream | Source paths resolved; byte-compiled `.pyc` caches mapped back to source `.py`. |
| `database` / `databases` | `DataFrame`, `Series`, `Dataset`, `DataArray`, `dict`, `Path` | JSON Schema array / text blocks | Duck-typed structural schema extraction (shapes, column names, dims, coordinates, index names). **Raw array values are never extracted**, preventing data leakage. |

---

## Embedded Module Storage Format

When imported local helper modules are supplied to `modules=[...]`, their source files are located on disk, read, and compiled into a line-delimited stream:

```text
%%% <modulename> % <relative_path>
<module_source_code_line_1>
<module_source_code_line_2>
...

%%% <second_module> % <relative_path>
<second_module_source_code>

```

* Module file paths are recorded relative to the calling script's parent directory.
* `smart_savefig.py` itself and the calling script are automatically filtered out to prevent redundant duplication.
* `pnggrep` parses this stream to display module summaries and extract module files independently.

---

# 3. `pnggrep` (Metadata Inspection CLI)

`pnggrep` is a standalone CLI utility written in Rust. It inspects embedded metadata across PNG, SVG, and PDF files without requiring external GUI libraries, image decoding stacks, or PDF runtimes.

## Core Operating Modes

`pnggrep` provides five primary operating modes:

```
pnggrep Execution Flow:
├── 1. Pattern Search Mode:    pnggrep <PATTERN> [PATHS...]
├── 2. Listing Mode:           pnggrep -l [-n LINES] [PATHS...]
├── 3. Fusion Pulse Search:    pnggrep -p <PULSE> [PATHS...]
├── 4. Module Inspection:      pnggrep -M [PATHS...]  /  pnggrep --cat-module <NAME> <FILE>
├── 5. Key Extraction:         pnggrep --cat <KEY> <FILE>
└── 6. Sanitization Mode:      pnggrep --sanitize [PATHS...]

```

1. **Pattern Search Mode (`pnggrep <PATTERN>`):** Recursively searches metadata keys and values for substring matches, displaying colored, line-numbered snippets.
2. **Listing Mode (`pnggrep -l`):** Displays a structured inventory of all metadata entries across figures, showing multiline values with vertical indentation bars (`|`) and configurable truncation (`-n`).
3. **Fusion Pulse Search Mode (`pnggrep -p`):** Performs exact matching across `Shot`, `Shots`, `Pulse`, and `Pulses` keys in scalar strings and JSON arrays.
4. **Module Inspection Mode (`pnggrep -M` / `--cat-module`):** Lists all bundled Python modules, their relative paths, and line counts (`-M`), or dumps a module's raw source code directly to stdout (`--cat-module`).
5. **Key Extraction Mode (`pnggrep --cat`):** Extracts the raw, unformatted value of any metadata key to stdout (for piping into files or tools).
6. **Sanitization Mode (`pnggrep --sanitize`):** Strips private provenance metadata from figures and writes `<file>_sanitized.<ext>` copies for publication.

---

## CLI Flags & Options Reference

| Flag / Option | Argument | Description |
| --- | --- | --- |
| `<PATTERN>` | String | Substring to search across metadata keys and values. |
| `-l` | *None* | Enables Listing Mode (displays all keys and values). |
| `-n` | Integer | Maximum lines to display per multiline value in list mode (`0` for all) [default: `3`]. |
| `-p`, `--pulse` | Identifier | Unified fusion search matching exact pulse or shot identifiers. |
| `-M`, `--modules` | *None* | Lists bundled local Python modules with paths and line counts. |
| `--cat-module` | Module Name | Dumps the source code of a bundled module to stdout (requires single target file). |
| `--after`, `--since` | Date / Duration | Matches figures created on or after date (`YYYY`, `YYYY-MM`, `YYYY-MM-DD`, ISO, or `2d`, `12h`). |
| `--before`, `--until` | Date / Duration | Matches figures created on or before date (`YYYY`, `YYYY-MM`, `YYYY-MM-DD`, ISO, or `2d`, `12h`). |
| `-k`, `--key` | Substring | Filters which metadata keys are searched or listed (case-insensitive). |
| `-K`, `--keys-only` | *None* | Emits matching metadata key names only, suppressing line content. |
| `-F`, `--files-only` | *None* | Emits matching filenames only (aliases: `--files-with-matches`, `--files-with-match`). |
| `--json` | *None* | Emits search and listing results as line-delimited JSON (ndjson). |
| `--cat`, `-cat` | Key Name | Dumps the exact raw value of a key without formatting (requires single target file). |
| `--sanitize` | *None* | Strips provenance metadata and creates `<file>_sanitized.<ext>`. |
| `-s`, `--skip`, `--exclude` | Dir Name | Skips specified directory name or path during traversal (repeatable). |
| `-V`, `--version` | *None* | Prints CLI version information. |
| `-h`, `--help` | *None* | Prints the CLI help screen. |

---

## Hierarchical Display & JSON Flattening

In SVG and PDF formats, metadata is packed inside `<dc:description>` or `/Keywords` JSON strings. `pnggrep` flattens these nested structures into hierarchical paths:

```json
{
  "directory": "/home/researcher",
  "figure_semantics": {
    "axis_labels": ["Time (s)", "Amplitude (V)"]
  }
}

```

This JSON payload is flattened and displayed in `pnggrep -l` as:

```text
figure.svg
  Description/directory: /home/researcher
  Description/figure_semantics/axis_labels:
    | Time (s)
    | Amplitude (V)

```

### Flexible Key Resolution (`resolve_key`)

When using `--cat` or `-k/--key`, you do not need to type the full hierarchical path:

1. **Full Path:** Matches `Keywords/source_code` or `Description/source_code`.
2. **Leaf Matching:** Passing `--cat SourceCode` matches `SourceCode` in PNG, `Description/source_code` in SVG, and `Keywords/source_code` in PDF.
3. **Punctuation & Case Tolerance:** `source_code`, `SourceCode`, `source-code`, and `SOURCECODE` resolve to the same underlying key.

---

## Intelligent Directory Traversal & Exclusion Engine

When run across home directories, network shares, or cluster scratch spaces, scanning unwanted dependency folders can slow down traversal:

```
Directory Traversal Filter:
├── 1. Command-Line Targets: Passed explicitly --> ALWAYS SCANNED (e.g. pnggrep -l .cargo/)
└── 2. Recursive Child Directories Encountered:
       ├── User Exclusions (-s, --skip, --exclude) --> SKIPPED (with stderr notification)
       ├── Default Build/Cache Folders (target/, dist/, etc.) --> SKIPPED (with stderr notification)
       ├── Python Virtual Environments (pyvenv.cfg, etc.) --> SKIPPED (with stderr notification)
       └── Silent Infrastructure Directories (.rustup, .cargo, etc.) --> SILENTLY SKIPPED

```

* **Silent Exclusions:** Infrastructure folders and caches are skipped without stderr output:
* *Version Control & Caches:* `.git`, `__pycache__`, `.hg`, `.svn`.
* *Language Stores:* `.julia`, `.juliaup`, `.rustup`, `.cargo`.
* *Remote Servers:* `.vscode-server`, `.vscode`.
* *Python Caches:* `.conda`, `.cache`, `.local`, `.ipython`, `.jupyter`.
* *Node/JavaScript:* `.npm`, `.nvm`, `.yarn`.
* *Containers & HPC:* `.singularity`, `.apptainer`, `.docker`, `.spack`.


* **Dynamic Python Virtual Environment Detection:** Identifies virtual environments regardless of directory name by checking for `pyvenv.cfg`, `conda-meta/`, or `bin/activate` + `bin/python`.
* **Explicit Overrides:** If you pass an excluded directory directly (e.g. `pnggrep -l .cargo/`), `pnggrep` will inspect it.

---

# 4. Installation Guide

## Linux & macOS (Native HPC & Workstation)

`pnggrep` compiles on Unix platforms using the standard Rust toolchain and links against system `zlib` via FFI:

```bash
git clone https://github.com/<your-username>/pnggrep.git
cd pnggrep
cargo build --release

# Install into user local binary path:
mkdir -p ~/.local/bin
cp target/release/pnggrep ~/.local/bin/

```

Ensure `~/.local/bin` is in your shell profile (`~/.bashrc` or `~/.zshrc`):

```bash
export PATH="$HOME/.local/bin:$PATH"

```

---

## Windows Installation

### Option A: Cross-Compilation via WSL (Recommended)

To avoid installing the multi-gigabyte Microsoft Visual C++ Build Tools on Windows, cross-compile `pnggrep.exe` from Linux or WSL using MinGW:

```bash
# In WSL or Linux terminal:
sudo apt-get update && sudo apt-get install -y gcc-mingw-w64-x86-64

# Add the Windows MinGW target to Rust:
rustup target add x86_64-pc-windows-gnu

# Compile the standalone Windows executable:
cargo build --release --target x86_64-pc-windows-gnu

```

The resulting `target/x86_64-pc-windows-gnu/release/pnggrep.exe` is a standalone binary that can be copied directly to your Windows system.

### Option B: Native Windows Build

1. Install Rust via [rustup.rs](https://www.google.com/search?q=https://rustup.rs/).
2. In PowerShell or Command Prompt, run:
```powershell
git clone https://github.com/<your-username>/pnggrep.git
cd pnggrep
cargo build --release

```


*(On Windows, `pnggrep` automatically switches to the pure-Rust `flate2` decompression backend.)*
3. Copy `target\release\pnggrep.exe` to a directory in your system `%PATH%` (such as `%USERPROFILE%\.cargo\bin`).

---

## Python Integration

Add `smart_savefig.py` to your Python search path:

```bash
# In ~/.bashrc or job submission scripts:
export PYTHONPATH="/path/to/pnggrep:$PYTHONPATH"

```

Or install it in editable mode inside your active Python environment:

```bash
pip install -e /path/to/pnggrep

```

---

# 5. Practical Examples & Workflows

## Generating Figures with Embedded Provenance

```python
import matplotlib.pyplot as plt
import numpy as np
import my_analysis_helper as helper
from smart_savefig import savefig

# 1. Setup multi-panel scientific plot
fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(8, 6), sharex=True)
t = np.linspace(0, 10, 1000)
signal = np.sin(2 * np.pi * 0.5 * t) * np.exp(-t / 3.0)

ax1.plot(t, signal, color='tab:blue', label='Mode n=1')
ax1.set_title("MHD Mode Amplitude")
ax1.set_ylabel("Perturbation (a.u.)")
ax1.legend()

ax2.plot(t, np.abs(signal), color='tab:red', label='Envelope')
ax2.set_xlabel("Time (s)")
ax2.set_ylabel("Magnitude")
ax2.legend()

# 2. Save with execution provenance, shot number, and helper module:
savefig(
    "mhd_stability.png",
    fig=fig,
    shot=92185,
    modules=[helper],
    dpi=300
)

# 3. Vector format with auto-rasterization enabled:
savefig(
    "mhd_stability.pdf",
    fig=fig,
    shot=92185,
    modules=[helper]
)

```

---

## Querying Metadata by Text, Pulse, and Date

### Substring Search

```bash
# Search for specific library calls across figures
pnggrep "scipy.signal.spectrogram" .

# Search for specific LaTeX axis labels
pnggrep "\rho_{pol}" .

# Print matching filenames only
pnggrep -F "eigenfunctions" ./figures/

```

### Fusion Pulse / Shot Search (`-p`)

```bash
# Find all figures generated for pulse 92185 (PNG, SVG, and PDF)
pnggrep -p 92185 .

# Pipe matching filenames into a viewer
pnggrep -p 92185 -F . | xargs evince

```

### Date and Time Window Queries

```bash
# Figures created during calendar year 2026
pnggrep --after 2026 --before 2026 -l .

# Figures created in October 2026
pnggrep --after 2026-10 -l .

# Figures generated within a campaign window
pnggrep --after 2026-10-01 --before 2026-10-15 -p 92185 .

# Figures generated in the last 2 days
pnggrep --since 2d -l .

```

---

## Inspecting and Extracting Bundled Python Modules

```bash
# List all bundled helper modules across figures
pnggrep -M ./figures/

# Sample output:
# ./figures/mhd_stability.png
#   my_analysis_helper [my_analysis_helper.py] (142 lines)

# Extract the bundled module's source code back to disk:
pnggrep --cat-module my_analysis_helper ./figures/mhd_stability.png > my_analysis_helper.py

```

---

## Restoring Scripts and Sanitizing Output

### Direct Script Recovery

```bash
# Extract the exact Python script that produced the figure
pnggrep --cat SourceCode mhd_stability.png > restored_script.py

# Works identically across vector formats
pnggrep --cat SourceCode mhd_stability.pdf > restored_script.py
pnggrep --cat SourceCode mhd_stability.svg > restored_script.py

```

### Preparing Public Figures

```bash
# Strip internal hostnames, usernames, paths, and source listings
pnggrep --sanitize mhd_stability.png

# Creates 'mhd_stability_sanitized.png' suitable for open-access distribution

```

---

# 6. Technical Architecture & Security Safeguards

## Lazy Chunk Decompression

To maintain high throughput across large directories containing multi-megabyte figures, `pnggrep` avoids decoding image bitmaps. For PNG files, it implements a two-stage chunk inspector:

```
PNG File Stream:
┌──────────────┬────────────────────────────────────────────────────────┐
│ Chunk Header │ Chunk Data Payload                                     │
├──────────────┼──────────────────────────────────────┬─────────────────┤
│ Length + Type│ Uncompressed Keyword (Null-Terminated)│ Deflate Stream  │
└──────────────┴──────────────────────────────────────┴─────────────────┘
                                ▲                               ▲
                                │                               │
                     [Stage 1: Read Keyword]        [Stage 2: Evaluate Filter]
                     - Shot                         - Matches query?
                     - Timestamp                      YES -> Decompress payload
                     - SourceCode (Multi-MB)          NO  -> SEEK FORWARD (Skip zlib)

```

1. **Stage 1 (Keyword Peeking):** In PNG `zTXt` and `iTXt` chunks, the chunk keyword (e.g. `SourceCode`, `LocalModules`, `Shot`) is stored as an uncompressed, null-terminated ASCII string *before* the compression flag and byte payload. `pnggrep` reads this initial header without invoking zlib.
2. **Stage 2 (Conditional Decompression):** If the user is filtering by key (`-k Shot`), pulse (`-p 92185`), or date (`--since 2d`), chunks matching other keys (such as a 5 MB embedded `SourceCode` or `LocalModules` payload) are skipped via filesystem seek (`SeekFrom::Current`) without decompressing them.

---

## Security Hardening & DoS Protections

`pnggrep` is designed to safely handle untrusted or corrupt image files:

* **Decompression Bomb Protection:** Decompression buffers are strictly bounded to **32 MB uncompressed** (`MAX_DECOMPRESSED_SIZE`). Any compressed stream demanding more memory aborts decompression immediately.
* **Chunk Allocation Limits:** Raw PNG chunks exceeding **16 MB** (`MAX_METADATA_CHUNK_SIZE`) and SVG files exceeding **32 MB** (`MAX_SVG_READ_SIZE`) are bypassed via forward seeks to prevent memory exhaustion.
* **Terminal Escape Sanitization:** All text sent to stdout is sanitized (`sanitize_for_terminal`). ANSI escape sequences (`\x1b[...]`) and control characters are stripped, preventing terminal spoofing or prompt injection attacks.
* **XML Parser Hardening:** The built-in SVG XML parser uses a lightweight scanner that does not resolve external entities (DTD), protecting against XML External Entity (XXE) and SSRF attacks.
* **JSON Recursion Guard:** The JSON parser limits nesting to a maximum depth of **8 levels** (`MAX_JSON_RECURSION_DEPTH`) to prevent stack overflow attacks.

---

# 7. Regulatory AI Disclosure & Compliance

In compliance with the transparency obligations of the **European Union Artificial Intelligence Act** (Regulation (EU) 2024/1689, Article 50):

* **Architectural Conception & Direction:** Conceived, scoped, architected, and directed by Edmondo Giovannozzi (Dirigente di Ricerca, ENEA).
* **AI-Assisted Development:** Developed in collaborative partnership with Large Language Model systems (**Google Gemini**). AI assistance was used for boilerplate implementation, cross-platform POSIX/Windows compatibility code, and test case scaffolding.
* **Supervision & Verification:** All algorithms, FFI bindings, parsing logic, and security measures were reviewed, tested, and validated by the project author to ensure technical correctness and reproducibility.

---

# 8. License

This project is open-source software licensed under the [MIT License](https://www.google.com/search?q=LICENSE).